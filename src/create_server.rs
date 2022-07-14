use axum_server::{tls_rustls::RustlsConfig, Handle};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio::task;

use crate::{db, webhook, Config};

pub async fn run(
	config: Config,
	db_wtx: db::WriteTx,
	db_rtx: db::ReadTx,
	batch_processor_tx: async_channel::Sender<()>,
	completed_processor_tx: async_channel::Sender<()>,
	shutdown_signal: tokio::sync::broadcast::Receiver<()>,
) {
	let tls_config = RustlsConfig::from_pem_file(
		PathBuf::from("cert/cert.pem"),
		PathBuf::from("cert/key.pem"),
	)
	.await
	.expect("Unable to load certificates");

	let app = webhook::create_routes(&config, db_wtx.clone(), completed_processor_tx);

	let handle = Handle::new();

	task::spawn(graceful_shutdown(handle.clone(), shutdown_signal));

	let addr = SocketAddr::from(([0, 0, 0, 0], 8081));
	axum_server::bind_rustls(addr, tls_config)
		.handle(handle)
		.serve(app.into_make_service())
		.await
		.expect("Failed to start server");
}

async fn graceful_shutdown(
	handle: Handle,
	mut shutdown_signal: tokio::sync::broadcast::Receiver<()>,
) {
	shutdown_signal.recv().await;
	handle.graceful_shutdown(Some(Duration::from_secs(10)));
}
