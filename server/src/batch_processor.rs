use crate::db;
use crate::oauth::AuthHelper;
use crate::Config;
use async_channel;
use async_recursion::async_recursion;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shared::structs::EnvelopeDetail;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;
use tokio::task;
use tokio::time::{sleep_until, Duration, Instant};

const CONCURRENCY_BUFFER: usize = 10;
pub fn init_batch_processor(
	http_client: &Client,
	config: &Config,
	token: AuthHelper,
	db_wtx: db::WriteTx,
	db_rtx: db::ReadTx,
) -> (
	async_channel::Sender<()>,
	task::JoinHandle<()>,
	async_channel::Sender<()>,
	task::JoinHandle<()>,
) {
	let http_client = http_client.clone();
	let config = config.clone();
	let (new_tx, new_rx) = async_channel::bounded(1);
	let new_handle = task::spawn({
		let http_client = http_client.clone();
		let config = config.clone();
		let db_wtx = db_wtx.clone();
		let db_rtx = db_rtx.clone();
		let token = token.clone();
		async move {
			new_batch_processor(http_client.clone(), config, token, new_rx, db_wtx, db_rtx).await
		}
	});
	let (completed_tx, completed_rx) = async_channel::bounded(1);
	let completed_handle = task::spawn(async move {
		completed_envelope_processor(http_client, config, token, completed_rx, db_wtx, db_rtx).await
	});
	(new_tx, new_handle, completed_tx, completed_handle)
}

async fn completed_envelope_processor(
	http_client: Client,
	config: Config,
	mut token: AuthHelper,
	rx: async_channel::Receiver<()>,
	db_wtx: db::WriteTx,
	db_rtx: db::ReadTx,
) {
	while let Ok(_) = rx.recv().await {
		let (otx, orx) = oneshot::channel();
		token.get().await;
		match db_rtx.send((db::ReadAction::UnprocessedEnvelopes, otx)) {
			Err(_) => panic!("db connection broken"),
			Ok(_) => match orx.await {
				Ok(db::ReadResult::Ok(db::RSuccess::UnprocessedEnvelopes(completed_envelopes))) => {
					stream::iter(completed_envelopes)
						.map(|comp_env| {
							let token = token.clone();
							let http_client = http_client.clone();
							let config = config.clone();
							let db_wtx = db_wtx.clone();
							async move {
								complete_envelope(http_client, &config, token, db_wtx, comp_env)
									.await
							}
						})
						.buffer_unordered(CONCURRENCY_BUFFER)
						.collect::<()>()
						.await;
				}
				_ => println!("Unable to get completed batch from db"),
			},
		}
	}

	async fn complete_envelope(
		client: Client,
		config: &Config,
		mut token: AuthHelper,
		db_writer: db::WriteTx,
		db::UnprocessedEnvelope { gid, status }: db::UnprocessedEnvelope,
	) {
		match status {
			db::UnprocessedStatus::Voided => {
				let (otx, orx) = oneshot::channel();
				let request = client
					.get(
						"https://".to_owned()
							+ &config.docusign.base_uri + "/restapi/v2.1/accounts/"
							+ &config.docusign.user_account_id
							+ "/envelopes/" + &gid,
					)
					.bearer_auth(token.get().await)
					.send()
					.await;

				let void_reason = match request {
					Ok(resp) => match resp.json::<serde_json::Value>().await {
						Ok(val) => match &val["voidedReason"] {
							serde_json::Value::String(s) => s.to_owned(),
							_ => "UNKNOWN_VOID_REASON".into(),
						},
						Err(_) => "UNKNOWN_VOID_REASON".into(),
					},
					Err(_) => "UNKNOWN_VOID_REASON".into(),
				};
				db_writer
					.send((
						db::WriteAction::UpdateStatus {
							gid,
							status: "voided".into(),
							api_err: None,
							void_reason: Some(void_reason.into()),
						},
						otx,
					))
					.unwrap_or_default();
			}
			db::UnprocessedStatus::Completed => {
				//get completed envelope data here

				//let (otx, orx) = oneshot::channel();
				// let request = client
				// 	.get(
				// 		"https://".to_owned()
				// 			+ &config.docusign.base_uri + "/restapi/v2.1/accounts/"
				// 			+ &config.docusign.user_account_id
				// 			+ "/envelopes/" + &gid,
				// 	)
				// 	.bearer_auth(token.get().await)
				// 	.send()
				// 	.await;
			}
		}
	}
}

