use crate::db;
use axum::{
	extract::State, http::StatusCode, middleware::from_fn, response::IntoResponse, routing::post,
	Json, Router,
};
use tokio::sync::oneshot;

pub fn create_routes(
	wtx: db::WriteTx,
	batch_tx: async_channel::Sender<()>,
) -> Router<(db::WriteTx, async_channel::Sender<()>)> {
	Router::with_state((wtx, batch_tx))
		.route("/batches", post(new_batch))
		.route_layer(from_fn(super::login::verify_user_session))
}

async fn new_batch(
	State((wtx, btx)): State<(db::WriteTx, async_channel::Sender<()>)>,
	Json(batch_data): Json<db::NewBatchData>,
) -> Result<impl IntoResponse, StatusCode> {
	let (tx, rx) = oneshot::channel();
	wtx.send((db::WriteAction::NewBatch(batch_data), tx))
		.unwrap_or_default();
	match rx.await {
		Ok(Ok(dups)) => {
			btx.try_send(()).unwrap_or_default();
			Ok((StatusCode::CREATED, format!("{dups}")))
		}
		Ok(Err(db::WFail::Duplicate)) => Err(StatusCode::CONFLICT),
		_ => Err(StatusCode::INTERNAL_SERVER_ERROR),
	}
}
