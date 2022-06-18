use crate::db::{self, ReadTx, WFail, WriteAction, WriteTx};
use crate::login_handler::{LoginResult, PasswordManager, SessionFailure, SessionManager};
use crate::websocket_handler::{self, connect, ConnectorMsg, Resource};
use ring::{constant_time::verify_slices_are_equal, hmac};
use serde::{Deserialize, Serialize};
use serde_json;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task;
use tokio::time::sleep;
use warp::{
	http::{self, header, HeaderMap, StatusCode},
	hyper::{self, body::Bytes},
	Filter, Reply,
};

//8 hours
const SESSION_COOKIE_AGE: u64 = 60 * 60 * 8;

pub fn create_server(
	config: &crate::Config,
	db_wtx: &WriteTx,
	db_rtx: &ReadTx,
	ws_handler_tx: async_channel::Sender<websocket_handler::ConnectorMsg>,
	password_manager: PasswordManager,
	session_manager: SessionManager,
) -> task::JoinHandle<()> {
	let config = config.clone();
	let wtx = db_wtx.clone();
	let rtx = db_rtx.clone();
	task::spawn(async move {
		server(
			config,
			wtx,
			rtx,
			ws_handler_tx,
			password_manager,
			session_manager,
		)
		.await
	})
}
async fn server(
	config: crate::Config,
	db_wtx: WriteTx,
	db_rtx: ReadTx,
	ws_handler_tx: async_channel::Sender<websocket_handler::ConnectorMsg>,
	password_manager: PasswordManager,
	session_manager: SessionManager,
) {
	println!("Building server");

	let key = hmac::Key::new(hmac::HMAC_SHA256, config.docusign.hmac_key.as_bytes());
	let wtx_webhook = db_wtx.clone();
	let webhook = warp::path("webhook")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::content_length_limit(1024 * 32))
		.and(warp::header::headers_cloned())
		.and(warp::body::bytes())
		.then(move |headers: HeaderMap, bytes: Bytes| {
			let key = key.clone();
			let wtx = wtx_webhook.clone();
			async move {
				match verify_msg(&key, &headers, &bytes).await {
					Ok(_) => process_msg(bytes, wtx).await,
					Err(string) => {
						println!("{string}");
						http::StatusCode::UNAUTHORIZED.into_response()
					}
				}
			}
		});

	let new_batch = warp::path("batch")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::content_length_limit(1024 * 32))
		.and(warp::body::json())
		.then(move |batch: db::BatchData| {
			let wtx = db_wtx.clone();
			async move {
				let (tx, rx) = oneshot::channel();
				wtx.send((WriteAction::NewBatch(batch), tx)).ok();
				match rx.await {
					Err(_) => warp::reply::with_status(
						warp::reply(),
						http::StatusCode::INTERNAL_SERVER_ERROR,
					)
					.into_response(),
					Ok(result) => match result {
						Ok(dups) => format!("{{\"duplicates\":{dups}").into_response(),
						Err(err) => match err {
							WFail::Duplicate => warp::reply::with_status(
								"Batch name is a duplicate.",
								http::StatusCode::UNPROCESSABLE_ENTITY,
							)
							.into_response(),
							_ => warp::reply::with_status(
								warp::reply(),
								http::StatusCode::INTERNAL_SERVER_ERROR,
							)
							.into_response(),
						},
					},
				}
			}
		});

	let ws_handler_2 = ws_handler_tx.clone();
	let ws_handler_3 = ws_handler_tx.clone();
	let session_manager2 = session_manager.clone();

	let websocket_main = warp::path::end()
		.and(warp::ws())
		.and(warp::cookie::optional("session"))
		.map(
			move |ws: warp::ws::Ws, maybe_cookie: Option<String>| match maybe_cookie {
				None => {
					warp::reply::with_status("Session cookie missing", StatusCode::UNAUTHORIZED)
						.into_response()
				}

				Some(cookie) => match session_manager.verify_session_token(&cookie) {
					Err(err) => warp::reply::with_status(err.to_string(), err.to_status_code())
						.into_response(),
					Ok(_) => {
						let ws_handler_tx = ws_handler_tx.clone();
						ws.on_upgrade(move |socket| async move {
							ws_handler_tx
								.send(ConnectorMsg {
									channel: connect(socket),
									resource: Resource::Main,
								})
								.await
								.unwrap_or_default();
						})
						.into_response()
					}
				},
			},
		);

	let session_manager3 = session_manager2.clone();
	let websocket_batch = warp::path!("batch" / i64)
		.and(warp::ws())
		.and(warp::cookie::optional("session"))
		.map(
			move |batch_id, ws: warp::ws::Ws, maybe_cookie: Option<String>| match maybe_cookie {
				None => {
					warp::reply::with_status("Session cookie missing", StatusCode::UNAUTHORIZED)
						.into_response()
				}
				Some(cookie) => match session_manager2.verify_session_token(&cookie) {
					Err(err) => warp::reply::with_status(err.to_string(), err.to_status_code())
						.into_response(),
					Ok(_) => {
						let ws_handler_tx = ws_handler_2.clone();
						ws.on_upgrade(move |socket| async move {
							let result = ws_handler_tx
								.send(ConnectorMsg {
									channel: connect(socket),
									resource: Resource::Batch(batch_id),
								})
								.await;
						})
						.into_response()
					}
				},
			},
		);

	let session_manager4 = session_manager3.clone();
	let websocket_individual = warp::path!("individual" / u32)
		.and(warp::ws())
		.and(warp::cookie::optional("session"))
		.map(
			move |ssn, ws: warp::ws::Ws, maybe_cookie: Option<String>| match maybe_cookie {
				None => {
					warp::reply::with_status("Session cookie missing", StatusCode::UNAUTHORIZED)
						.into_response()
				}
				Some(cookie) => match session_manager3.verify_session_token(&cookie) {
					Err(err) => warp::reply::with_status(err.to_string(), err.to_status_code())
						.into_response(),
					Ok(_) => {
						let ws_handler_tx = ws_handler_3.clone();
						ws.on_upgrade(move |socket| async move {
							ws_handler_tx
								.send(ConnectorMsg {
									channel: connect(socket),
									resource: Resource::Individual(ssn),
								})
								.await
								.unwrap_or_default();
						})
						.into_response()
					}
				},
			},
		);

	let websocket_filter = websocket_main.or(websocket_batch).or(websocket_individual);

	let hello = warp::any().map(|| "Hello World");

	let password_manager2 = password_manager.clone();
	let login_filter = warp::path!("login")
		.and(warp::body::content_length_limit(1024 * 32))
		.and(warp::post())
		.and(warp::body::form())
		.then(move |form_data: Box<[(String, String)]>| {
			let password_manager = password_manager.clone();
			async move {
				match (
					form_data
						.iter()
						.find_map(|(k, v)| if k == "username" { Some(v) } else { None }),
					form_data
						.iter()
						.find_map(|(k, v)| if k == "password" { Some(v) } else { None }),
				) {
					(Some(username), Some(password)) => match password_manager
						.login(username.clone(), password.clone())
						.await
					{
						LoginResult::Success(val) => warp::reply::with_header(
							warp::reply::with_header(
								warp::reply::with_status(warp::reply(), StatusCode::FOUND),
								header::LOCATION,
								"/",
							),
							header::SET_COOKIE,
							format!(
								"session={val}; Max-Age={SESSION_COOKIE_AGE};
									HttpOnly; Secure; SameSite=Strict"
							),
						)
						.into_response(),

						LoginResult::MustChangePwd => warp::reply::with_status(
							warp::reply::with_header(
								warp::reply(),
								header::LOCATION,
								"/login/change_password#mustchange",
							),
							StatusCode::FOUND,
						)
						.into_response(),

						LoginResult::Failed => warp::reply::with_status(
							warp::reply::with_header(
								warp::reply(),
								header::LOCATION,
								"/login#failed",
							),
							StatusCode::FOUND,
						)
						.into_response(),
					},
					_ => warp::reply::with_status(warp::reply(), StatusCode::BAD_REQUEST)
						.into_response(),
				}
			}
		});

	let change_pswd_filter = warp::path!("login" / " change_password")
		.and(warp::body::content_length_limit(1024 * 32))
		.and(warp::post())
		.and(warp::body::form())
		.then(move |form_data: Box<[(String, String)]>| {
			let password_manager = password_manager2.clone();
			async move {
				match (
					form_data
						.iter()
						.find_map(|(k, v)| if k == "username" { Some(v) } else { None }),
					form_data
						.iter()
						.find_map(|(k, v)| if k == "password" { Some(v) } else { None }),
					form_data
						.iter()
						.find_map(|(k, v)| if k == "new_pass1" { Some(v) } else { None }),
					form_data
						.iter()
						.find_map(|(k, v)| if k == "new_pass2" { Some(v) } else { None }),
				) {
					(Some(username), Some(password), Some(new_pass1), Some(new_pass2)) => {
						match password_manager
							.change_pswd(
								username.clone(),
								password.clone(),
								new_pass1.clone(),
								new_pass2.clone(),
							)
							.await
						{
							crate::login_handler::PswdChangeResult::Success => {
								warp::reply::with_status(
									warp::reply::with_header(
										warp::reply(),
										header::LOCATION,
										"/login#changedpassword",
									),
									StatusCode::FOUND,
								)
								.into_response()
							}

							crate::login_handler::PswdChangeResult::MismatchedPswds => {
								warp::reply::with_status(
									warp::reply::with_header(
										warp::reply(),
										header::LOCATION,
										"/login/change_password#mismatched",
									),
									StatusCode::FOUND,
								)
								.into_response()
							}

							crate::login_handler::PswdChangeResult::Failed => {
								warp::reply::with_status(
									warp::reply::with_header(
										warp::reply(),
										header::LOCATION,
										"/login/change_password#failed",
									),
									StatusCode::FOUND,
								)
								.into_response()
							}
						}
					}
					_ => warp::reply::with_status(warp::reply(), StatusCode::BAD_REQUEST)
						.into_response(),
				}
			}
		});

	let session_manager5 = session_manager4.clone();
	let get_users = warp::path::end()
		.and(warp::get())
		.and(warp::cookie::optional("session"))
		.then(move |maybe_cookie: Option<String>| {
			let session_manager4 = session_manager4.clone();
			async move {
				match maybe_cookie {
					None => {
						warp::reply::with_status("Session cookie missing", StatusCode::UNAUTHORIZED)
							.into_response()
					}
					Some(cookie) => match session_manager4.get_users(&cookie).await {
						Err(err) => warp::reply::with_status(err.to_string(), err.to_status_code())
							.into_response(),
						Ok(users) => warp::reply::json(&users).into_response(),
					},
				}
			}
		});

	let session_manager6 = session_manager5.clone();
	let create_user = warp::post()
		.and(warp::path::end())
		.and(warp::cookie::optional("session"))
		.and(warp::body::json())
		.then(move |maybe_cookie: Option<String>, new_user: NewUser| {
			let session_manager5 = session_manager5.clone();
			async move {
				match maybe_cookie {
					None => {
						warp::reply::with_status("Session cookie missing", StatusCode::UNAUTHORIZED)
							.into_response()
					}
					Some(cookie) => match session_manager5
						.create_user(&cookie, new_user.user_name, new_user.email, new_user.admin)
						.await
					{
						Err(err) => warp::reply::with_status(err.to_string(), err.to_status_code())
							.into_response(),
						Ok(temp_pass) => temp_pass.into_response(),
					},
				}
			}
		});

	let session_manager7 = session_manager6.clone();
	let delete_user = warp::delete()
		.and(warp::path!(String))
		.and(warp::cookie::optional("session"))
		.then(move |user, maybe_cookie: Option<String>| {
			let session_manager6 = session_manager6.clone();
			async move {
				match maybe_cookie {
					None => {
						warp::reply::with_status("Session cookie missing", StatusCode::UNAUTHORIZED)
							.into_response()
					}
					Some(cookie) => match session_manager6.delete_user(&cookie, user).await {
						Err(err) => warp::reply::with_status(err.to_string(), err.to_status_code())
							.into_response(),
						Ok(_) => warp::reply().into_response(),
					},
				}
			}
		});

	#[derive(Deserialize)]
	struct Update {
		email: Option<String>,
		reset_required: bool,
		admin: bool,
	}
	let update_user = warp::put()
		.and(warp::path!(String))
		.and(warp::cookie::optional("session"))
		.and(warp::body::json())
		.then(
			move |id: String, maybe_cookie: Option<String>, update: Update| {
				let session_manager7 = session_manager7.clone();
				async move {
					match maybe_cookie {
						None => warp::reply::with_status(
							"Session cookie missing",
							StatusCode::UNAUTHORIZED,
						)
						.into_response(),
						Some(cookie) => match session_manager7
							.update_user(
								&cookie,
								id,
								update.email,
								update.reset_required,
								update.admin,
							)
							.await
						{
							Ok(()) => warp::reply().into_response(),
							Err(err) => {
								warp::reply::with_status(err.to_string(), err.to_status_code())
									.into_response()
							}
						},
					}
				}
			},
		);

	let users = warp::path("users").and(get_users.or(create_user).or(delete_user).or(update_user));

	#[derive(Deserialize)]
	struct NewUser {
		user_name: String,
		email: Option<String>,
		admin: bool,
	}

	let api = warp::path("api").and(
		webhook
			.or(new_batch)
			.or(websocket_filter)
			.or(login_filter)
			.or(change_pswd_filter)
			.or(users)
			.or(hello),
	);

	warp::serve(api)
		.tls()
		.cert_path("cert/cert.pem")
		.key_path("cert/key.pem")
		.run(([127, 0, 0, 1], 8081))
		.await;

	println!("Shutting down Server");
}
async fn process_msg(bytes: Bytes, wtx: WriteTx) -> http::Response<hyper::Body> {
	match serde_json::from_slice::<Msg>(&bytes) {
		Err(_) => {
			println!("Unable to parse Json");
			warp::reply::with_status("Unable to parse Json", http::StatusCode::BAD_REQUEST)
				.into_response()
		}
		Ok(msg) => {
			let (tx, rx) = oneshot::channel();
			if let "envelope-completed" = msg.event.as_str() {
				wtx.send((msg.to_db_complete(), tx)).ok();
			} else {
				wtx.send((msg.to_db_update(), tx)).ok();
			}
			match rx.await {
				Err(_) => {
					println!("Write transaction didn't complete");
					http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
				}
				Ok(write_result) => match write_result {
					Err(_) => warp::reply::with_status(
						warp::reply(),
						http::StatusCode::INTERNAL_SERVER_ERROR,
					)
					.into_response(),
					Ok(_) => warp::reply().into_response(),
				},
			}
		}
	}
}

