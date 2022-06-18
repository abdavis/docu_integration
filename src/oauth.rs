use crate::Config;
use async_channel;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::oneshot;
use tokio::task;
pub fn auth_initiate(config: &Config) -> AuthHelper {
	let (tx, rx) = async_channel::unbounded();
	let shadow = config.clone();
	task::spawn(async {
		auth_server(shadow, rx).await;
	});
	AuthHelper {
		token: "".to_string(),
		expire: Instant::now(),
		tx: tx,
	}
}

async fn auth_server(
	config: Config,
	rx: async_channel::Receiver<oneshot::Sender<(String, Instant)>>,
) {
	println!("Auth server started");
	let rsa_pem =
		std::fs::read("oauth_keys/rsa_private_key.pem").expect("rsa_private_key.pem is missing.");
	let key = EncodingKey::from_rsa_pem(&rsa_pem).unwrap();
	let head = Header::new(Algorithm::RS256);
	let http_client = reqwest::Client::new();
	let mut claims = Claims {
		sub: config.docusign.user_id,
		iss: config.docusign.api_key,
		aud: config.docusign.auth_uri,
		scope: "signature impersonation".to_string(),
		iat: 0,
		exp: 0,
	};
	claims.update();
	let mut token = ("".to_string(), Instant::now());
	while let Ok(tx) = rx.recv().await {
		if token.1 > Instant::now() {
			tx.send(token.clone()).unwrap_or_default();
		} else {
			claims.update();
			token = renew_token(&head, &claims, &key, &http_client).await;
			tx.send(token.clone()).unwrap_or_default();
		}
	}

	async fn renew_token(
		head: &Header,
		claims: &Claims,
		key: &EncodingKey,
		http_client: &Client,
	) -> (String, Instant) {
		let jwt = encode(head, claims, key).unwrap();
		let res = http_client
			.post("https://".to_string() + &claims.aud + "/oauth/token")
			.body(
				"grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer&assertion=".to_string()
					+ &jwt,
			)
			.header("Content-Type", "application/x-www-form-urlencoded")
			.send()
			.await
			.unwrap();
		let json: serde_json::Value = serde_json::from_str(&res.text().await.unwrap()).unwrap();
		(
			json["access_token"].as_str().unwrap().to_string(),
			Instant::now() + Duration::from_secs(3000),
		)
	}
}
#[derive(Serialize)]
struct Claims {
	sub: String,
	iss: String,
	aud: String,
	scope: String,
	iat: u64,
	exp: u64,
}

impl Claims {
	fn update(&mut self) {
		let time = SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH)
			.unwrap()
			.as_secs();
		self.iat = time;
		self.exp = time + 3600;
	}
}
#[derive(Clone)]
pub struct AuthHelper {
	token: String,
	expire: Instant,
	tx: async_channel::Sender<oneshot::Sender<(String, Instant)>>,
}

impl AuthHelper {
	pub async fn get(&mut self) -> String {
		if self.expire <= Instant::now() {
			let (oneshottx, oneshotrx) = oneshot::channel();
			self.tx.send(oneshottx).await.unwrap_or_default();
			let new_token = oneshotrx.await.unwrap();
			self.token = new_token.0;
			self.expire = new_token.1;
		}
		self.token.clone()
	}
}
