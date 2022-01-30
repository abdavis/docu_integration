use crossbeam_channel;
use rusqlite::{config::DbConfig, named_params, Connection, OpenFlags, ffi::{Error as SqliteError}};
use std::thread;
use tokio::sync::oneshot;

const READ_THREADS: usize = 4;
const WRITE_CHANNEL_SIZE: usize = 100;
const READ_CHANNEL_SIZE: usize = 400;

pub fn init() -> (
	crossbeam_channel::Sender<(WriteAction, oneshot::Sender<WriteResult>)>,
	crossbeam_channel::Sender<(ReadAction, oneshot::Sender<ReadResult>)>,
	Vec<thread::JoinHandle<()>>,
) {
	let (wtx, wrx) = crossbeam_channel::bounded(WRITE_CHANNEL_SIZE);
	let mut handles = vec![];
	handles.push(thread::spawn(|| database_writer(wrx)));

	let (rtx, rrx) = crossbeam_channel::bounded(READ_CHANNEL_SIZE);
	for _n in 0..READ_THREADS {
		let rrx = rrx.clone();
		handles.push(thread::spawn(|| database_reader(rrx)))
	}

	(wtx, rtx, handles)
}

fn database_writer(rx: crossbeam_channel::Receiver<(WriteAction, oneshot::Sender<WriteResult>)>) {
	let mut conn = Connection::open_with_flags(
		"db.sqlite3",
		OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
	)
	.expect("Unable to Open Database. Is it missing?");
	conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)
		.expect("Enable Foreign Key Failed.");
	
	conn.pragma_update(None, "journal_mode", "WAL").expect("Setting Wal mode failed");
	conn.pragma_update(None, "synchronous", "normal").expect("Setting synchronous mode failed");
	conn.pragma_update(None, "temp_store", "memory").expect("setting temp_store failed");
	conn.pragma_update(None, "mmap_size", 30000000000 as u64).expect("unable to set mmap size");

	for (event, tx) in rx {
		match conn.transaction() {
			Err(_) => {
				tx.send(WriteResult::Err(WFail::DbError("Unable to begin transaction".into())));
			},
			Ok(tran) => {
				let mut dups = 0;

				 match match event {
					WriteAction::NewBatch {
						batch_name,
						description,
						records,
					} => {
						match tran.prepare_cached("INSERT INTO company_batches (batch_name, description) VALUES (:name, :desc)") {
							Err(_) => WriteResult::Err(WFail::DbError("Prepare Company Batches Failed".into())),
							Ok(mut stmt) => {
								match stmt.insert(named_params! {":name": batch_name, ":desc": description}){
									//Duplicate insert path
									Err(rusqlite::Error::SqliteFailure(SqliteError{ code: _, extended_code: 2067 }, _)) => {WriteResult::Err(WFail::Duplicate)},
									//All other errors
									Err(_) => WriteResult::Err(WFail::DbError("Failed to insert into company_batches".into())),
									
									Ok(id) => {
										match (
											tran.prepare_cached("INSERT INTO acct_data (ssn) VALUES (:ssn)"),
											tran.prepare_cached("INSERT INTO ssn_batch_relat (batch_id, ssn) VALUES (:id, :ssn)"),
											tran.prepare_cached("INSERT INTO envelopes (ssn, fname, mname, lname, dob, addr1, addr2, city, state, zip, email, phone, spouse_fname, spouse_mname, spouse_lname, spouse_email)
												VALUES (:ssn, :fname, :mname, :lname, :dob, :addr1, :addr2, :city, :state, :zip, :email, :phone, :spouse_fname, :spouse_mname, :spouse_lname, :spouse_email)")
										){
											(Ok(mut acct_ins), Ok(mut relat_ins), Ok(mut envl_ins)) => {
												let mut result = WriteResult::Ok(0);
												for record in records{
													if let Err(error) = acct_ins.execute(named_params!{":ssn": record.ssn}){
														if let rusqlite::Error::SqliteFailure(SqliteError{code: _, extended_code}, _) = error {
															if extended_code == 2067 || extended_code == 1555{
																dups += 1;
															} else{
																result = WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at acct_ins".into()));
																break
															}
														} else {
															result = WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at acct_ins".into()));
															break
														}
													}

													if let Err(error) = relat_ins.execute(named_params!{":ssn": record.ssn, ":id": id}){
														if let rusqlite::Error::SqliteFailure(SqliteError{code: _, extended_code}, _) = error {
															if extended_code == 2067 || extended_code == 1555{
																dups += 1;
															} else{
																result = WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at relat_ins".into()));
																break
															}
														} else {
															result = WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at relat_ins".into()));
															break
														}
													}
													let (spouse_fname, spouse_mname, spouse_lname, spouse_email) = match record.spouse {
														None => (None, None, None, None),
														Some(spouse) => (Some(spouse.first_name), spouse.middle_name, Some(spouse.last_name), Some(spouse.email))
													};

													if let Err(error) = envl_ins.execute(named_params!{
														":ssn": record.ssn,
														":fname": record.first_name,
														":mname": record.middle_name,
														":lname": record.last_name,
														":dob": record.dob,
														":addr1": record.addr1,
														":addr2": record.addr2,
														":city": record.city,
														":state": record.state,
														":zip": record.zip,
														":email": record.email,
														":phone": record.phone,
														":spouse_fname": spouse_fname,
														":spouse_mname": spouse_mname,
														":spouse_lname": spouse_lname,
														":spouse_email": spouse_email
													}){
														if let rusqlite::Error::SqliteFailure(SqliteError{code: _, extended_code}, _) = error {
															if extended_code == 2067 || extended_code == 1555{
																dups += 1;
															} else{
																result = WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at envl_ins".into()));
																break
															}
														} else {
															result = WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at envl_ins".into()));
															break
														}
													}
												}
												if let Ok(_) = result {
													result = WriteResult::Ok(dups);
												}
												result
											}
											_ => {
												WriteResult::Err(WFail::DbError("Unable to prepare statements for batch insert loop".into()))
												
											}
										}
									}
								}
							}
						}
					}

					WriteAction::UpdateAcct(acct) => match tran.prepare_cached("UPDATE acct_data SET primary_acct = :acct_num, info_codes = :info, created_acct = :new_acct, host_err = :err WHERE ssn = :ssn") {
						Ok(mut stmt) => match stmt.execute(named_params!{":acct_num": acct.primary_acct, ":info": acct.info_codes, ":new_acct": acct.created_acct, ":err": acct.host_err, ":ssn": acct.ssn}) {
							Ok(updates) => if updates == 1 {WriteResult::Ok(0)} else {WriteResult::Err(WFail::NoRecord)},
							Err(_) => WriteResult::Err(WFail::DbError("Unable to execute UpdateAcct stmt".into()))
						},
						Err(_) => WriteResult::Err(WFail::DbError("Unable to prepare UpdateAcct stmt".into()))
					},

					WriteAction::UpdateStatus { gid, status } => match tran.prepare_cached("UPDATE envelopes SET status = :status WHERE gid = :gid"){
						Ok(mut stmt) => match stmt.execute(named_params!{":gid": gid, ":status": status}){
							Ok(num) => if num == 1 {WriteResult::Ok(0)} else {WriteResult::Err(WFail::DbError("record mismatch".into()))},
							Err(_) => WriteResult::Err(WFail::DbError("Unable to update status".into()))
						},
						Err(_) => WriteResult::Err(WFail::DbError("Unable to prepare status update".into()))
					},

					WriteAction::SetGid { id, gid, status, api_err } => WriteResult::Err(WFail::DbError("this query is not yet prepared".into())),

					WriteAction::CompleteEnvelope { gid, status, beneficiaries, authorized_users } => WriteResult::Err(WFail::DbError("this query is not yet prepared".into()))
				} {
					Ok(_) => match tran.commit() {
						Ok(_) => {tx.send(WriteResult::Ok(dups));},
						Err(_) => {tx.send(WriteResult::Err(WFail::DbError("unable to commit write transaction".into())));}
					},
					Err(val) => {tx.send(WriteResult::Err(val));}

				}
			}

		}
	}
}

