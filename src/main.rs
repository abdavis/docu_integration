use toml;
use std::fs;
use serde::{Deserialize, Serialize};
use async_channel;
use tokio::{sync::oneshot, task};
use tokio;
use serde_json;

mod oauth;
mod worker;
mod db;

#[tokio::main]
async fn main(){
	let config: Config = toml::from_str(&fs::read_to_string("config.toml").expect("No Config File!"))
	.expect("Improper Config");
	
	let mut token_auth = oauth::auth_initiate(&config);
	

	

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