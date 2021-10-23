use async_channel;
use tokio::task;
use crate::CsvImport;
use crate::Config;
use serde::Serialize;
use reqwest::Client;
use crate::oauth::AuthHelper;

pub fn init(concurrent_workers: usize, config: &Config, token: AuthHelper) -> async_channel::Sender<CsvImport> {
	let (tx, rx) = async_channel::bounded(concurrent_workers * 10);
	let http_client = Client::new();
	for n in 0..concurrent_workers {
		let rx = rx.clone();
		let http_client = http_client.clone();
	}
	tx
}

async fn worker(rx: async_channel::Receiver<){

}

impl Envelope {
	fn from_csv(csv: &CsvImport, config: &Config) -> Self {
		let csv = csv.clone();
		let mut env = Self {
			template_id: config.docusign.templateId.clone(),
			template_roles: vec!(EnvelopeRecipient {
				name: format!("{} {}{}", csv.first_name, match csv.middle_name {
						Some(val) => val + " ", None => "".into()
					},
					csv.last_name),

				email: csv.email,
				role_name: "Signer".into(),

				tabs: Some(Tabs {
					text_tabs: [
						TabValue {
							tab_label: "SSN".into(),
							value: csv.ssn.to_string()
						},
						TabValue {
							tab_label: "DOB".into(),
							value: csv.dob
						},
						TabValue {
							tab_label: "address".into(),
							value: format!("{}{}", csv.addr1, match csv.addr2{
								Some(val) => " ".to_string() + &val,
								None => "".into()
							})
						},
						TabValue{
							tab_label: "cit-st-zip".into(),
							value: format!("{}, {} {}", csv.city, csv.state, csv.zip)
						},
						TabValue{
							tab_label: "phone".into(),
							value: csv.phone
						}
					]
				})
			}),
			status: "Sent".into()
		};

		if let Some(spouse) = csv.spouse {
			env.template_roles.push(EnvelopeRecipient{
				role_name: "Spouse".into(),

				name: format!("{} {}{}", spouse.first_name, match spouse.middle_name{
						Some(val) => val + " ",
						None => "".into(),
					},
					spouse.last_name),

				email: spouse.email,
				tabs: None
			})
		}

		env
	}
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
	template_id: String,
	template_roles: Vec<EnvelopeRecipient>,
	status: String

}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvelopeRecipient {
	name: String,
	email: String,
	role_name: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	tabs: Option<Tabs>
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tabs {
	text_tabs: [TabValue; 5]
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TabValue {
	tab_label: String,
	value: String
}