fn database_reader(rx: crossbeam_channel::Receiver<(ReadAction, oneshot::Sender<ReadResult>)>) {
	let conn = Connection::open_with_flags(
		"db.sqlite3",
		OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
	)
	.unwrap();
	conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)
		.unwrap();
}

pub enum WriteAction {
	NewBatch {
		batch_name: String,
		description: String,
		records: Vec<CsvImport>,
	},

	UpdateAcct(Acct),

	UpdateStatus {
		gid: String,
		status: String,
	},

	SetGid {
		id: i64,
		gid: String,
		status: Option<String>,
		api_err: Option<String>
	},

	CompleteEnvelope {
		gid: String,
		status: String,
		beneficiaries: Vec<Beneficiary>,
		authorized_users: Vec<AuthorizedUser>
	}
}

struct Beneficiary {
	kind: BeneficiaryType,
	name: String,
	address: String,
	city_state_zip: String,
	dob: String,
	relationship: String,
	ssn: i32,
	percent: i32
}
enum BeneficiaryType {
	Primary,
	Contingent,
}

impl ToString for BeneficiaryType {
	fn to_string(&self) -> String {
		match self {
			Primary => "primary".into(),
			Contingent => "contingent".into()
		}
	}
}

struct AuthorizedUser {
	name: String,
	dob: String
}

pub type WriteResult = Result<i32, WFail>;
//#[derive(Debug)]
//pub struct WSuccess {}
#[derive(Debug)]
pub enum WFail {
	Duplicate,
	NoRecord,
	DbError(String),
}

pub enum ReadAction {
	ActiveBatches,
	BatchDetail,
	OldBatches { page: u32 },
}

pub type ReadResult = Result<RSuccess, RFail>;
pub enum RSuccess {
	ActiveBatches(Vec<BatchSummary>),
	BatchDetail(Vec<(CsvImport, Acct)>),
	OldBatches(Vec<BatchSummary>),
}
pub enum RFail {
	Err,
}

#[derive(Clone)]
pub struct CsvImport {
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
	pub spouse: Option<CsvSpouse>,
}
#[derive(Clone)]
pub struct CsvSpouse {
	pub first_name: String,
	pub middle_name: Option<String>,
	pub last_name: String,
	pub email: String,
}

pub struct Acct {
	pub ssn: u32,
	pub primary_acct: Option<u32>,
	pub info_codes: Option<String>,
	pub created_acct: Option<u32>,
	pub host_err: Option<String>,
}

pub struct BatchSummary {
	pub id: i64,
	pub name: String,
	pub description: String,
	pub total: u32,
	pub working: u32,
	pub complete: u32,
	pub err: u32,
}