async fn new_batch_processor(
	http_client: Client,
	config: Config,
	mut token: AuthHelper,
	rx: async_channel::Receiver<()>,
	db_wtx: db::WriteTx,
	db_rtx: db::ReadTx,
) {
	println!("Batch Processer started");
	while let Ok(_) = rx.recv().await {
		let (otx, orx) = oneshot::channel();
		token.get().await;
		match db_rtx.send((db::ReadAction::NewEnvelopes, otx)) {
			Err(_) => panic!("db connection broken"),
			Ok(_) => match orx.await {
				Ok(db::ReadResult::Ok(db::RSuccess::EnvelopeDetails(envelopes))) => {
					stream::iter(envelopes)
						.map(|envelope| {
							println! {"Envelope Api Called"}
							let token = token.clone();
							let http_client = http_client.clone();
							let config = config.clone();
							let db_wtx = db_wtx.clone();
							async move {
								send_envelope(http_client, &config, token, db_wtx, envelope).await
							}
						})
						.buffer_unordered(CONCURRENCY_BUFFER)
						.collect::<()>()
						.await;
				}
				_ => println!("unable to get batch from db"),
			},
		}
	}

	#[async_recursion]
	async fn send_envelope(
		client: Client,
		config: &Config,
		mut token: AuthHelper,
		db_writer: db::WriteTx,
		envelope: EnvelopeDetail,
	) {
		let request = client
			.post(
				"https://".to_owned()
					+ &config.docusign.base_uri
					+ "/restapi/v2.1/accounts/"
					+ &config.docusign.user_account_id
					+ "/envelopes",
			)
			.bearer_auth(token.get().await)
			.json(&NewEnvelope::from_db_env(&envelope, config))
			.send()
			.await;

		match request {
			Err(_) => {
				let (tx, rx) = oneshot::channel();
				db_writer
					.send((
						db::WriteAction::UpdateStatusWithId {
							id: envelope.id,
							api_err: Some("Unable to connect to docusign api".into()),
							gid: None,
							status: "cancelled".into(),
							void_reason: None,
						},
						tx,
					))
					.unwrap_or_default();
				if let Err(_) = rx.await {
					//TODO: log if db didn't work
					println!("db write failed")
				}
			}
			Ok(resp) => match resp.status().as_u16() {
				201 => match resp.json::<Sent>().await {
					Ok(sent_envelope) => {
						let (tx, rx) = oneshot::channel();
						db_writer
							.send((
								db::WriteAction::UpdateStatusWithId {
									id: envelope.id,
									gid: Some(sent_envelope.envelope_id),
									status: sent_envelope.status,
									api_err: None,
									void_reason: None,
								},
								tx,
							))
							.unwrap_or_default();

						match rx.await {
							Ok(result) => {
								if let Err(_) = result {
									//todo: log write error
								}
							}
							Err(_) => (), //todo: log oneshot channel error
						}
						println!("Docusign sent!")
					}
					Err(error) => println!("Unable to parse json body: {error}"),
				},
				400..=499 => {
					println!("{resp:?}");
					let await_time = {
						match resp.headers().get("X-RateLimit-Reset") {
							Some(time) => match time.to_str() {
								Ok(time) => match time.parse::<u64>() {
									Ok(time) => Some(
										Instant::now()
											+ Duration::from_secs(
												time - SystemTime::now()
													.duration_since(UNIX_EPOCH)
													.expect("System Clock Error")
													.as_secs(),
											),
									),
									Err(_) => None,
								},
								Err(_) => None,
							},
							None => None,
						}
					};
					match resp.json::<ErrorDetails>().await {
						Ok(error_detail) => {
							println!("{error_detail:?}");
							if error_detail.error_code == "HOURLY_APIINVOCATION_LIMIT_EXCEEDED" {
								if let Some(time) = await_time {
									sleep_until(time).await;
									send_envelope(client, config, token, db_writer, envelope).await;
								}
							}
						}
						Err(_) => println!("unable to parse error msg"),
					}
				}
				500..=599 => (),
				_ => (),
			},
		}
		#[derive(Deserialize)]
		#[serde(rename_all = "camelCase")]
		struct Sent {
			envelope_id: String,
			status: String,
		}
		#[derive(Deserialize, Debug)]
		#[serde(rename_all = "camelCase")]
		#[allow(dead_code)]
		struct ErrorDetails {
			error_code: String,
			message: String,
		}
	}
}

