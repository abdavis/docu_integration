use rusqlite::{params, Connection};
use crossbeam_channel;
use tokio::sync::oneshot;

pub enum WriteAction{
    NewBatch {
        batch_name: String,
        description: String,
        records: Vec<(Acct, CsvImport)>
    },
}

pub type WriteResult = Result<success, fail>;
pub enum success{

}
pub enum fail{

}

#[derive(Clone)]
pub struct CsvImport {
	pub ssn: u64,
	pub first_name: String,
	pub middle_name: Option<String>,
	pub last_name: String,
	pub dob: String,
	pub addr1: String,
	pub addr2: Option<String>,
	pub city: String,
	pub state: String,
	pub zip: String,
	pub email: String,
	pub phone: String,
	pub spouse: Option<CsvSpouse>
}
#[derive(Clone)]
pub struct CsvSpouse {
	pub first_name: String,
	pub middle_name: Option<String>,
	pub last_name: String,
	pub email: String
}

pub struct Acct {
    pub ssn: u64,
    pub primary_acct: Option<u64>,
    pub info_codes: Option<String>,
    pub created_acct: Option<u64>,
    pub host_err: Option<String>
}

fn database_writer(rx: crossbeam_channel::Receiver<(WriteAction, oneshot::Sender<WriteResult>)>) {

}