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
		channel::Message,
		event::MessageUpdateEvent,
		id::{ChannelId, MessageId},
		prelude::Ready,
	},
};
use std::{collections::HashMap, env, sync::Arc, time::Duration};
use tokio::sync::Mutex;

lazy_static! {
	static ref REGEX: Regex =
		Regex::new("\\?(eval|play)\\s+```rust\\n([\\s\\S]*?)\\n+```")
			.unwrap();
	static ref REQWEST_CLIENT: reqwest::Client =
		ReqwestClient::builder()
			.timeout(Duration::from_secs(10))
			.build()
			.unwrap();
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
		.await;
	let res = match res {
		Ok(r) => r.json::<ApiResponse>().await.unwrap(),
		Err(e) if e.is_timeout() => ApiResponse {
			stdout: "".to_string(),
			stderr: "Request exceeded timeout (>10s)".to_string(),
			success: false,
		},
		Err(e) => panic!("{}", e),
	};

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

enum BotEvent<'a> {
	OnMessage,
	OnEdit(&'a mut Message),
}

trait MessageCtx {
	fn get_channel_id(&self) -> ChannelId;

	fn get_id(&self) -> MessageId;
}

macro_rules! impl_msg_ctx {
	($($m:ty),+) => {
		$(
			impl MessageCtx for $m {
				fn get_channel_id(&self) -> ChannelId { self.channel_id }

				fn get_id(&self) -> MessageId { self.id }
			}
		)+
	};
}

impl_msg_ctx!(Message, MessageUpdateEvent);

async fn process_message<'a, M>(
	matches: &Option<Captures<'a>>,
	ctx: &Context,
	query: &M,
	evt: BotEvent<'a>,
) -> Option<Message>
where
	M: MessageCtx,
{
	let body = matches.as_ref().unwrap();
	let output = extract_message_output(body).await;

	match output.len() {
		0..=1999 => match evt {
			BotEvent::OnMessage => Some(
				query
					.get_channel_id()
					.say(&ctx.http, format!("```\n{}```", output))
					.await
					.unwrap(),
			),
			BotEvent::OnEdit(old) => {
				old.edit(&ctx.http, |m| {
					m.content(format!("```\n{}```", output))
				})
				.await
				.unwrap();
				None
			}
		},
		2000..=7999999 => Some(
			query
				.get_channel_id()
				.send_files(
					&ctx.http,
					vec![AttachmentType::from((
						output.as_bytes(),
						format!("Result-{}.txt", query.get_id())
							.as_str(),
					))],
					|m| m,
				)
				.await
				.unwrap(),
		),
		_ => Some(
			query
				.get_channel_id()
				.say(
					&ctx.http,
					"Response exceeded 8MB limit, please manually \
					 evaluate!",
				)
				.await
				.unwrap(),
		),
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

		let typing = msg.channel_id.start_typing(&ctx.http).unwrap();
		let response = process_message(
			&matches,
			&ctx,
			&msg,
			BotEvent::OnMessage,
		)
		.await;

		typing.stop();

		let mut map = RESPONSE_MAP.lock().await;
		map.insert(msg.id, response.unwrap());
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

		let content = event.content.clone().unwrap();
		let matches = REGEX.captures(&content);
		if matches.is_none() {
			return;
		}

		let bot_message = bot_message.unwrap();

		let typing =
			event.channel_id.start_typing(&ctx.http).unwrap();

		process_message(
			&matches,
			&ctx,
			&event,
			BotEvent::OnEdit(bot_message),
		)
		.await;

		typing.stop();
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
