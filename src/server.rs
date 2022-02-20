use ring::hmac;
use tokio::task;
use warp::{
	http::{self, HeaderMap},
	hyper::body::Bytes,
	Filter, Reply,
};
pub fn create_server(config: &crate::Config) -> task::JoinHandle<()> {
	let config = config.clone();
	task::spawn(async move { server(config).await })
}
async fn server(config: crate::Config) {
	println!("Building server");
	let key = hmac::Key::new(hmac::HMAC_SHA256, config.docusign.hmac_key.as_bytes());
	let webhook = warp::path("webhook")
		.and(warp::post())
		.and(warp::body::content_length_limit(4194304))
		.and(warp::header::headers_cloned())
		.and(warp::body::bytes())
		.then(move |headers: HeaderMap, bytes: Bytes| {
			let key = key.clone();
			async move {
				match verify_msg(&key, &headers, &bytes) {
					Ok(_) => {
						println!("Message is Valid!");
						process_msg(bytes).await.into_response()
					}
					Err(string) => {
						println!("{string}");
						warp::reply::with_status(warp::reply(), http::StatusCode::UNAUTHORIZED)
							.into_response()
					}
				}
			}
		});

	warp::serve(webhook)
		.tls()
		.cert_path("cert/cert.pem")
		.key_path("cert/key.pem")
		.run(([0, 0, 0, 0], 443))
		.await;

	println!("Shutting down Server");
}
async fn process_msg(bytes: Bytes) -> impl Reply {
	warp::reply()
}
fn verify_msg(key: &hmac::Key, headers: &HeaderMap, bytes: &Bytes) -> Result<(), String> {
	let calculated_tag = base64::encode(hmac::sign(key, bytes));
	println!("calculated_tag: {calculated_tag}");
	println!("webhook headers: {headers:?}");
	for n in 1..101 {
		match headers.get(format!("X-DocuSign-Signature-{n}")) {
			None => return Err("hmac authentication codes are invalid or missing".into()),
			Some(header_tag) => match header_tag.to_str() {
				Ok(tag) => {
					if tag == calculated_tag {
						return Ok(());
					}
				}
				Err(_) => return Err("unable to parse header tag".into()),
			},
		}
	}
	Err("all 100 provided hmac authentication codes are invalid".into())
}
