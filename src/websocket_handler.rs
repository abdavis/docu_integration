use crate::db::{self, ReadAction};
use futures::prelude::*;
use rand::{thread_rng, Rng};
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot};
use tokio::task;
use tokio::time::{sleep, sleep_until, Duration, Instant};
use warp::ws::{Message, WebSocket};
pub struct ConnectorMsg {
	pub channel: oneshot::Sender<DbUpdater>,
	pub resource: Resource,
}
// ping interval, in seconds
const PING_INTERVAL: u64 = 600;

//loop update min interval
const UPDATE_INTERVAL: Duration = Duration::from_secs(10);

pub struct DbUpdater {
	receive_channel: broadcast::Receiver<String>,
	initial_json: String,
	//never accessed, simply droping value notifies sender that all clients disconnected
	_disconnect_channel: Arc<oneshot::Sender<()>>,
}

#[derive(Hash, Eq, PartialEq, Copy, Clone)]
pub enum Resource {
	Main,
	Batch(i64),
	Individual(u32),
}

pub async fn connector_task(
	rx: async_channel::Receiver<ConnectorMsg>,
	db_rtx: db::ReadTx,
	update_notifier: broadcast::Sender<()>,
) {
	let mut map = HashMap::new();

	let (close_tx, close_rx) = async_channel::bounded(1000);

	loop {
		println!("connector loop");
		tokio::select! {
			//new connections
			maybe_new_connect = rx.recv() => {
				match maybe_new_connect {
					Err(_) => {println!("Exiting connecter loop, close_rx channel is closed"); break},
					Ok(new_connect) => {
						let (updater_tx, _updater_rx) = map.entry(new_connect.resource).or_insert({
							let (tx, rx) = async_channel::bounded(100);
							task::spawn({
								let rx = rx.clone();
								let db_rtx = db_rtx.clone();
								let update_notifier = update_notifier.subscribe();
								let close_tx = close_tx.clone();
								async move {updater_task(rx, update_notifier, db_rtx, close_tx, new_connect.resource).await}
							});
							(tx, rx)
						});
						updater_tx.send(new_connect.channel).await.expect("failed to send connection to updater");
					}
				}
			}
			//closed updaters
			close_msg_result = close_rx.recv() => {
				match close_msg_result{
					Err(_) => {println!("Exiting connecter loop, close_rx channel is closed"); break},
					Ok(msg) => match msg.leftover{
						Some(val) => if let Some((tx, rx)) = map.get(&msg.resource) {
							tx.send(val).await.unwrap_or_default();
							let rx = rx.clone();
							let update_notifier = update_notifier.subscribe();
							let db_rtx = db_rtx.clone();
							let close_tx = close_tx.clone();
							task::spawn(async move{updater_task(rx, update_notifier, db_rtx, close_tx, msg.resource).await});
						},
						None => if let Some((tx, rx)) = map.get(&msg.resource) {
							if tx.is_empty() {
								map.remove(&msg.resource);
							} else {
								task::spawn({
									let rx = rx.clone();
									let db_rtx = db_rtx.clone();
									let update_notifier = update_notifier.subscribe();
									let close_tx = close_tx.clone();
									async move {updater_task(rx, update_notifier, db_rtx, close_tx, msg.resource).await}
								});
							}
						}
					}
				}
			}
		}
	}
}

struct UpdaterCloseMsg {
	resource: Resource,
	leftover: Option<oneshot::Sender<DbUpdater>>,
}

