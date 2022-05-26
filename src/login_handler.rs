use argon2::{Config, ThreadMode, Variant, Version};
use dashmap::{mapref::entry::Entry, DashMap};
use rand::{thread_rng, Rng};
use serde::Serialize;
use std::cmp::min;
use std::collections::binary_heap::PeekMut;
use std::collections::BinaryHeap;
use std::sync::Arc;
use tokio::select;
use tokio::sync::oneshot;
use tokio::task;
use tokio::time::{sleep, sleep_until, Duration, Instant};

use crate::db::{RSuccess, ReadAction, ReadTx, WriteAction, WriteTx};

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

const DELAY_MILLIS: u64 = 3000;

const SESSION_TIMEOUT: u64 = 60 * 60; //one hour

const SESSION_EXPIRY: u64 = 60 * 60 * 8; //eight hours

#[derive(Debug, Serialize, Clone)]
pub struct User {
	pub id: String,
	pub email: String,
	pub phc_hash: String,
	pub reset_required: bool,
	pub admin: bool,
}

struct Session {
	user: User,
	timeout: Instant,
	expiry: Instant,
}

type Map = Arc<DashMap<[u8; 32], Session>>;

#[derive(Clone)]
pub struct PasswordManager {
	map: Map,
	db_rtx: ReadTx,
	db_wtx: WriteTx,
	cleanup_tx: async_channel::Sender<CleanupKey>,
}

pub enum LoginResult {
	Success(String),
	MustChangePwd,
	Failed,
}

pub enum PswdChangeResult {
	Success,
	MismatchedPswds,
	Failed,
}

impl PasswordManager {
	pub async fn login(&self, usr: String, pswd: String) -> LoginResult {
		let timeout = sleep(Duration::from_millis(
			//delay somewhere between 1 and 1.5 times the configured value
			thread_rng().gen_range(DELAY_MILLIS..(DELAY_MILLIS * 3 / 2)),
		));
		let (tx, rx) = oneshot::channel();
		self.db_rtx
			.send((ReadAction::GetUser { user_id: usr }, tx))
			.unwrap_or_default();

		let pass_result = match rx.await {
			Ok(Ok(RSuccess::User(Some(fetched_user)))) => {
				match argon2::verify_encoded(&fetched_user.phc_hash, pswd.as_bytes()) {
					Err(_) => {
						println!("Error hashing password");
						LoginResult::Failed
					}
					Ok(b) => match b {
						true => match fetched_user.reset_required {
							true => LoginResult::MustChangePwd,
							false => {
								let random_bytes = thread_rng().gen();
								let encoded = base64::encode(random_bytes);
								self.map.insert(
									random_bytes,
									Session {
										user: fetched_user,
										timeout: Instant::now()
											+ Duration::from_secs(SESSION_TIMEOUT),
										expiry: Instant::now()
											+ Duration::from_secs(SESSION_EXPIRY),
									},
								);
								self.cleanup_tx
									.send(CleanupKey {
										time: Instant::now() + Duration::from_secs(SESSION_TIMEOUT),
										key: random_bytes,
									})
									.await
									.unwrap_or_default();
								LoginResult::Success(encoded)
							}
						},
						false => LoginResult::Failed,
					},
				}
			}
			catch => {
				println!("{catch:?}");
				LoginResult::Failed
			}
		};

		if let LoginResult::Failed = pass_result {
			timeout.await;
		}
		pass_result
	}

