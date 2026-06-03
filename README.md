# vent-mcp

[![Crates.io Version](https://img.shields.io/crates/v/vent-mcp)](https://crates.io/crates/vent-mcp)
[![CI](https://img.shields.io/github/actions/workflow/status/bnomei/vent-mcp/ci.yml?branch=main)](https://github.com/bnomei/vent-mcp/actions/workflows/ci.yml)
[![Crates.io Downloads](https://img.shields.io/crates/d/vent-mcp)](https://crates.io/crates/vent-mcp)
[![License](https://img.shields.io/crates/l/vent-mcp)](https://crates.io/crates/vent-mcp)
[![Discord](https://flat.badgen.net/badge/discord/bnomei?color=7289da&icon=discord&label)](https://discordapp.com/users/bnomei)
[![Buymecoffee](https://flat.badgen.net/badge/icon/donate?icon=buymeacoffee&color=FF813F&label)](https://www.buymeacoffee.com/bnomei)

Allow your agent to complain before the same paper cut becomes tomorrow's bug.

`vent-mcp` is a small STDIO MCP server that gives agents a non-destructive
place to send process feedback, complaints, and friction reports while they are
working. The crate is named `vent-mcp`; the installed binary is named `vent`.
The Rust library surface is internal support for that shipped binary, not a
stable embedding API.

The idea pairs well with Benjamin Verbeek's Lovable talk,
[The agent that files its own bug reports](https://www.youtube.com/watch?v=KA5kPbdkK2E):
give the agent a low-friction route to report confusion, missing affordances,
and repeated workflow failures while the context is still fresh.

## Installation

### Cargo (crates.io)

```bash
cargo install vent-mcp
```

### Homebrew

```bash
brew install bnomei/vent-mcp/vent
```

### GitHub Releases

Download a prebuilt archive from the GitHub Releases page, extract it, and
place `vent` on your `PATH`.

### From source

```bash
git clone https://github.com/bnomei/vent-mcp.git
cd vent-mcp
cargo build --release
```

For a JSONL-only build without webhook/HTTP dependencies:

```bash
cargo build --release --no-default-features
```

For a CLI-enabled JSONL-only build:

```bash
cargo build --release --no-default-features --features cli
```

Then configure your MCP client to run:

```json
{
  "command": "vent"
}
```

## CLI

With no arguments, `vent` runs as the STDIO MCP server.

```bash
vent
```

The same binary can also be used directly from the shell when built with the
`cli` feature, which is enabled by default:

```bash
vent list
vent "The queue changed mid-run."
vent --channel ci "The failing check output was hard to correlate."
vent --mcp
```

## Tools

- `list_channels`: lists configured channel names and descriptions.
- `vent`: sends a message to a channel and returns an ACK-only delivery result.

`vent` accepts:

```json
{
  "message": "The failing check output was hard to correlate with the changed file.",
  "channel": "ci"
}
```

If `channel` is omitted, the configured `default_channel` is used.

## Configuration

Path precedence:

1. `VENT_MCP_CONFIG`
2. `$XDG_CONFIG_HOME/vent-mcp/config.toml`
3. `~/.config/vent-mcp/config.toml`

When the default path is missing, `vent-mcp` creates a usable default config.
If `VENT_MCP_CONFIG` points to a missing file, startup fails.

See [configs/config.sample.toml](configs/config.sample.toml).

## Sinks

The default `log` sink writes JSONL events to `vents.jsonl` beside the config
file. If `[logging].jsonl_dir` is set, JSONL events are written there instead.
If a config has no sinks, `vent` uses the built-in `log` JSONL sink.

Webhook sinks POST the vent event as JSON. With no provider, the raw event is
sent unchanged:

```json
{
  "id": "aZ8pQ2xK",
  "timestamp": "2026-06-03T12:34:56Z",
  "channel": "ci",
  "message": "The failing check output was hard to correlate with the changed file.",
  "project": "my-repo"
}
```

Each event includes an id, UTC timestamp, channel, message, and the project
directory name. It does not include the full current working directory path.

Header values are read from environment variables. Webhook requests default to a
10 second timeout; set `timeout_ms` to override that per sink.

```toml
[[sinks]]
type = "webhook"
name = "relay"
url = "https://example.com/vent"
timeout_ms = 10000
headers = [
  { name = "Authorization", env = "VENT_MCP_WEBHOOK_AUTHORIZATION" },
]
```

Provider maps live in the same TOML config file. The left side is a vent event
field and the value is the dotted output JSON path. Numeric path segments create
arrays. If `field_label_key` is set, paths ending in `.value` also get a label
generated from the source key, such as `channel` to `Channel`.

```toml
[providers.discord]
field_label_key = "name"
message = "content"
channel = "embeds.0.fields.0.value"
project = "embeds.0.fields.1.value"

[[sinks]]
type = "webhook"
name = "discord"
provider = "discord"
url = "https://discord.com/api/webhooks/..."
```

### Built-In Provider Defaults

`vent-mcp` ships a sane preconfigured provider list for common webhook receivers
in North America and Europe. These defaults are ready to use from the generated
config and [configs/config.sample.toml](configs/config.sample.toml): pick a
provider, add the webhook URL, and keep the mapping as-is unless the receiving
workflow needs a custom shape.

Automation hubs receive the full raw event. Chat providers receive the smallest
useful message shape their incoming webhook supports.

| Provider | Target | Default payload |
| --- | --- | --- |
| `zapier` | [Webhooks by Zapier](https://help.zapier.com/hc/en-us/articles/8496288690317-Trigger-Zap-workflows-from-webhooks) | Raw event fields |
| `make` | [Make webhooks](https://help.make.com/webhooks) | Raw event fields |
| `n8n` | [n8n Webhook node](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.webhook/) | Raw event fields |
| `pipedream` | [Pipedream HTTP triggers](https://pipedream.com/docs/workflows/building-workflows/triggers/) | Raw event fields |
| `workato` | [Workato webhooks](https://docs.workato.com/connectors/workato-webhooks.html) | Raw event fields |
| `ifttt` | [IFTTT Webhooks](https://help.ifttt.com/hc/en-us/articles/115010230347-Webhooks-service-FAQ) | `value1`, `value2`, `value3` |
| `slack` | [Slack incoming webhooks](https://api.slack.com/messaging/webhooks) | `text` plus attachment fields |
| `mattermost` | [Mattermost incoming webhooks](https://developers.mattermost.com/integrate/webhooks/incoming/) | `text` plus attachment fields |
| `discord` | [Discord Execute Webhook](https://docs.discord.com/developers/resources/webhook#execute-webhook) | `content` plus embed fields |
| `microsoft_teams` | [Microsoft Teams incoming webhooks](https://learn.microsoft.com/en-us/microsoftteams/platform/webhooks-and-connectors/how-to/add-incoming-webhook) | `text` |
| `google_chat` | [Google Chat incoming webhooks](https://developers.google.com/workspace/chat/quickstart/webhooks) | `text` |
| `webex` | [Webex incoming webhooks](https://apphub.webex.com/applications/incoming-webhooks-cisco-systems-38054-23307-75252) | `markdown` |

Telegram, WhatsApp Business, PagerDuty, and Opsgenie are not built-in defaults
yet. They are useful targets, but they need static required fields or secrets
outside the current dynamic field mapper: chat or recipient ids, message type
constants, routing keys, event actions, bearer tokens, or API keys.

### Zapier

Use a Zapier Catch Hook when you want Zapier to parse the JSON fields, or Catch
Raw Hook when later Zap steps should receive the whole JSON body.

```toml
[providers.zapier]
id = "id"
timestamp = "timestamp"
channel = "channel"
message = "message"
project = "project"

[[sinks]]
type = "webhook"
name = "zapier"
provider = "zapier"
url = "https://hooks.zapier.com/hooks/catch/123456/abcdef/"
timeout_ms = 10000
```

Zapier accepts valid JSON webhook payloads, so the default `vent-mcp` event body
can be sent directly.

The same raw mapping is used for `make`, `n8n`, `pipedream`, and `workato`.

### Slack

The default Slack provider maps `message` to the top-level `text` fallback and
maps `channel`/`project` into attachment fields. Mattermost uses the same
Slack-compatible mapping.

```toml
[providers.slack]
field_label_key = "title"
message = "text"
channel = "attachments.0.fields.0.value"
project = "attachments.0.fields.1.value"

[[sinks]]
type = "webhook"
name = "slack"
provider = "slack"
url = "https://hooks.slack.com/services/..."
timeout_ms = 10000
headers = [
  { name = "Authorization", env = "VENT_MCP_SLACK_WEBHOOK_AUTHORIZATION" },
]
```

### Discord

Discord expects at least one message field such as `content`, `embeds`,
`components`, a file, or a poll. The default Discord provider maps `message` to
`content` and maps `channel`/`project` into generated embed fields.

```toml
[providers.discord]
field_label_key = "name"
message = "content"
channel = "embeds.0.fields.0.value"
project = "embeds.0.fields.1.value"

[[sinks]]
type = "webhook"
name = "discord"
provider = "discord"
url = "https://discord.com/api/webhooks/..."
timeout_ms = 10000
headers = [
  { name = "Authorization", env = "VENT_MCP_DISCORD_WEBHOOK_AUTHORIZATION" },
]
```

### Text Webhooks

Microsoft Teams, Google Chat, and Webex use message-only provider maps because
their simple incoming webhook paths accept a single text or markdown body. The
same sink shape works for all three:

```toml
[providers.microsoft_teams]
message = "text"

[[sinks]]
type = "webhook"
name = "microsoft_teams"
provider = "microsoft_teams"
url = "https://..."
timeout_ms = 10000
```
