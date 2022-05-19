use argon2::{Config, ThreadMode, Variant, Version};
use serde::Serialize;

const CONFIG: Config = Config {
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

#[derive(Debug, Serialize)]
pub struct User {
	pub id: String,
	pub email: String,
	pub phc_hash: String,
	pub reset_required: bool,
	pub admin: bool,
}
