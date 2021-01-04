import Discord from 'discord.js'
import dotenv from 'dotenv'
import fetch from 'node-fetch'

const responseMap = new Map()

const client = new Discord.Client()
const HELP = `\`\`\`RustBot v0.1.0

USAGE:
    ?help | ?eval | ?play { rust codeblock }

COMMANDS:
    ?help - display this help command
    ?eval - evaluate the code and Debug the result
    ?play - execute code and send stdout/stderr (equivalent to local run)
\`\`\``

async function queryPlayground(messageString: string) {
	const data = {
		channel: 'stable',
		mode: 'debug',
		edition: '2018',
		crateType: 'bin',
		tests: false,
		code: messageString,
		backtrace: false,
	}
	const url = 'https://play.integer32.com/execute'

	const res = await fetch(url, {
		method: 'POST',
		headers: {
			'Content-Type': 'application/json',
		},
		body: JSON.stringify(data),
	}).then((res) => res.json())

	const codeWrap = (text: string) => `\`\`\`${text}\`\`\``

	if (res.success) return codeWrap(res.stdout)
	return codeWrap(res.stderr)
}

async function extractMessageOutput(match: RegExpMatchArray) {
	let messageString = ''
	switch (match[1]) {
		case 'eval':
			messageString = `fn main() { println!("{:?}", { ${match[2]} }) }`
			break
		case 'play':
			messageString = match[2]
	}

	const res = await queryPlayground(messageString)
	if (res.length <= 500) return res
	return 'response too long, manually evaluate!'
}

function messageValid(content: string) {
	const EVAL_REGEX = new RegExp('\\?(eval|play)\\s+```rust\\n([\\s\\S]*?)\\n+```')

	if (!EVAL_REGEX.test(content)) return { valid: false }
	return { valid: true, body: content.match(EVAL_REGEX) }
}

client.on('ready', () => {
	console.log(`Logged in as ${client.user?.tag}!`)
})

client.on('messageUpdate', async (oldMsg, newMsg) => {
	const correspondingMessage = responseMap.get(oldMsg.id)
	if (correspondingMessage) {
		const match = messageValid(newMsg.content ?? '')
		if (!match.valid) return

		const sent = await correspondingMessage.edit('loading...')
		const output = await extractMessageOutput(match.body ?? [])

		await sent.edit(output)
	}
})

client.on('message', async (msg) => {
	if (msg.content === '?help') {
		await msg.channel.send(HELP)
		return
	}

	const match = messageValid(msg.content)
	if (!match.valid) return

	const sent = await msg.channel.send('loading...')
	const output = await extractMessageOutput(match.body ?? [])

	if (output) {
		await sent.edit(output)
		responseMap.set(msg.id, sent)
	}
})

dotenv.config()
client.login(process.env.TOKEN)
