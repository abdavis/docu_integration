use async_channel;
use serde::{Deserialize, Serialize};
use serde_json;
use std::{fs, thread};
use tokio::{self, sync::oneshot, task};
use toml;

mod batch_processor;
mod db;
mod oauth;

#[tokio::main]
async fn main() {
	let config: Config =
		toml::from_str(&fs::read_to_string("config.toml").expect("No Config File!"))
			.expect("Improper Config");

	// let mut token_auth = oauth::auth_initiate(&config);
	// token_auth.get().await;

	let (wtx, rtx, handles) = db::init();
	let (tx, rx) = oneshot::channel();
	wtx.send((
		db::WriteAction::NewBatch {
			batch_name: "Bob's Burgers 4".into(),
			description: "A wonderful burger joint".into(),
			records: vec![
				db::CsvImport {
					ssn: 123456789,
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
					ssn: 987654321,
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
		},
		tx,
	));

	println!("{:?}", rx.await);
	drop(wtx);
	drop(rtx);

	task::block_in_place(|| {
		for handle in handles {
			handle.join();
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
	cert_path: String,
	webhook_endpoint_url: String,
}

#[derive(Deserialize, Clone)]
pub struct DocusignCredentials {
	hmac_key: String,
	api_key: String,
	user_account_id: String,
	base_uri: String,
	templateId: String,
	user_id: String,
	auth_uri: String,
}
