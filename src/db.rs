use rusqlite::{params, Connection};
use crossbeam_channel;
use tokio::sync::oneshot;

const READ_THREADS: u8 = 4;

pub enum WriteAction{
    NewBatch {
        batch_name: String,
        description: String,
        records: Vec<CsvImport>
    },
	UpdateAcct(Acct)
}

pub type WriteResult = Result<WSuccess, WFail>;
pub enum WSuccess{

}
pub enum WFail{
	duplicate,
	err
}

pub enum ReadAction{
	ActiveBatches,
	BatchDetail,
	OldBatches{page: u32}
}

pub type ReadResult = Result<RSuccess, RFail>;
pub enum RSuccess{
	ActiveBatches(Vec<BatchSummary>),
	BatchDetail(Vec<(CsvImport, Acct)>),
	OldBatches(Vec<BatchSummary>)
}
pub enum RFail{
	Err
}

#[derive(Clone)]
pub struct CsvImport{
	pub ssn: u32,
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
    pub ssn: u32,
    pub primary_acct: Option<u32>,
    pub info_codes: Option<String>,
    pub created_acct: Option<u32>,
    pub host_err: Option<String>
}

pub struct BatchSummary{
	pub name: String,
	pub description: String,
	pub total: u32,
	pub working: u32,
	pub complete: u32,
	pub err: u32
}
fn database_writer(rx: crossbeam_channel::Receiver<(WriteAction, oneshot::Sender<WriteResult>)>) {

}

fn database_reader(rx: crossbeam_channel::Receiver<(ReadAction, oneshot::Sender<ReadResult>)>){

}