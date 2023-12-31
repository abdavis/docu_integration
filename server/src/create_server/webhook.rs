use axum::{
	body::Bytes,
	extract::{self, State},
	http::{header::HeaderMap, StatusCode},
	routing::post,
	Router,
};
use ring::{constant_time::verify_slices_are_equal, hmac};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio::{sync::oneshot, time::sleep};

use crate::{db, Config};

pub fn create_routes(
	config: &Config,
	db_wtx: db::WriteTx,
	completed_tx: async_channel::Sender<()>,
) -> Router {
	let key = Arc::new(hmac::Key::new(
		hmac::HMAC_SHA256,
		config.docusign.hmac_key.as_bytes(),
	));
	let db_wtx = db_wtx.clone();
	Router::new().route(
		"/webhook",
		post(webhook_handler).with_state((key, db_wtx, completed_tx)),
	)
}
async fn webhook_handler(
	headers: HeaderMap,
	State((key, wtx, completed_tx)): State<(
		Arc<hmac::Key>,
		db::WriteTx,
		async_channel::Sender<()>,
	)>,
	body: Bytes,
) -> StatusCode {
	println!("webhook hit");
	match verify_msg(&key, &headers, &body).await {
		Err(_) => StatusCode::FORBIDDEN,
		Ok(_) => process_msg(body, wtx, completed_tx).await,
	}
}

async fn process_msg(
	body: Bytes,
	wtx: db::WriteTx,
	completed_tx: async_channel::Sender<()>,
) -> StatusCode {
	match serde_json::from_slice::<Msg>(&body) {
		Err(_) => StatusCode::BAD_REQUEST,
		Ok(msg) => {
			let completed = msg.event == "envelope-completed" || msg.event == "envelope-voided";
			let (tx, rx) = oneshot::channel();
			wtx.send((msg.into_db_update(), tx)).unwrap_or_default();
			match rx.await {
				Ok(Ok(_)) => {
					if completed {
						completed_tx.try_send(()).unwrap_or_default();
					}
					StatusCode::OK
				}
				Ok(Err(db::WFail::NoRecord)) => StatusCode::NOT_FOUND,
				_ => StatusCode::INTERNAL_SERVER_ERROR,
			}
		}
	}
}

async fn verify_msg(key: &hmac::Key, headers: &HeaderMap, body: &Bytes) -> Result<(), ()> {
	let const_time_complete = sleep(Duration::from_secs(1));
	let calculated_tag = base64::encode(hmac::sign(key, body));
	for n in 1..101 {
		match headers.get(format!("X-DocuSign-Signature-{n}")) {
			None => break,
			Some(header_tag) => {
				if let Ok(()) =
					verify_slices_are_equal(calculated_tag.as_bytes(), header_tag.as_bytes())
				{
					return Ok(());
				}
			}
		}
	}
	const_time_complete.await;
	Err(())
}

impl Msg {
	fn into_db_update(self) -> db::WriteAction {
		db::WriteAction::UpdateStatus {
			gid: self.data.envelope_id,
			status: self.event.split('-').last().unwrap_or("").into(),
			void_reason: None,
			api_err: None,
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
}
