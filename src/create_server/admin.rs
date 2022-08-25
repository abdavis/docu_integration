use axum::{
	extract::{Extension, Json, Path},
	http::StatusCode,
	middleware,
	routing::{delete, post},
	Router,
};
use tokio::sync::oneshot;

use rand::{thread_rng, Rng};

use axum_macros::debug_handler;

use crate::{
	create_server::login::{verify_admin_session, Salt, TempPass, User, CONFIG},
	db,
};

pub fn create_routes(wtx: db::WriteTx, rtx: db::ReadTx) -> Router {
	Router::new()
		.route(
			"/admin/users",
			post(create_user).get(get_users).put(update_user),
		)
		.route("/admin/users/:username", delete(delete_user))
		.route("/admin/users/:username/reset_password", post(reset_pass))
		.layer(Extension((wtx, rtx)))
		.route_layer(middleware::from_fn(verify_admin_session))
}
#[debug_handler]
async fn create_user(
	Extension((wtx, _)): Extension<(db::WriteTx, db::ReadTx)>,
	Json(mut user): Json<User>,
) -> Result<(StatusCode, String), StatusCode> {
	let temp_pass = base64::encode(thread_rng().gen::<TempPass>());
	match argon2::hash_encoded(
		temp_pass.as_bytes(),
		&{
			let salt = thread_rng().gen::<Salt>();
			salt
		},
		&CONFIG,
	) {
		Ok(phc_hash) => {
			user.phc_hash = phc_hash;
			let (tx, rx) = oneshot::channel();
			wtx.send((db::WriteAction::CreateUser(user), tx))
				.unwrap_or_default();
			match rx.await {
				Ok(Ok(_)) => Ok((StatusCode::CREATED, temp_pass)),
				Ok(Err(db::WFail::Duplicate)) => Err(StatusCode::CONFLICT),
				_ => Err(StatusCode::INTERNAL_SERVER_ERROR),
			}
		}
		_ => Err(StatusCode::INTERNAL_SERVER_ERROR),
	}
}
async fn get_users(
	Extension((_, rtx)): Extension<(db::WriteTx, db::ReadTx)>,
) -> Result<Json<Vec<User>>, StatusCode> {
	let (tx, rx) = oneshot::channel();
	rtx.send((db::ReadAction::GetUsers, tx)).unwrap_or_default();
	match rx.await {
		Ok(Ok(db::RSuccess::Users(users))) => Ok(Json(users)),
		_ => Err(StatusCode::INTERNAL_SERVER_ERROR),
	}
}
async fn delete_user(
	Extension((wtx, _)): Extension<(db::WriteTx, db::ReadTx)>,
	Path(username): Path<String>,
) -> StatusCode {
	let (tx, rx) = oneshot::channel();
	wtx.send((db::WriteAction::DeleteUser(username), tx))
		.unwrap_or_default();
	match rx.await {
		Ok(Ok(_)) => StatusCode::OK,
		Ok(Err(db::WFail::NoRecord)) => StatusCode::NOT_FOUND,
		_ => StatusCode::INTERNAL_SERVER_ERROR,
	}
}
async fn reset_pass(
	Extension((wtx, _)): Extension<(db::WriteTx, db::ReadTx)>,
	Path(username): Path<String>,
) -> Result<String, StatusCode> {
	let temp_pass = base64::encode(thread_rng().gen::<TempPass>());
	match argon2::hash_encoded(
		temp_pass.as_bytes(),
		&{
			let salt = thread_rng().gen::<Salt>();
			salt
		},
		&CONFIG,
	) {
		Ok(phc_hash) => {
			let (tx, rx) = oneshot::channel();
			wtx.send((
				db::WriteAction::ResetPassword {
					id: username,
					phc_hash,
				},
				tx,
			))
			.unwrap_or_default();
			match rx.await {
				Ok(Ok(_)) => Ok(temp_pass),
				Ok(Err(db::WFail::NoRecord)) => Err(StatusCode::NOT_FOUND),
				_ => Err(StatusCode::INTERNAL_SERVER_ERROR),
			}
		}
		Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
	}
}
async fn update_user(
	Extension((wtx, _)): Extension<(db::WriteTx, db::ReadTx)>,
	Json(user): Json<User>,
) -> StatusCode {
	let (tx, rx) = oneshot::channel();
	wtx.send((db::WriteAction::UpdateUser(user), tx))
		.unwrap_or_default();
	match rx.await {
		Ok(Ok(_)) => StatusCode::OK,
		Ok(Err(db::WFail::NoRecord)) => StatusCode::NOT_FOUND,
		_ => StatusCode::INTERNAL_SERVER_ERROR,
	}
}
