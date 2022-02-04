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
												//wrap loop inside closure so we can return early
												let closure = ||{
													let mut dups = 0;

													for record in records{
														if let Err(error) = acct_ins.execute(named_params!{":ssn": record.ssn}){
															if let rusqlite::Error::SqliteFailure(SqliteError{code: _, extended_code}, _) = error {
																if extended_code == 2067 || extended_code == 1555{
																	dups += 1;
																} else{
																	return WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at acct_ins".into()));
																}
															} else {
																return WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at acct_ins".into()));
															
															}
														}

														if let Err(error) = relat_ins.execute(named_params!{":ssn": record.ssn, ":id": id}){
															if let rusqlite::Error::SqliteFailure(SqliteError{code: _, extended_code}, _) = error {
																if extended_code == 2067 || extended_code == 1555{
																	dups += 1;
																} else{
																	return WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at relat_ins".into()));
																	
																}
															} else {
																return WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at relat_ins".into()));
																
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
																	return WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at envl_ins".into()));
																	
																}
															} else {
																return WriteResult::Err(WFail::DbError("{batch_name} Failed during new batch insert loop at envl_ins".into()));
																
															}
														}
													}
													//return ok if all loops complete
													WriteResult::Ok(dups)
												};
												closure()
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

					WriteAction::UpdateStatus { gid, status, void_reason, api_err } => match tran.prepare_cached(
							"UPDATE envelopes SET status = :status, void_reason = :void_reason, api_err = :api_err WHERE gid = :gid"){
						Ok(mut stmt) => match stmt.execute(named_params!{":gid": gid, ":status": status, ":void_reason": void_reason, ":api_err": api_err}){
							Ok(num) => if num == 1 {WriteResult::Ok(0)} else {WriteResult::Err(WFail::NoRecord)},
							Err(_) => WriteResult::Err(WFail::DbError("Unable to update status".into()))
						},
						Err(_) => WriteResult::Err(WFail::DbError("Unable to prepare status update".into()))
					},

					WriteAction::SetGid { id, gid, status, api_err, void_reason } => match tran.prepare_cached(
							"UPDATE envelopes SET gid = :gid, status = :status, api_err = :api_err, void_reason = :void_reason WHERE id = :id") {
						Err(_) => WriteResult::Err(WFail::DbError("Unable to prepare set gid query".into())),
						Ok(mut stmt) => match stmt.execute(named_params!{":id": id, ":gid": gid, ":status": status, ":api_err": api_err, ":void_reason": void_reason}) {
							Err(_) => WriteResult::Err(WFail::DbError("Unable to set gid".into())),
							Ok(num) => if num == 1 {WriteResult::Ok(0)} else {WriteResult::Err(WFail::NoRecord)}
						}
					}

					WriteAction::CompleteEnvelope { gid, status, beneficiaries, authorized_users, pdf } => match tran.prepare_cached("UPDATE envelopes SET status = :status, pdf = :pdf WHERE gid = :gid") {
						Err(_) => WriteResult::Err(WFail::DbError("Unable to prepare set status query".into())),
						Ok(mut stmt) => match stmt.execute(named_params!{":status": status, ":gid": gid, ":pdf": pdf}) {
							Ok(num) => match num {
								1 => {
									match (
										tran.prepare_cached("INSERT INTO beneficiaries (gid, type, name, address, city_state_zip, dob, relationship, ssn, percent)
											VALUES (:gid, :type, :name, :address, :city_state_zip, :dob, :relationship, :ssn, :percent"),
										tran.prepare_cached("INSERT INTO authorized_users (gid, name, dob) VALUES (:gid, :name, :dob)")
									) {
										(Ok(mut benef_insert), Ok(mut auth_insert)) => {
											//wrap loop inside closure so we can return early
											let closure = || {
												for benef in beneficiaries {
													if let Err(_) = benef_insert.execute(named_params!{
															":gid": gid,
															":type": benef.kind.to_string(),
															":name": benef.name,
															":address": benef.address,
															":city_state_zip": benef.city_state_zip,
															":dob": benef.dob,
															":relationship": benef.relationship,
															":ssn": benef.ssn,
															":percent": benef.percent
														}) {
														//Return out of loop early with failure if we encounter an error
														return WriteResult::Err(WFail::DbError("Failed at benef insert loop".into()))
													}
												}

												for user in authorized_users {
													if let Err(_) = auth_insert.execute(named_params!{":gid": gid, ":name": user.name, ":dob": user.dob}){
														return WriteResult::Err(WFail::DbError("Failed at authorized user insert loop".into()))
													}
												}
												//return ok if all loops complete
												WriteResult::Ok(0)
											};

											closure()
										},
										_ => WriteResult::Err(WFail::DbError("Unable to prepare benef and auth queries".into()))
									}
								}
								_=> WriteResult::Err(WFail::NoRecord)
							}
							Err(_) => WriteResult::Err(WFail::DbError("Unable to update completed status".into())),


						}
					}
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
		let _res = tx.send(match event {
			ReadAction::ActiveBatches => {
				match conn.prepare_cached("
					SELECT batch.id, batch.batch_name, batch.description, batch.start_date, batch.end_date,
					count(*),
					count(status NOT IN ('completed', 'declined', 'voided', 'cancelled') AND status IS NOT NULL),
					count(created_account AND (status = 'completed' or ignore_error)),
					count(host_err OR api_err OR void_reason OR status IN ('declined', 'voided', 'cancelled'))
					FROM company_batches batch
					INNER JOIN ssn_batch_relat relat
					ON batch.id = relat.batch_id
					INNER JOIN acct_data acct
					ON relat.ssn = acct.ssn
					INNER JOIN (
						SELECT ssn, status, void_reason, api_err, ROW_NUMBER() OVER(PARTITION BY ssn ORDER BY date_created) rn
						FROM envelopes
					) last_env
					ON acct.ssn = last_env.ssn
					WHERE last_env.rn = 1
					AND (batch.end_date IS NULL OR batch.end_date > strftime('%s', 'now') - (60*60*24*7))
					GROUP BY batch.id, batch.batch_name, batch.description
					ORDER BY batch.end_date DESC NULLS FIRST, batch.id DESC
				") {
					Err(_) => ReadResult::Err("Unable to prepare active batches query".into()),
					Ok(mut active_stmt) => {
						match active_stmt.query([]) {
							Err(_) => ReadResult::Err("unable to execute active batches stmt".into()),
							Ok(mut rows) => {
								let mut active_closure = || {
									let mut batches = vec!();
									loop {
										match rows.next() {
											Err(_) => {return ReadResult::Err("Error during active batches read".into());},
											Ok(None) => {break;}
											Ok(Some(row)) => {
												match (row.get(0), row.get(1), row.get(2), row.get(3),
													row.get(4), row.get(5), row.get(6), row.get(7), row.get(8)) {
														(Ok(id), Ok(name), Ok(description), Ok(start_date), Ok(end_date), Ok(total), Ok(working), Ok(complete), Ok(err)) => {
															batches.push(BatchSummary{
																id, name, description, start_date, end_date, total, working, complete, err
															});

														}
														_ => return ReadResult::Err("Error during read loop".into())
													}
											}
										}
									}

									ReadResult::Ok(RSuccess::ActiveBatches(batches))
								};

								active_closure()
							}


						}
					}
				}
			},
			ReadAction::BatchDetail{rowid } => {
				match conn.prepare_cached("
					SELECT acct_data.ssn, primary_acct, created_account, fname, mname, lname, info_codes, host_err, status, void_reason, api_err, ignore_error
					FROM company_batches
					INNER JOIN ssn_batch_relat
					ON company_batches.id = ssn_batch_relat.batch_id
					INNER JOIN acct_data
					ON acct_data.ssn = ssn_batch_relat.ssn
					INNER JOIN (
						SELECT ssn, status, void_reason, api_err, fname, mname, lname, ROW_NUMBER() OVER(PARTITION BY ssn ORDER BY date_created) rn
						FROM envelopes
					) last_env
					ON acct_data.ssn = last_env.ssn
					WHERE last_env.rn = 1
					AND company_batches.id = :id
					ORDER BY CASE WHEN (
						primary_acct IS NULL
						OR info_codes IS NOT NULL
						OR host_err IS NOT NULL
						OR status IN ('declined', 'voided', 'cancelled')
					) THEN 0 ELSE 1 END ASC, acct_data.ssn ASC
				") {
					Err(_) => ReadResult::Err("unable to prepare batch detail query".into()),
					Ok(mut stmt) => match stmt.query(named_params!{":id": rowid}) {
						Err(_) => ReadResult::Err("unable to execute batch detail query".into()),
						Ok(mut rows) => {
							let mut closure = || {
								let mut details = vec!();
								loop {
									match rows.next() {
										Err(_) => {return ReadResult::Err("Error during active batches read".into());}
										Ok(None) => {break;},
										Ok(Some(row)) => match (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4), row.get(5), row.get(6), row.get(7), row.get(8), row.get(9), row.get(10), row.get(11)) {
											(Ok(ssn), Ok(primary_acct), Ok(created_account), Ok(fname), Ok(mname), Ok(lname), Ok(info_codes), Ok(host_err), Ok(status), Ok(void_reason), Ok(api_err), Ok(ignore_error)) => {
												details.push(BatchDetail{ssn, primary_acct, created_account, fname, mname, lname, info_codes, host_err, status, void_reason, api_err, ignore_error})
											},
											_ => return ReadResult::Err("Error during read loop".into())
										}
									}
								}
								ReadResult::Ok(RSuccess::BatchDetails(details))
							};

							closure()
						}
					}
				}
			},
			ReadAction::EnvelopeDetail{ssn} => {
				match conn.transaction() {
					Err(_) => ReadResult::Err("Unable to begin transaction for envelope details".into()),
					Ok(tran) => match (
						tran.prepare_cached("
							SELECT id, gid, status, void_reason, api_err, fname, mname, lname, dob,
								addr1, addr2, city, state, zip, email, phone, spouse_fname, spouse_mname, spouse_lname, spouse_email, date_created
							FROM envelopes
							WHERE ssn = :ssn
							ORDER BY date_created DESC
						"),
						tran.prepare_cached("
							SELECT type, name, address, city_state_zip, dob, relationship, ssn, percent
							FROM beneficiaries
							WHERE gid = :gid
						"),
						tran.prepare_cached("
							SELECT name, dob
							FROM authorized_users
							WHERE gid = :gid
						")
					) {
						(Err(_), _, _) | (_, Err(_), _) | (_,_, Err(_))=> ReadResult::Err("Unable to prepare Queries".into()),
						(Ok(mut env_stmt), Ok(mut benef_stmt), Ok(mut auth_stmt))=> {
							match env_stmt.query(named_params!{":ssn": ssn}) {
								Err(_) => ReadResult::Err("Unable to execute env stmt".into()),
								Ok( mut rows) => {
									let mut closure = ||{
										let mut envelopes = vec!();
										loop {
											match rows.next() {
												Err(_) => {return ReadResult::Err("Error during envelope detail read".into());}
												Ok(None) => break,
												Ok(Some(row)) => {
													match (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4), row.get(5),
														row.get(6), row.get(7), row.get(8), row.get(9), row.get(10), row.get(11),
														row.get(12), row.get(13), row.get(14), row.get(15), row.get(16), row.get(17),
														row.get(18), row.get(19), row.get(20)) {
															(Ok(id), Ok(gid), Ok(status), Ok(void_reason), Ok(api_err), Ok(first_name),Ok(middle_name), Ok(last_name), Ok(dob),
															Ok(addr1), Ok(addr2), Ok(city), Ok(state), Ok(zip), Ok(email), Ok(phone),
															Ok(spouse_fname), Ok(spouse_mname), Ok(spouse_lname), Ok(spouse_email), Ok(date_created)) => {
																let mut envelope = EnvelopeDetail {
																	id, gid, ssn, status, void_reason, api_err, first_name, middle_name, last_name, dob,
																	addr1, addr2, city, state, zip, email, phone,
																	spouse_fname, spouse_mname, spouse_lname, spouse_email, date_created,
																	beneficiaries: vec!(), auth_users: vec!()
																};
																match benef_stmt.query(named_params!{":gid": envelope.gid}){
																	Err(_) => {return ReadResult::Err("Error during beneficiary execution".into());}
																	Ok(mut rows) => {
																		loop {
																			match rows.next() {
																				Err(_) => return ReadResult::Err("Error during beneficiary read".into()),
																				Ok(None) => break,
																				Ok(Some(row)) => {
																					match (row.get(0), row.get(1), row.get(2), row.get(3),
																					row.get(4), row.get(5), row.get(6), row.get(7)) {
																						(Ok(kind), Ok(name), Ok(address), Ok(city_state_zip),
																						Ok(dob), Ok(relationship), Ok(ssn), Ok(percent)) => {
																							envelope.beneficiaries.push(Beneficiary {
																								kind, name, address, city_state_zip, dob,
																								relationship, ssn, percent
																							})
																						}
																						_=> return ReadResult::Err("Error during beneficiary execution".into())
																					}
																				}
																			}
																		}
																	}
																}
																match auth_stmt.query(named_params!{":gid": envelope.gid}){
																	Err(_) => {return ReadResult::Err("Error during beneficiary execution".into());}
																	Ok(mut rows) => {
																		loop {
																			match rows.next() {
																				Err(_) => return ReadResult::Err("Error during beneficiary read".into()),
																				Ok(None) => break,
																				Ok(Some(row)) => {
																					match (row.get(0), row.get(1)) {
																						(Ok(name), Ok(dob)) => {
																							envelope.auth_users.push(
																								AuthorizedUser{
																									name, dob
																								}
																							)
																						}
																						_=> return ReadResult::Err("Error during beneficiary execution".into())
																					}
																				}
																			}
																		}
																	}
																}
																envelopes.push(envelope);
															}

															_=> {return ReadResult::Err("Error during envelope detail loop".into());}
														}
												}
											}
										}
										ReadResult::Ok(RSuccess::EnvelopeDetails(envelopes))
									};
									closure()
								}
							}
						}
					}
				}
			}

			ReadAction::FetchPdf => ReadResult::Err("placeholder".into()),

			ReadAction::NewEnvelopes => ReadResult::Err("placeholder".into())

		});
	}
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
		void_reason: Option<String>,
		api_err: Option<String>
	},

	SetGid {
		id: i64,
		gid: String,
		status: Option<String>,
		void_reason: Option<String>,
		api_err: Option<String>
	},

	CompleteEnvelope {
		gid: String,
		status: String,
		beneficiaries: Vec<Beneficiary>,
		authorized_users: Vec<AuthorizedUser>,
		pdf: Vec<u8>
	}
}

