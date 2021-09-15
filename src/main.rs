use reqwest;
use toml;
use std::fs;
use serde::Deserialize;
use async_channel;
use tokio::{sync, task};
use tokio;

#[tokio::main]
async fn main(){
    let config: Config = toml::from_str(&fs::read_to_string("config.toml").expect("No Config File!"))
    .expect("Improper Config");

}

async fn reqwest_worker() {
    
}


#[derive(Deserialize)]
struct Config {
    network: Network,
    docusign_credentials: DocusignCredentials
}

#[derive(Deserialize)]
struct Network {
    cert_path: String,
    webhook_endpoint_url: String
}

#[derive(Deserialize)]
struct DocusignCredentials {
    hmac_key: String,
    api_key: String,
    api_account_id: String,
    base_uri: String
}