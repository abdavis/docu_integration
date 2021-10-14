use reqwest::Client;
use toml;
use std::fs;
use serde::{Deserialize, Serialize};
use async_channel;
use tokio::{sync::oneshot, task};
use tokio;
use serde_json;

mod oauth;
mod worker;

#[tokio::main]
async fn main(){
	let config: Config = toml::from_str(&fs::read_to_string("config.toml").expect("No Config File!"))
	.expect("Improper Config");
	let test_envelope = Envelope {
		templateId: config.docusign.templateId.clone(),
		status: "sent".to_string(),
			templateRoles: vec!(
			EnvelopeRecipient {
				name: "Bob J. Duncan".to_string(),
				email: "abdavis7@gmail.com".to_string(),
				roleName: "Signer".to_string(),
				tabs: Some(Tabs {
						textTabs: vec![
							TabValue {
								tabLabel: "SSN".to_string(),
								value: "123456789".to_string()
							},
							TabValue {
								tabLabel: "DOB".to_string(),
								value: "1990-05-12".to_string()
							},
							TabValue {
								tabLabel: "address".to_string(),
								value: "123 Fake ST".to_string()
							},
							TabValue {
								tabLabel: "cit-st-zip".to_string(),
								value: "Ogden UT 84401".to_string()
							},
							TabValue {
								tabLabel: "phone".to_string(),
								value: "(801)123-4567".to_string()
							}
						]
					}
				)
			},
		),
	};
	println!("{}", serde_json::to_string_pretty(&test_envelope).unwrap());
	let mut token_auth = oauth::auth_initiate(&config);
	let client = reqwest::Client::new();
	let builder = client.post(String::from("https://") + &config.docusign.base_uri + "/v2.1/accounts/" + &config.docusign.user_account_id + "/envelopes")
		.bearer_auth(token_auth.get().await)
		.header("content-Type", "application/json")
		.body(serde_json::to_string(&test_envelope).unwrap());

	let result = builder.send().await.unwrap();
	println!("{:?}", result);
	let body = result.text().await.unwrap();
	println!("{}", body);

}

pub struct CsvImport {
	ssn: u64,
	first_name: String,
	middle_name: Option<String>,
	last_name: String,
	dob: String,
	addr1: String,
	addr2: Option<String>,
	city: String,
	state: String,
	zip: String,
	email: String,
	phone: String,
	spouse: Option<CsvSpouse>
}
struct CsvSpouse {
	first_name: String,
	middle_name: Option<String>,
	last_name: String,
	email: String
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