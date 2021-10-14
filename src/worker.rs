use async_channel;
use tokio::task;
use crate::CsvImport;
use crate::Config;
use serde::Serialize;
use serde_json;
use reqwest::Client;

pub fn init(concurrent_workers: usize) -> async_channel::Sender<CsvImport> {
	let (tx, rx) = async_channel::bounded(concurrent_workers * 10);
	let http_client = Client::new();
	for n in 0..concurrent_workers {
		let rx = rx.clone();
		let http_client = http_client.clone();
		task::spawn( async {
			worker(rx, http_client).await;
		}
		);
	}
	tx
}

async fn worker(rx: async_channel::Receiver<CsvImport>, http_client: Client) {

}

impl Envelope {
	fn from_csv(csv: &CsvImport, config: &Config) -> Self {
		Self {
			templateId: config.docusign.templateId.clone(),
			templateRoles: vec!(EnvelopeRecipient {
				name: csv.first_name + match csv.middle_name {
					Some(val) => " ".into() + val,
					None => "".into()
				}
				+ &csv.last_name,
				email: csv.email,
				roleName: "Signer".into(),
				tabs: Some(Tabs {
					textTabs: [
						TabValue {
							tabLabel: "SSN".into(),
							value: csv.ssn.to_string()
						},
						TabValue {
							tabLabel: "DOB".into(),
							value: csv.dob
						},
						TabValue {
							tabLabel: "address".into(),
							value: csv.addr1 + match csv.addr2 {
								Some(val)=> " ".into() + val,
								None => ""
							}
						},
						TabValue{
							tabLabel: "cit-st-zip"
							value:
						},
						TabValue{

						}
					]
				})
			}),
			status: "Sent".into()
		}
	}
}

#[derive(Serialize)]
struct Envelope {
	templateId: String,
	templateRoles: Vec<EnvelopeRecipient>,
	status: String

}
#[derive(Serialize)]
struct EnvelopeRecipient {
	name: String,
	email: String,
	roleName: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	tabs: Option<Tabs>
}
#[derive(Serialize)]
struct Tabs {
	textTabs: [TabValue; 5]
}

#[derive(Serialize)]
struct TabValue {
	tabLabel: String,
	value: String
}