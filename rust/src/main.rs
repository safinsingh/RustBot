use dotenv::dotenv;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::{
	async_trait,
	client::{Client, Context, EventHandler},
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

static HELP: &str = r#"```RustBot v0.1.0

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

fn message_valid<'a>(content: &'a str) -> Option<Captures<'a>> {
	if !REGEX.is_match(content) {
		return None;
	}

	let matches = REGEX.captures(content);
	Some(matches.unwrap())
}

async fn query_playground<'a, S>(code: S) -> String
where
	S: Into<String> + Serialize,
{
	static ENDPOINT: &str = "https://play.integer32.com/execute";
	let body = json!({
		"channel": "stable",
		"mode": "debug",
		"edition": "2018",
		"crateType": "bin",
		"tests": false,
		"code": code,
		"backtrace": false,
	});

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
		return res.stdout;
	}
	res.stderr
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

	if output.len() <= 500 {
		let _ = sent
			.edit(&ctx.http, |m| {
				m.content(format!("```\n{}```", output))
			})
			.await;
	} else {
		let _ = sent
			.edit(&ctx.http, |m| {
				m.content("response too long, manually evaluate!")
			})
			.await;
	}
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
	async fn message(&self, ctx: Context, new_message: Message) {
		if new_message.content.as_str() == "?help" {
			let _ = new_message
				.channel_id
				.send_message(&ctx.http, |m| m.content(HELP))
				.await;
			return;
		}

		let matches = message_valid(&new_message.content);
		if matches.is_none() {
			return;
		}

		let mut sent = new_message
			.channel_id
			.send_message(&ctx.http, |m| m.content("loading..."))
			.await
			.unwrap();

		process_message(&matches, &ctx, &mut sent).await;

		let mut map = RESPONSE_MAP.lock().await;
		map.insert(new_message.id, sent);
	}

	async fn message_update(
		&self,
		ctx: Context,
		_old_if_available: Option<Message>,
		_new: Option<Message>,
		event: MessageUpdateEvent,
	) {
		let content = event.content.unwrap().clone();
		let matches = message_valid(&content);
		if matches.is_some() {
			return;
		}

		let mut bot_response = RESPONSE_MAP.lock().await;
		let bot_message = bot_response.get_mut(&event.id).unwrap();

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