impl NewEnvelope {
	fn from_db_env(db_env: &EnvelopeDetail, config: &Config) -> Self {
		let mut env = Self {
			template_id: config.docusign.templateId.clone(),
			template_roles: vec![EnvelopeRecipient {
				name: format!(
					"{} {}{}",
					db_env.first_name,
					match &db_env.middle_name {
						Some(val) => val.to_owned() + " ",
						None => "".into(),
					},
					db_env.last_name
				),

				email: db_env.email.clone(),
				role_name: "Signer".into(),

				tabs: Some(Tabs {
					text_tabs: [
						TabValue {
							tab_label: "SSN".into(),
							value: db_env.ssn.to_string(),
						},
						TabValue {
							tab_label: "DOB".into(),
							value: db_env.dob.clone(),
						},
						TabValue {
							tab_label: "address".into(),
							value: format!(
								"{}{}",
								db_env.addr1,
								match &db_env.addr2 {
									Some(val) => " ".to_string() + &val,
									None => "".into(),
								}
							),
						},
						TabValue {
							tab_label: "cit-st-zip".into(),
							value: format!("{}, {} {}", db_env.city, db_env.state, db_env.zip),
						},
						TabValue {
							tab_label: "phone".into(),
							value: db_env.phone.clone(),
						},
					],
				}),
			}],
			status: "sent".into(),
			event_notification: EventNotification {
				url: config.network.endpoint_url.clone() + "/webhook",
				require_acknowledgment: "true".into(),
				logging_enabled: "true".into(),
				delivery_mode: "SIM".into(),
				events: vec![
					"envelope-resent".into(),
					"envelope-delivered".into(),
					"envelope-completed".into(),
					"envelope-declined".into(),
					"envelope-voided".into(),
					"envelope-corrected".into(),
					"envelope-purge".into(),
					"envelope-deleted".into(),
				],
				event_data: EventData {
					version: "restv2.1".into(),
					format: "json".into(),
				},
				include_HMAC: "true".into(),
			},
		};

		if let (Some(fname), Some(lname), Some(email)) = (
			db_env.spouse_fname.clone(),
			db_env.spouse_lname.clone(),
			db_env.spouse_email.clone(),
		) {
			env.template_roles.push(EnvelopeRecipient {
				role_name: "Spouse".into(),

				name: format!(
					"{} {}{}",
					fname,
					match &db_env.spouse_mname {
						Some(val) => val.to_string() + " ",
						None => "".into(),
					},
					lname
				),
				email,
				tabs: None,
			})
		}

		env
	}
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewEnvelope {
	template_id: String,
	template_roles: Vec<EnvelopeRecipient>,
	status: String,
	event_notification: EventNotification,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvelopeRecipient {
	name: String,
	email: String,
	role_name: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	tabs: Option<Tabs>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tabs {
	text_tabs: [TabValue; 5],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TabValue {
	tab_label: String,
	value: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(non_snake_case)]
struct EventNotification {
	url: String,
	require_acknowledgment: String,
	logging_enabled: String,
	delivery_mode: String,
	events: Vec<String>,
	event_data: EventData,
	include_HMAC: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventData {
	version: String,
	format: String,
}