pub struct Beneficiary {
	pub kind: BeneficiaryType,
	pub name: String,
	pub address: String,
	pub city_state_zip: String,
	pub dob: String,
	pub relationship: String,
	pub ssn: u32,
	pub percent: u32
}
pub enum BeneficiaryType {
	Primary,
	Contingent,
}

impl ToString for BeneficiaryType {
	fn to_string(&self) -> String {
		match self {
			BeneficiaryType::Primary => "primary".into(),
			BeneficiaryType::Contingent => "contingent".into()
		}
	}
}

impl rusqlite::types::FromSql for BeneficiaryType {

fn column_result( value: rusqlite::types::ValueRef<'_>) -> std::result::Result<Self, rusqlite::types::FromSqlError> {
	let val = value.as_str()?;
	//set type to contingent if specified, otherwise assume primary
	if val == "contingent" {
		return Ok(Self::Contingent)
	}
	if val == "primary" {
		return Ok(Self::Primary)
	}
	Err(rusqlite::types::FromSqlError::InvalidType)
}
}
pub struct AuthorizedUser {
	pub name: String,
	pub dob: String
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
	//rowid for batch detail
	BatchDetail{rowid: i64},
	//ssn for individual detail
	EnvelopeDetail{ssn:u32},
	FetchPdf,
	NewEnvelopes
}

