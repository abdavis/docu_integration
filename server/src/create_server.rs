//use axum_server::tls_rustls::RustlsConfig;
//use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio::{sync::oneshot, task};

use crate::{db, Config};
use axum::routing::get;

//mod admin;
mod api;
//pub mod login;
mod webhook;
pub mod websocket_handler;

pub async fn run(
	config: Config,
	db_wtx: db::WriteTx,
	db_rtx: db::ReadTx,
	ws_handler_tx: async_channel::Sender<websocket_handler::ConnectorMsg>,
	batch_processor_tx: async_channel::Sender<()>,
	completed_processor_tx: async_channel::Sender<()>,
	shutdown_signal: tokio::sync::broadcast::Receiver<()>,
) {
	let app = webhook::create_routes(&config, db_wtx.clone(), completed_processor_tx)
		//.merge(login::create_routes(db_wtx.clone(), db_rtx.clone()).await)
		//.merge(admin::create_routes(db_wtx.clone(), db_rtx.clone()))
		.merge(api::create_routes(
			db_wtx,
			batch_processor_tx,
			ws_handler_tx,
		))
		.route("/", get(hello_world));

	let (tx, rx) = oneshot::channel();

	task::spawn(graceful_shutdown(tx, shutdown_signal));

	axum::Server::bind(&([127, 0, 0, 1], 8081).into())
		.serve(app.into_make_service())
		.with_graceful_shutdown(async {
			rx.await.ok();
		})
		.await
		.expect("Failed to start server");
}

async fn hello_world() -> &'static str {
	"Hello, World"
}

async fn graceful_shutdown(
	handle: oneshot::Sender<()>,
	mut shutdown_signal: tokio::sync::broadcast::Receiver<()>,
) {
	shutdown_signal.recv().await.unwrap_or_default();
	handle.send(()).unwrap_or_default();
}
