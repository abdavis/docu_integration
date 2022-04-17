use crate::db::{self, RSuccess, ReadAction};
use futures::prelude::*;
use rand::{thread_rng, Rng};
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot};
use tokio::task;
use tokio::time::{sleep, Duration};
use warp::ws::{Message, WebSocket};
pub struct ConnectorMsg {
	channel: oneshot::Sender<DbUpdater>,
	resource: Resource,
}

const KEEPALIVE_INTERVAL: u64 = 30;
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

async fn connector_task(
	rx: async_channel::Receiver<ConnectorMsg>,
	db_rtx: db::ReadTx,
	update_notifier: broadcast::Sender<()>,
) {
	let mut map = HashMap::new();

	let (close_tx, close_rx) = async_channel::bounded(1000);

	loop {
		tokio::select! {
			//new connections
			maybe_new_connect = rx.recv() => {
				match maybe_new_connect {
					Err(_) => break,
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
						updater_tx.send(new_connect.channel).await.unwrap_or_default();
					}
				}
			}
			//closed updaters
			close_msg_result = close_rx.recv() => {
				match close_msg_result{
					Err(_) => break,
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
	//first time setup, once we have sent one msg the loop handles future connections.
	if let Ok(new_subscriber_chnl) = incoming.recv().await {
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
			if let Ok(Ok(rsuccess)) = temp_rx.await {
				if let Ok(mut json_state) = serde_json::to_string(&rsuccess) {
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
							tokio::select! {
								//notify clients of new database changes
								recv_result = db_updates.recv() =>{
									match recv_result{
										Err(_) => break UpdaterCloseMsg{resource, leftover: None},
										Ok(_) => {
											let (tx, rx) = oneshot::channel();
											db_reader_channel.send((read_query.clone(), tx)).unwrap_or(());
											match rx.await {
												Ok(Ok(res)) => match serde_json::to_string(&res) {
													Ok(str) => {
														json_state = str;
														updater_tx.send(json_state.clone()).unwrap_or(0);
													}
													Err(_) => break UpdaterCloseMsg{resource, leftover: None}
												}
												_=> break UpdaterCloseMsg{resource, leftover: None},
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
											None => break UpdaterCloseMsg{resource, leftover: Some(new_client)}
										}
										Err(_) => break UpdaterCloseMsg{resource, leftover: None}
									}
								}
								//Close out of loop if all clients drop their handle
								_ = &mut subscriber_status_rx => {break UpdaterCloseMsg{resource, leftover: None}}
							}
						})
						.await
						.unwrap_or(());
				}
			}
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
				loop {
					tokio::select! {
						update = database_updates.receive_channel.recv() => {
							match update {
								Err(_) => break,
								Ok(s) => if let Err(_) = tx.send(Message::text(s)).await {break}
							}
						}
						ws_receive = rx.next() => {if let None = ws_receive {break}}

						_ = sleep(Duration::from_millis(KEEPALIVE_INTERVAL * 1000 + thread_rng().gen_range(0..(KEEPALIVE_INTERVAL * 500))))
							=> {if let Err(_) = tx.send(Message::ping([])).await {break}}
					}
				}
			}
		}
	});

	otx
}
