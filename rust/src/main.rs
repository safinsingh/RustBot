use dotenv::dotenv;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use serenity::{
	async_trait,
	client::{Client, Context, EventHandler},
	http::AttachmentType,
	model::{
		channel::Message, event::MessageUpdateEvent, id::MessageId,
		prelude::Ready,
	},
};
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::Mutex;

lazy_static! {
	static ref REGEX: Regex =
		Regex::new("\\?(eval|play)\\s+```rust\\n([\\s\\S]*?)\\n+```")
			.unwrap();
	static ref REQWEST_CLIENT: reqwest::Client = ReqwestClient::new();
	static ref RESPONSE_MAP: Arc<Mutex<HashMap<MessageId, Message>>> =
		Arc::new(Mutex::new(HashMap::new()));
}

const ENDPOINT: &str = "https://play.integer32.com/execute";
const HELP: &str = r#"```RustBot v0.1.0

USAGE:
    ?help | ?eval | ?play { rust codeblock }

COMMANDS:
    ?help - display this help command
    ?eval - evaluate the code and Debug the result
    ?play - execute code and send stdout/stderr (equivalent to local run)
```"#;

#[derive(Deserialize, Debug)]
struct ApiResponse {
	stdout: String,
	stderr: String,
	success: bool,
}

#[derive(Serialize)]
struct ApiRequest<'a, S>
where
	S: Into<String>,
{
	channel: &'a str,
	mode: &'a str,
	edition: &'a str,
	#[serde(rename = "crateType")]
	crate_type: &'a str,
	tests: bool,
	code: S,
	backtrace: bool,
}

impl<'a, S: Into<String>> ApiRequest<'a, S> {
	fn new(code: S) -> ApiRequest<'a, S> {
		Self {
			channel: "stable",
			mode: "debug",
			edition: "2018",
			crate_type: "bin",
			tests: false,
			code,
			backtrace: false,
		}
	}
}

async fn query_playground<'a, S>(code: S) -> String
where
	S: Into<String> + Serialize,
{
	let body = ApiRequest::new(code);

	// lol
	let res = REQWEST_CLIENT
		.post(ENDPOINT)
		.body(serde_json::to_string(&body).unwrap())
		.header("Content-Type", "application/json")
		.send()
		.await
		.unwrap()
		.json::<ApiResponse>()
		.await
		.unwrap();

	if res.success {
		res.stdout
	} else {
		res.stderr
	}
}

async fn extract_message_output<'a>(
	matches: &Captures<'a>,
) -> String {
	match &matches[1] {
		"eval" => {
			query_playground(format!(
				"fn main() {{ println!(\"{{:?}}\", {{ {} }}) }}",
				&matches[2]
			))
			.await
		}
		"play" => query_playground(&matches[2]).await,
		_ => unreachable!(),
	}
}

async fn process_message<'a>(
	matches: &Option<Captures<'a>>,
	ctx: &Context,
	sent: &mut Message,
) {
	let body = (&matches).as_ref().unwrap();
	let output = extract_message_output(body).await;

	match output.len() {
		0..=1999 => {
			let _ = sent
				.edit(&ctx.http, |m| {
					m.content(format!("```\n{}```", output))
				})
				.await;
		}
		2000..=7999999 => {
			let _ = sent
				.channel_id
				.send_files(
					&ctx.http,
					vec![AttachmentType::from((
						output.as_bytes(),
						format!("Result-{}.txt", sent.id).as_str(),
					))],
					|m| m,
				)
				.await;
		}
		_ => {
			let _ = sent
				.edit(&ctx.http, |m| {
					m.content(
						"Response exceeded 8MB limit, please \
						 manually evaluate!",
					)
				})
				.await;
		}
	}
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
	async fn message(&self, ctx: Context, msg: Message) {
		if msg.content.as_str() == "?help" {
			let _ = msg.channel_id.say(&ctx.http, HELP).await;
			return;
		}

		let matches = REGEX.captures(&msg.content);
		if matches.is_none() {
			return;
		}

		let mut sent = msg
			.channel_id
			.say(&ctx.http, "loading...")
			.await
			.unwrap();

		process_message(&matches, &ctx, &mut sent).await;

		let mut map = RESPONSE_MAP.lock().await;
		map.insert(msg.id, sent);
	}

	async fn message_update(
		&self,
		ctx: Context,
		_old: Option<Message>,
		_new: Option<Message>,
		event: MessageUpdateEvent,
	) {
		let mut bot_response = RESPONSE_MAP.lock().await;
		let bot_message = bot_response.get_mut(&event.id);
		if bot_message.is_none() {
			return;
		}
		println!("thing.");

		let content = event.content.unwrap();
		let matches = REGEX.captures(&content);
		if matches.is_some() {
			return;
		}

		let bot_message = bot_message.unwrap();
		let _ = bot_message
			.edit(&ctx.http, |m| m.content("loading..."))
			.await;

		process_message(&matches, &ctx, bot_message).await;
	}

	async fn ready(&self, _ctx: Context, data_about_bot: Ready) {
		println!("Logged in as {}!", data_about_bot.user.tag());
	}
}

#[tokio::main]
async fn main() {
	dotenv().ok();

	let token = env::var("TOKEN").expect("token");
	let mut client = Client::builder(token)
		.event_handler(Handler)
		.await
		.expect("Error creating client");

	if let Err(why) = client.start().await {
		println!(
			"An error occurred while running the client: {:?}",
			why
		);
	}
}
