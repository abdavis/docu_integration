use async_channel;
use serde::Deserialize;
use std::fs;
use tokio::{self, sync::oneshot, task};
use toml;

mod batch_processor;
mod create_server;
mod db;
//mod login_handler;
mod oauth;
//mod server;
mod websocket_handler;

const DB_SCHEMA: &'static str = include_str!("schema.sql");

#[tokio::main]
async fn main() {
	let config: Config =
		toml::from_str(&fs::read_to_string("config.toml").expect("No Config File!"))
			.expect("Improper Config");

	let token_auth = oauth::auth_initiate(&config);

	let (wtx, rtx, db_update_tx, handles) = db::init();
	let (tx, rx) = oneshot::channel();
	wtx.send((
		db::WriteAction::NewBatch(db::BatchData {
			name: "Bob's Burgers 12".into(),
			description: "A wonderful burger joint".into(),
			records: vec![
				db::CsvImport {
					ssn: 1236789010,
					first_name: "Bob".into(),
					middle_name: None,
					last_name: "Duncan".into(),
					dob: "1988-05-25".into(),
					addr1: "123 fake st".into(),
					addr2: None,
					city: "Ogden".into(),
					state: "Utah".into(),
					zip: "84414".into(),
					email: "abdavis7@gmail.com".into(),
					phone: "123-45-6789".into(),
					spouse: None,
				},
				db::CsvImport {
					ssn: 654321010,
					first_name: "Bob".into(),
					middle_name: None,
					last_name: "Duncan".into(),
					dob: "1988-05-25".into(),
					addr1: "123 fake st".into(),
					addr2: None,
					city: "Ogden".into(),
					state: "Utah".into(),
					zip: "84414".into(),
					email: "abdavis7@gmail.com".into(),
					phone: "123-45-6789".into(),
					spouse: None,
				},
				db::CsvImport {
					ssn: 12345610,
					first_name: "Bob".into(),
					middle_name: None,
					last_name: "Duncan".into(),
					dob: "1988-05-25".into(),
					addr1: "123 fake st".into(),
					addr2: None,
					city: "Ogden".into(),
					state: "Utah".into(),
					zip: "84414".into(),
					email: "abdavis7@gmail.com".into(),
					phone: "123-45-6789".into(),
					spouse: None,
				},
				db::CsvImport {
					ssn: 98765410,
					first_name: "Bob".into(),
					middle_name: None,
					last_name: "Duncan".into(),
					dob: "1988-05-25".into(),
					addr1: "123 fake st".into(),
					addr2: None,
					city: "Ogden".into(),
					state: "Utah".into(),
					zip: "84414".into(),
					email: "abdavis7@gmail.com".into(),
					phone: "123-45-6789".into(),
					spouse: None,
				},
			],
		}),
		tx,
	))
	.unwrap_or_default();
	let db_result = rx.await;
	println!("db write status: {db_result:?}");
	let client = reqwest::Client::new();

	let mut tasks = vec![];

	let (processor_tx, proc_handle) = batch_processor::init_batch_processor(
		&client,
		&config,
		token_auth,
		wtx.clone(),
		rtx.clone(),
	);
	tasks.push(proc_handle);
	let (ws_handler_tx, ws_handler_rx) = async_channel::bounded(1000);
	tasks.push(task::spawn(websocket_handler::connector_task(
		ws_handler_rx,
		rtx.clone(),
		db_update_tx,
	)));
	processor_tx.send(()).await.unwrap_or_default();
	let (completed_tx, completed_rx) = async_channel::bounded(1);
	let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
	tasks.push(task::spawn(create_server::run(
		config,
		wtx,
		rtx,
		ws_handler_tx,
		processor_tx,
		completed_tx,
		shutdown_rx,
	)));

	for task in tasks {
		task.await.unwrap_or_default();
	}
	//println!("{:?}", rx.await);
	task::block_in_place(|| {
		for handle in handles {
			handle.join().unwrap_or_default();
		}
	})
}

#[derive(Deserialize, Clone)]
pub struct Config {
	network: Network,
	docusign: DocusignCredentials,
}

#[derive(Deserialize, Clone)]
struct Network {
	endpoint_url: String,
}

#[derive(Deserialize, Clone)]
#[allow(non_snake_case)]
pub struct DocusignCredentials {
	hmac_key: String,
	api_key: String,
	user_account_id: String,
	base_uri: String,
	templateId: String,
	user_id: String,
	auth_uri: String,
}
