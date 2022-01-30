use toml;
use std::{fs, thread};
use serde::{Deserialize, Serialize};
use async_channel;
use tokio::{sync::oneshot, task, self};
use serde_json;

mod oauth;
mod worker;
mod db;

#[tokio::main]
async fn main(){
	let config: Config = toml::from_str(&fs::read_to_string("config.toml").expect("No Config File!"))
	.expect("Improper Config");
	
	let mut token_auth = oauth::auth_initiate(&config);

	token_auth.get().await;
	
	let (wtx, rtx, handles) = db::init();
	let (tx, rx) = oneshot::channel();
	wtx.send((db::WriteAction::NewBatch{batch_name: "Bob's Burgers 2".into(), description: "A wonderful burger joint".into(), records: vec![]}, tx));

	println!("{:?}", rx.await);
	drop(wtx);
	drop(rtx);

	task::block_in_place( || {
		for handle in handles{
			handle.join();
		}
	})

}

#[derive(Deserialize, Clone)]
pub struct Config {
	network: Network,
	docusign: DocusignCredentials
}

#[derive(Deserialize, Clone)]
struct Network {
	cert_path: String,
	webhook_endpoint_url: String
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