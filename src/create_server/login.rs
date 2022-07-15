use axum::{
	extract::RequestParts,
	headers,
	http::{Request, StatusCode},
	middleware::{self, Next},
	response::{IntoResponse, Response},
	routing::{get, post, put},
	Extension, Router,
};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Token {
	sub: String,
	admin: bool,
	timeout: i64,
	exp: i64,
}
pub async fn verify_session_cookie<B: Send>(req: Request<B>, next: Next<B>) -> impl IntoResponse {
	let mut request_parts = RequestParts::new(req);

	StatusCode::INTERNAL_SERVER_ERROR.into_response()
}