	pub async fn change_pswd(
		&self,
		usr: String,
		pwd: String,
		new_pwd: String,
		new_pwd2: String,
	) -> PswdChangeResult {
		if new_pwd != new_pwd2 {
			return PswdChangeResult::MismatchedPswds;
		}

		let timeout = sleep(Duration::from_millis(
			//delay somewhere between 1 and 1.5 seconds
			thread_rng().gen_range(DELAY_MILLIS..(DELAY_MILLIS * 3 / 2)),
		));
		let (tx, rx) = oneshot::channel();
		self.db_rtx
			.send((ReadAction::GetUser { user_id: usr }, tx))
			.unwrap_or_default();

		let pass_update_result = match rx.await {
			Ok(Ok(RSuccess::User(Some(fetched_user)))) => {
				match argon2::verify_encoded(&fetched_user.phc_hash, pwd.as_bytes()) {
					Err(_) => {
						println!("Error hashing password");
						PswdChangeResult::Failed
					}
					Ok(b) => match b {
						true => {
							match argon2::hash_encoded(
								new_pwd.as_bytes(),
								&thread_rng().gen::<[u8; 16]>(),
								&CONFIG,
							) {
								Err(_) => PswdChangeResult::Failed,
								Ok(new_phc) => {
									let (tx, rx) = oneshot::channel();
									self.db_wtx
										.send((
											WriteAction::UpdateUser(User {
												id: fetched_user.id,
												email: fetched_user.email,
												phc_hash: new_phc,
												reset_required: false,
												admin: fetched_user.admin,
											}),
											tx,
										))
										.unwrap_or_default();
									match rx.await {
										Ok(Ok(_)) => PswdChangeResult::Success,
										catch => {
											println! {"{catch:?}"};
											PswdChangeResult::Failed
										}
									}
								}
							}
						}
						false => PswdChangeResult::Failed,
					},
				}
			}
			catch => {
				println!("{catch:?}");
				PswdChangeResult::Failed
			}
		};

		if let PswdChangeResult::Failed = pass_update_result {
			timeout.await;
		}
		pass_update_result
	}
}

pub struct SessionManager {
	map: Map,
	db_wtx: WriteTx,
	db_rtx: ReadTx,
}
pub enum SessionKind {
	User,
	Admin,
}
impl SessionManager {
	pub fn verify_session_token(&self, token: &str) -> Option<User> {
		if let Ok(decoded_vec) = base64::decode(token) {
			if let Ok(key) = decoded_vec.try_into() {
				if let Entry::Occupied(mut occ) = self.map.entry(key) {
					let session = occ.get_mut();
					session.timeout = min(
						Instant::now() + Duration::from_secs(SESSION_TIMEOUT),
						session.expiry,
					);
					return Some(session.user.clone());
				}
			}
		}
		None
	}

	pub fn logout(&self, token: &str) {
		if let Ok(decoded_vec) = base64::decode(token) {
			if let Ok(key) = decoded_vec.try_into() {
				let typed_key: [u8; 32] = key;
				self.map.remove(&key);
			}
		}
	}

	pub fn logout_everywhere(&self, token: &str) {
		if let Some(usr) = self.verify_session_token(token) {
			self.map.retain(|_, v| v.user.id != usr.id);
		}
	}
}

#[derive(Eq)]
struct CleanupKey {
	time: Instant,
	key: [u8; 32],
}
impl PartialEq for CleanupKey {
	fn eq(&self, other: &Self) -> bool {
		self.time == other.time
	}
}
impl Ord for CleanupKey {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		//comparing in reverse order, since we want earlier times to be "larger" in the binary heap
		other.time.cmp(&self.time)
	}
}
impl PartialOrd for CleanupKey {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}
async fn cleanup_handler(rx: async_channel::Receiver<CleanupKey>, map: Map) {
	let mut heap: BinaryHeap<CleanupKey> = std::collections::BinaryHeap::new();
	loop {
		select! {
			maybe_cleanup = rx.recv() => match maybe_cleanup {
				Ok(cleanup) => heap.push(cleanup),
				Err(_) => break
			},
			true = async {
				match heap.peek() {
					Some(val) => {
						sleep_until(val.time).await;
						true
					},
					None => false
				}
			} => {
				if let Some(mut peeked) = heap.peek_mut() {
					match map.entry(peeked.key) {
						Entry::Vacant(_) => {PeekMut::pop(peeked);},
						Entry::Occupied(occ_entry) => {
							let session = occ_entry.get();
							if Instant::now() > session.timeout {
								occ_entry.remove();
								PeekMut::pop(peeked);
							} else {
								peeked.time = session.timeout;
							}
						}
					}
				}
			}
		}
	}
}

pub fn new(db_rtx: ReadTx, db_wtx: WriteTx) -> (PasswordManager, SessionManager) {
	let map = Arc::new(DashMap::new());
	let (tx, rx) = async_channel::bounded(1000);

	task::spawn(cleanup_handler(rx, map.clone()));

	(
		PasswordManager {
			map: map.clone(),
			db_rtx: db_rtx.clone(),
			db_wtx: db_wtx.clone(),
			cleanup_tx: tx,
		},
		SessionManager {
			map,
			db_rtx,
			db_wtx,
		},
	)
}