async fn updater_task(
	incoming: async_channel::Receiver<oneshot::Sender<DbUpdater>>,
	mut db_updates: broadcast::Receiver<()>,
	db_reader_channel: db::ReadTx,
	close_channel: async_channel::Sender<UpdaterCloseMsg>,
	resource: Resource,
) {
	fn empty(rx: &mut broadcast::Receiver<()>) {
		while let Ok(_) = rx.try_recv() {}
	}
	//first time setup, once we have sent one msg the loop handles future connections.
	if let Ok(new_subscriber_chnl) = incoming.recv().await {
		println!("got first new subscriber");
		let (subscriber_status_tx, mut subscriber_status_rx) = oneshot::channel();
		let disconnect_handle = Arc::new(subscriber_status_tx);
		let weak_disconnect = Arc::downgrade(&disconnect_handle);
		let read_query = match resource {
			Resource::Main => ReadAction::ActiveBatches,
			Resource::Batch(num) => ReadAction::BatchDetail { rowid: num },
			Resource::Individual(num) => ReadAction::EnvelopeDetail { ssn: num },
		};
		let (temp_tx, temp_rx) = oneshot::channel();
		if let Ok(_) = db_reader_channel.try_send((read_query.clone(), temp_tx)) {
			match temp_rx.await {
				Ok(Ok(rsuccess)) => {
					println!("got msg from db");
					let mut current_state = rsuccess;
					if let Ok(mut json_state) = serde_json::to_string(&current_state) {
						let (updater_tx, updater_rx) = broadcast::channel(100);
						new_subscriber_chnl
							.send(DbUpdater {
								receive_channel: updater_rx,
								initial_json: json_state.clone(),
								_disconnect_channel: disconnect_handle,
							})
							.unwrap_or(());

						//start of logic loop
						close_channel
						.send(loop {
							println!("updater loop");
							tokio::select! {
								//notify clients of new database changes
								recv_result = db_updates.recv() =>{
									match recv_result{
										Err(_) => {println!("Exiting updater loop, db update channel is closed"); break UpdaterCloseMsg{resource, leftover: None}},
										Ok(_) => {
											let (tx, rx) = oneshot::channel();
											empty(&mut db_updates);
											db_reader_channel.send((read_query.clone(), tx)).unwrap_or_default();
											match rx.await {
												Ok(Ok(res)) => {
													if res != current_state {
														current_state = res;
														match serde_json::to_string(&current_state) {
														Ok(str) => {
															json_state = str;
															updater_tx.send(json_state.clone()).unwrap_or_default();
														}
														Err(_) => {println!("Exiting updater loop, unable to parse db response"); break UpdaterCloseMsg{resource, leftover: None}}
													}
												}
												sleep(UPDATE_INTERVAL).await;
											}
												_=> {println!("Exiting updater loop, new client oneshot channel error or db read error"); break UpdaterCloseMsg{resource, leftover: None}},
											}
										}
									}
								}
								//accept new clients
								maybe_new_client = incoming.recv() => {
									match maybe_new_client{
										Ok(new_client) => match weak_disconnect.upgrade(){
											Some(new_disconnect_handle) => {new_client.send(DbUpdater {
												receive_channel: updater_tx.subscribe(), initial_json: json_state.clone(), _disconnect_channel: new_disconnect_handle
											}).unwrap_or(());},
											None => {println!("Exiting updater loop, all clients disconnected while adding a new client"); break UpdaterCloseMsg{resource, leftover: Some(new_client)}}
										}
										Err(_) => {println!("Exiting updater loop, new_client_channel error"); break UpdaterCloseMsg{resource, leftover: None}}
									}
								}
								//Close out of loop if all clients drop their handle
								_ = &mut subscriber_status_rx => {println!("Exiting updater loop, all clients are gone"); break UpdaterCloseMsg{resource, leftover: None}}
							}
						})
						.await
						.unwrap_or_default();
					}
				}
				result => {
					println!("error in updater task when reading from db: {result:?}");
					close_channel
						.send(UpdaterCloseMsg {
							resource,
							leftover: None,
						})
						.await
						.unwrap_or_default();
				}
			}
		} else {
			println!("error sending action to db");
			close_channel
				.send(UpdaterCloseMsg {
					resource,
					leftover: None,
				})
				.await
				.unwrap_or_default();
		}
	}
}

pub fn connect(ws: WebSocket) -> oneshot::Sender<DbUpdater> {
	let (otx, orx): (
		tokio::sync::oneshot::Sender<DbUpdater>,
		tokio::sync::oneshot::Receiver<DbUpdater>,
	) = oneshot::channel();
	//connection handling logic here
	task::spawn(async move {
		if let Ok(mut database_updates) = orx.await {
			let (mut tx, mut rx) = ws.split();
			if let Ok(_) = tx.send(Message::text(database_updates.initial_json)).await {
				let mut ping_deadline = Instant::now()
					+ Duration::from_millis(
						PING_INTERVAL * 1000 + thread_rng().gen_range(0..(PING_INTERVAL * 500)),
					);

				let mut pong_deadline = Instant::now() + Duration::from_secs(PING_INTERVAL * 2);

				loop {
					tokio::select! {
						update = database_updates.receive_channel.recv() => {
							match update {
								Err(_) => break,
								Ok(s) => if let Err(_) = tx.send(Message::text(s)).await {break}
							}
						}

						ws_receive = rx.next() => {
							match ws_receive {
								None => {println!("Websocked receiver closed"); break},
								Some(Ok(msg)) => if msg.is_pong() {pong_deadline = Instant::now() + Duration::from_secs(PING_INTERVAL * 2);},
								_=>()
							}
						}

						_ = sleep_until(ping_deadline)
							=> {
								if let Err(_) = tx.send(Message::ping([])).await {
									println!("exiting connect loop from timeout");
									break
								}
								ping_deadline = Instant::now()
									+ Duration::from_millis(
										PING_INTERVAL * 1000
											+ thread_rng().gen_range(0..(PING_INTERVAL * 500)),
									);
							}

						_= sleep_until(pong_deadline) => {println!("No pong received"); break}
					}
				}
			}
		}
	});

	otx
}
