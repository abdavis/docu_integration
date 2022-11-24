use crate::db;
use axum::{
	extract::{
		ws::{WebSocket, WebSocketUpgrade},
		Path, State,
	},
	handler::Handler,
	http::StatusCode,
	middleware::from_fn,
	response::{IntoResponse, Response},
	routing::{get, post},
	Json, Router,
};
use shared::structs::NewBatchData;
use tokio::sync::oneshot;

use super::websocket_handler::{connect, ConnectorMsg, Resource};

pub fn create_routes(
	wtx: db::WriteTx,
	//rtx: db::ReadTx,
	batch_tx: async_channel::Sender<()>,
	ws_handler_tx: async_channel::Sender<super::websocket_handler::ConnectorMsg>,
) -> Router {
	Router::new()
		.route_service("/api/batches", post(new_batch).with_state((wtx, batch_tx)))
		.route_service(
			"/wss/batches",
			get(connect_ws).with_state(ws_handler_tx.clone()),
		)
		.route_service(
			"/wss/batches/:batch_id",
			get(connect_ws_batch).with_state(ws_handler_tx.clone()),
		)
		.route_service(
			"/wss/people/:person_id",
			get(connect_ws_person).with_state(ws_handler_tx),
		)
	//.route_layer(from_fn(super::login::verify_user_session))
}

async fn connect_ws(
	ws: WebSocketUpgrade,
	State(wstx): State<async_channel::Sender<ConnectorMsg>>,
) -> Response {
	ws.on_upgrade(move |socket| async move {
		wstx.send(ConnectorMsg {
			channel: connect(socket),
			resource: Resource::Main,
		})
		.await
		.unwrap_or_default();
	})
}

async fn connect_ws_batch(
	ws: WebSocketUpgrade,
	Path(batch_id): Path<i64>,
	State(wstx): State<async_channel::Sender<ConnectorMsg>>,
) -> Response {
	ws.on_upgrade(move |socket| async move {
		wstx.send(ConnectorMsg {
			channel: connect(socket),
			resource: Resource::Batch(batch_id),
		})
		.await
		.unwrap_or_default();
	})
}

async fn connect_ws_person(
	ws: WebSocketUpgrade,
	Path(person_id): Path<u32>,
	State(wstx): State<async_channel::Sender<ConnectorMsg>>,
) -> Response {
	ws.on_upgrade(move |socket| async move {
		wstx.send(ConnectorMsg {
			channel: connect(socket),
			resource: Resource::Individual(person_id),
		})
		.await
		.unwrap_or_default();
	})
}

async fn new_batch(
	State((wtx, btx)): State<(db::WriteTx, async_channel::Sender<()>)>,
	Json(batch_data): Json<NewBatchData>,
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
