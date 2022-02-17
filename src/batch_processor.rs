use crate::db;
use crate::oauth::AuthHelper;
use crate::Config;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio::task;

const CONCURRENCY_BUFFER: usize = 10;

pub async fn new_batch_processor(
	http_client: Client,
	config: Config,
	mut token: AuthHelper,
	mut rx: mpsc::Receiver<()>,
	db_wtx: crossbeam_channel::Sender<(db::WriteAction, oneshot::Sender<db::WriteResult>)>,
	db_rtx: crossbeam_channel::Sender<(db::ReadAction, oneshot::Sender<db::ReadResult>)>,
) {
	while let Some(_) = rx.recv().await {
		let (otx, orx) = oneshot::channel();

		match db_rtx.try_send((db::ReadAction::NewEnvelopes, otx)) {
			Err(_) => (),
			Ok(_) => match orx.await {
				Ok(db::ReadResult::Ok(db::RSuccess::EnvelopeDetails(envelopes))) => {
					stream::iter(envelopes)
						.map(|envelope| {
							let mut token = token.clone();
							let http_client = http_client.clone();
							let config = config.clone();
							let db_wtx = db_wtx.clone();
							async move {
								send_envelope(
									http_client,
									&config,
									token.get().await,
									db_wtx,
									envelope,
								)
								.await
							}
						})
						.buffer_unordered(CONCURRENCY_BUFFER);
				}
				_ => (),
			},
		}
	}

	async fn send_envelope(
		client: Client,
		config: &Config,
		token: String,
		db_writer: crossbeam_channel::Sender<(db::WriteAction, oneshot::Sender<db::WriteResult>)>,
		envelope: db::EnvelopeDetail,
	) {
		let request = client
			.post(
				"https://".to_owned()
					+ &config.docusign.base_uri
					+ "/restapi/v2.1/accounts/"
					+ &config.docusign.user_account_id
					+ "/envelopes",
			)
			.bearer_auth(token)
			.json(&NewEnvelope::from_db_env(&envelope, config))
			.send()
			.await;

		match request {
			Err(error) => {
				let (tx, rx) = oneshot::channel();
				db_writer.send((
					db::WriteAction::UpdateStatusWithId {
						id: envelope.id,
						api_err: Some("Unable to connect to docusign api".into()),
						gid: None,
						status: "cancelled".into(),
						void_reason: None,
					},
					tx,
				));
				if let Err(error) = rx.await {
					//TODO: log if db didn't work
					println!("db write failed")
				}
			}
			Ok(resp) => match resp.status().as_u16() {
				201 => match resp.json::<Sent>().await {
					Ok(sent_envelope) => {
						let (tx, rx) = oneshot::channel();
						db_writer.send((
							db::WriteAction::UpdateStatusWithId {
								id: envelope.id,
								gid: Some(sent_envelope.envelope_id),
								status: sent_envelope.status,
								api_err: None,
								void_reason: None,
							},
							tx,
						));
					}
					Err(error) => println!("Unable to parse json body: {error}"),
				},
				400..=499 => {
					let headers = resp.headers();
					match resp.json::<ErrorDetails>().await {
						Ok(error_detail) => {
							if error_detail.error_code == "HOURLY_APIINVOCATION_LIMIT_EXCEEDED" {
								
							}
						}
						Err(_) => println!("unable to parse error msg")
					}
				},
				500..=599 => {}
				_ => {}
			},
		}
		#[derive(Deserialize)]
		#[serde(rename_all = "camelCase")]
		struct Sent {
			envelope_id: String,
			status: String,
		}
		#[derive(Deserialize)]
		#[serde(rename_all = "camelCase")]
		struct ErrorDetails {
			error_code: String,
			message: String,
		}
		enum Retry {
			count(i32),
			time(i32)
		}
	}
}

impl NewEnvelope {
	fn from_db_env(db_env: &db::EnvelopeDetail, config: &Config) -> Self {
		let mut env = Self {
			template_id: config.docusign.templateId.clone(),
			transaction_id: db_env.id.to_string(),
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
				email: email,
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
	transaction_id: String,
	template_roles: Vec<EnvelopeRecipient>,
	status: String,
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