pub type ReadResult = Result<RSuccess, String>;
pub enum RSuccess {
	ActiveBatches(Vec<BatchSummary>),
	BatchDetails(Vec<BatchDetail>),
	EnvelopeDetails(Vec<EnvelopeDetail>),
	OldBatches(Vec<BatchSummary>),
	PdfBlob(Vec<u8>)
}

pub struct BatchDetail {
	pub ssn: u32,
	pub primary_acct: u32,
	pub created_account: u32,
	pub fname: String,
	pub mname: Option<String>,
	pub lname: String,
	pub info_codes: String,
	pub host_err: String,
	pub status: String,
	pub void_reason: String,
	pub api_err: String,
	pub ignore_error: bool
}

pub struct EnvelopeDetail {
	pub id: i64,
	pub gid: Option<String>,
	pub status: Option<String>,
	pub void_reason: Option<String>,
	pub api_err: Option<String>,
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
	pub spouse_fname: Option<String>,
	pub spouse_mname: Option<String>,
	pub spouse_lname: Option<String>,
	pub spouse_email: Option<String>,
	pub date_created: String,
	pub beneficiaries: Vec<Beneficiary>,
	pub auth_users: Vec<AuthorizedUser>
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
	pub spouse: Option<Spouse>,
}
#[derive(Clone)]
pub struct Spouse {
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
	pub start_date: u32,
	pub end_date: u32,
	pub total: u32,
	pub working: u32,
	pub complete: u32,
	pub err: u32,
}