impl Msg {
	fn to_db_update(self) -> WriteAction {
		WriteAction::UpdateStatus {
			gid: self.data.envelope_id,
			status: self.data.envelope_summary.status,
			void_reason: self.data.envelope_summary.voided_reason,
			api_err: None,
		}
	}
	fn to_db_complete(self) -> WriteAction {
		WriteAction::CompleteEnvelope {
			gid: self.data.envelope_id,
			status: "completed".into(),
			beneficiaries: vec![],
			authorized_users: vec![],
			pdf: vec![],
		}
	}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Msg {
	event: String,
	data: MsgData,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MsgData {
	envelope_id: String,
	envelope_summary: EnvelopeSummary,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnvelopeSummary {
	status: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	voided_reason: Option<String>,
	recipients: Recipients,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Recipients {
	signers: Vec<Signer>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Signer {
	tabs: Tabs,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tabs {
	text_tabs: Vec<TextTab>,
	radio_group_tabs: Vec<RadioGroupTab>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextTab {
	value: String,
	original_value: String,
	tab_label: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RadioGroupTab {
	group_name: String,
	radios: Vec<Radio>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Radio {
	value: String,
	selected: String,
}
async fn verify_msg(key: &hmac::Key, headers: &HeaderMap, bytes: &Bytes) -> Result<(), String> {
	let const_time_complete = sleep(Duration::from_secs(5));
	let calculated_tag = base64::encode(hmac::sign(key, bytes));
	for n in 1..101 {
		match headers.get(format!("X-DocuSign-Signature-{n}")) {
			None => {
				const_time_complete.await;
				return Err("hmac authentication codes are invalid or missing".into());
			}
			Some(header_tag) => match header_tag.to_str() {
				Ok(tag) => {
					if let Ok(()) =
						verify_slices_are_equal(calculated_tag.as_bytes(), tag.as_bytes())
					{
						return Ok(());
					}
				}
				Err(_) => break,
			},
		}
	}
	const_time_complete.await;
	Err("Invald HMAC Authentication Code".into())
}
