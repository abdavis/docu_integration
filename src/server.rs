use crate::db::{ReadTx, WriteAction, WriteTx};
use ring::hmac;
use serde::Deserialize;
use serde_json;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task;
use tokio::time::sleep;
use warp::{
	http::{self, HeaderMap},
	hyper::{self, body::Bytes},
	Filter, Reply,
};

pub fn create_server(config: &crate::Config, wtx: &WriteTx, rtx: &ReadTx) -> task::JoinHandle<()> {
	let config = config.clone();
	let wtx = wtx.clone();
	let rtx = rtx.clone();
	task::spawn(async move { server(config, wtx, rtx).await })
}
async fn server(config: crate::Config, wtx: WriteTx, rtx: ReadTx) {
	println!("Building server");
	let key = hmac::Key::new(hmac::HMAC_SHA256, config.docusign.hmac_key.as_bytes());
	let webhook = warp::path("webhook")
		.and(warp::post())
		.and(warp::body::content_length_limit(4194304))
		.and(warp::header::headers_cloned())
		.and(warp::body::bytes())
		.then(move |headers: HeaderMap, bytes: Bytes| {
			let key = key.clone();
			let wtx = wtx.clone();
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

	warp::serve(webhook)
		.tls()
		.cert_path("cert/cert.pem")
		.key_path("cert/key.pem")
		.run(([0, 0, 0, 0], 443))
		.await;

	println!("Shutting down Server");
}
async fn process_msg(bytes: Bytes, wtx: WriteTx) -> http::Response<hyper::Body> {
	match serde_json::from_slice::<Msg>(&bytes) {
		Err(_) => {
			println!("Unable to parse Json");
			warp::reply::with_status(warp::reply(), http::StatusCode::INTERNAL_SERVER_ERROR)
				.into_response()
		}
		Ok(msg) => {
			let (tx, rx) = oneshot::channel();
			if let "envelope-completed" = msg.event.as_str() {
				wtx.send((msg.to_db_complete(), tx));
			} else {
				wtx.send((msg.to_db_update(), tx));
			}
			match rx.await {
				Err(_) => {
					println!("Write transaction didn't complete");
					http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
				}
				Ok(write_result) => match write_result {
					Err(fail) => warp::reply::with_status(
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
			None => return Err("hmac authentication codes are invalid or missing".into()),
			Some(header_tag) => match header_tag.to_str() {
				Ok(tag) => {
					if tag == calculated_tag {
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
