pub mod structs {
	use serde::{Deserialize, Serialize};
	#[derive(Deserialize, Serialize)]
	pub struct NewBatchData {
		pub name: String,
		pub description: String,
		pub records: Vec<NewEnvelopes>,
	}
	#[derive(Clone, Deserialize, Serialize)]
	pub struct NewEnvelopes {
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
	#[derive(Clone, Deserialize, Serialize)]
	pub struct Spouse {
		pub first_name: String,
		pub middle_name: Option<String>,
		pub last_name: String,
		pub email: String,
	}
	#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
	pub struct BatchDetail {
		pub ssn: u32,
		pub primary_acct: Option<u32>,
		pub created_account: Option<u32>,
		pub fname: String,
		pub mname: Option<String>,
		pub lname: String,
		pub info_codes: Option<String>,
		pub host_err: Option<String>,
		pub status: Option<String>,
		pub void_reason: Option<String>,
		pub api_err: Option<String>,
		pub ignore_error: bool,
	}
	#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
	pub struct EnvelopeDetail {
		pub id: i64,
		pub gid: Option<String>,
		pub status: Option<String>,
		pub void_reason: Option<String>,
		pub host_api_err: Option<String>,
		pub docusign_api_err: Option<String>,
		pub info_codes: Option<String>,
		pub primary_account: Option<u32>,
		pub created_account: Option<u32>,
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
		pub date_created: i64,
		pub is_married: Option<bool>,
		pub beneficiaries: Vec<Beneficiary>,
		pub auth_users: Vec<AuthorizedUser>,
	}
	#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
	pub struct BatchSummary {
		pub id: i64,
		pub name: String,
		pub description: String,
		pub start_date: u32,
		pub end_date: Option<u32>,
		pub total: u32,
		pub working: u32,
		pub complete: u32,
		pub err: u32,
	}
	#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
	pub struct Beneficiary {
		pub kind: BeneficiaryType,
		pub name: String,
		pub address: String,
		pub city_state_zip: String,
		pub dob: String,
		pub relationship: String,
		pub ssn: u32,
		pub percent: u32,
	}

	#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
	pub enum BeneficiaryType {
		Primary,
		Contingent,
	}
	#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
	pub struct AuthorizedUser {
		pub name: String,
		pub dob: String,
	}
	impl ToString for BeneficiaryType {
		fn to_string(&self) -> String {
			match self {
				BeneficiaryType::Primary => "primary".into(),
				BeneficiaryType::Contingent => "contingent".into(),
			}
		}
	}

	impl rusqlite::types::FromSql for BeneficiaryType {
		fn column_result(
			value: rusqlite::types::ValueRef<'_>,
		) -> std::result::Result<Self, rusqlite::types::FromSqlError> {
			match value.as_str()? {
				"contingent" => Ok(Self::Contingent),
				"primary" => Ok(Self::Primary),
				_ => Err(rusqlite::types::FromSqlError::InvalidType),
			}
		}
	}
}
