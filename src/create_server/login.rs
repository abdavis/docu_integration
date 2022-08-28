use argon2::{Config, ThreadMode, Variant, Version};
use axum::{
	extract::{Json, State},
	headers::Cookie,
	http::{header::SET_COOKIE, Request, StatusCode},
	middleware::Next,
	response::{IntoResponse, Response},
	routing::post,
	Router, TypedHeader,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use lazy_static::lazy_static;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::{
	cmp::min,
	time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
	sync::oneshot,
	time::{sleep, Duration},
};

use crate::db::{self, RSuccess};

lazy_static! {
	static ref JWT_KEY: [u8; 32] = thread_rng().gen();
	static ref JWT_ENCODE: EncodingKey = EncodingKey::from_secret(&*JWT_KEY);
	static ref JWT_DECODE: DecodingKey = DecodingKey::from_secret(&*JWT_KEY);
}

const COOKIE_NAME: &str = "session";
const COOKIE_PARAMS: &str = "SameSite=Strict; Secure; path=/";
pub const CONFIG: Config = Config {
	variant: Variant::Argon2id,
	version: Version::Version13,
	mem_cost: 65536,
	time_cost: 10,
	lanes: 4,
	thread_mode: ThreadMode::Parallel,
	secret: &[],
	ad: &[],
	hash_length: 32,
};

pub type Salt = [u8; 16];
pub type TempPass = [u8; 12];

pub async fn create_routes(wtx: db::WriteTx, rtx: db::ReadTx) -> Router<(db::WriteTx, db::ReadTx)> {
	let (tx, rx) = oneshot::channel();
	rtx.send((
		db::ReadAction::GetUser {
			user_id: "admin".into(),
		},
		tx,
	))
	.unwrap_or_default();

	match rx.await {
		Ok(Ok(RSuccess::User(maybe_user))) => {
			if let None = maybe_user {
				let temp_password = base64::encode(thread_rng().gen::<TempPass>());
				match argon2::hash_encoded(
					temp_password.as_bytes(),
					&thread_rng().gen::<Salt>(),
					&CONFIG,
				) {
					Ok(phc_hash) => {
						let (tx, _rx) = oneshot::channel();
						println!("creating admin user with temp password: {phc_hash}");
						wtx.send((
							db::WriteAction::CreateUser(User {
								id: "admin".into(),
								phc_hash,
								email: None,
								reset_required: true,
								admin: true,
							}),
							tx,
						))
						.unwrap_or_default()
					}
					_ => panic!(),
				}
			}
		}
		_ => panic!(),
	}

	Router::with_state((wtx, rtx))
		.route("/auth/login", post(login_handler))
		.route("/auth/change_pswd", post(change_pass_handler))
}

#[derive(Deserialize)]
struct Login {
	user: String,
	password: String,
}
#[derive(Deserialize)]
struct ChangePassword {
	user: String,
	new_password: String,
	old_password: String,
}
async fn change_pass_handler(
	State((wtx, rtx)): State<(db::WriteTx, db::ReadTx)>,
	Json(change): Json<ChangePassword>,
) -> Response {
	let sleep = sleep(Duration::from_secs(5));
	let (tx, rx) = oneshot::channel();
	rtx.send((
		db::ReadAction::GetUser {
			user_id: change.user,
		},
		tx,
	))
	.unwrap_or_default();
	match rx.await {
		Ok(Ok(RSuccess::User(Some(user)))) => {
			match argon2::verify_encoded(&user.phc_hash, change.old_password.as_bytes()) {
				Ok(true) => {
					match argon2::hash_encoded(
						change.new_password.as_bytes(),
						&{
							let salt = thread_rng().gen::<Salt>();
							salt
						},
						&CONFIG,
					) {
						Ok(phc) => {
							let (tx, rx) = oneshot::channel();
							wtx.send((
								db::WriteAction::ChangePassword {
									id: user.id,
									phc_hash: phc,
								},
								tx,
							))
							.unwrap_or_default();
							match rx.await {
								Ok(Ok(_)) => StatusCode::OK.into_response(),
								_ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
							}
						}
						_ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
					}
				}
				_ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			}
		}
		_ => {
			argon2::hash_encoded("foobar".as_bytes(), "somesalt".as_bytes(), &CONFIG)
				.unwrap_or_default();
			sleep.await;
			StatusCode::UNAUTHORIZED.into_response()
		}
	}
}

async fn login_handler(
	State((_, rtx)): State<(db::WriteTx, db::ReadTx)>,
	Json(login): Json<Login>,
) -> Response {
	let sleep = sleep(Duration::from_secs(5));
	let (tx, rx) = oneshot::channel();
	rtx.send((
		db::ReadAction::GetUser {
			user_id: login.user,
		},
		tx,
	))
	.unwrap_or_default();
	match rx.await {
		Ok(Ok(db::RSuccess::User(Some(user)))) => {
			match argon2::verify_encoded(&user.phc_hash, login.password.as_bytes()) {
				Ok(true) => match user.reset_required {
					true => (StatusCode::FORBIDDEN, "Password Reset Required").into_response(),
					false => {
						let time = SystemTime::now()
							.duration_since(UNIX_EPOCH)
							.expect("system clock error")
							.as_secs();
						([(
							SET_COOKIE,
							format!(
								"{}={}; Max-Age={}; {}",
								COOKIE_NAME,
								encode(
									&Header::default(),
									&Claims {
										sub: user.id,
										admin: user.admin,
										exp: time + 60 * 60,
										refresh: time + 60 * 60 * 8,
									},
									&JWT_ENCODE
								)
								.unwrap(),
								60 * 60,
								COOKIE_PARAMS
							),
						)])
						.into_response()
					}
				},
				_ => {
					sleep.await;
					StatusCode::UNAUTHORIZED.into_response()
				}
			}
		}
		_ => {
			//hash a nonsense password to simulate a similar workload
			argon2::hash_encoded("foobar".as_bytes(), "somesalt".as_bytes(), &CONFIG)
				.unwrap_or_default();
			sleep.await;
			StatusCode::UNAUTHORIZED.into_response()
		}
	}
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
	pub id: String,
	pub email: Option<String>,
	#[serde(skip)]
	pub phc_hash: String,
	pub reset_required: bool,
	pub admin: bool,
}
#[derive(Serialize, Deserialize)]
struct Claims {
	sub: String,
	admin: bool,
	exp: u64,
	refresh: u64,
}
pub async fn verify_user_session<B: Send>(
	TypedHeader(cookies): TypedHeader<Cookie>,
	req: Request<B>,
	next: Next<B>,
) -> Result<impl IntoResponse, Response> {
	verify_session(cookies, false, req, next).await
}

pub async fn verify_admin_session<B: Send>(
	TypedHeader(cookies): TypedHeader<Cookie>,
	req: Request<B>,
	next: Next<B>,
) -> Result<impl IntoResponse, Response> {
	verify_session(cookies, true, req, next).await
}

async fn verify_session<B: Send>(
	cookies: Cookie,
	admin: bool,
	req: Request<B>,
	next: Next<B>,
) -> Result<impl IntoResponse, Response> {
	match cookies.get("session") {
		None => Err(StatusCode::UNAUTHORIZED.into_response()),
		Some(cookie) => {
			let (new_token, max_age) = verify_jwt(cookie, admin)?;
			Ok((
				[(
					SET_COOKIE,
					format!(
						"{}={}; Max-Age={}; {}",
						COOKIE_NAME, new_token, max_age, COOKIE_PARAMS
					),
				)],
				next.run(req).await,
			))
		}
	}
}

fn verify_jwt(token: &str, admin_required: bool) -> Result<(String, u64), Response> {
	match decode::<Claims>(token, &JWT_DECODE, &Validation::new(Algorithm::HS256)) {
		Ok(token) => {
			if admin_required && !token.claims.admin {
				Err(StatusCode::FORBIDDEN.into_response())
			} else {
				let time = SystemTime::now()
					.duration_since(UNIX_EPOCH)
					.expect("system clock error")
					.as_secs();
				let new_claims = Claims {
					sub: token.claims.sub,
					admin: token.claims.admin,
					refresh: token.claims.refresh,
					exp: min(token.claims.refresh, time + 60 * 60),
				};
				Ok((
					encode(&Header::default(), &new_claims, &JWT_ENCODE).unwrap(),
					new_claims.exp - time,
				))
			}
		}
		Err(_) => Err((
			[(SET_COOKIE, format!("{}=deleted; Max-Age=0", COOKIE_NAME))],
			StatusCode::UNAUTHORIZED,
		)
			.into_response()),
	}
}
