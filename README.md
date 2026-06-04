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

The idea pairs well with Benjamin Verbeek's talk,
[The agent that files its own bug reports](https://www.youtube.com/watch?v=KA5kPbdkK2E) and the [official Lovable blog post](https://lovable.dev/blog/we-gave-our-agent-a-vent-tool):
give the agent a low-friction route to report confusion, missing affordances,
and repeated workflow failures while the context is still fresh.

## Installation

### Cargo (crates.io)

```bash
cargo install vent-mcp
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

### MCP client shortcuts

If `vent` is on your `PATH`, add it as a local STDIO MCP server with:

```bash
codex mcp add vent -- vent
claude mcp add --transport stdio vent -- vent
```

Use an absolute path instead of `vent` if your MCP client does not inherit your
shell `PATH`.

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

## Channels, Sinks, Providers

`vent-mcp` keeps routing deliberately simple:

- A **channel** is the route the agent can choose, or omit to use
  `default_channel`. Each channel names one or more sinks.
- A **sink** is a concrete delivery destination, such as local JSONL logging or a
  specific Discord incoming webhook.
- A **provider** is a webhook payload shape. For example,
  `provider = "discord"` formats the event for a Discord incoming webhook.

Sink names and channel names do not have to match. A channel named `ci` can
target a sink named `discord-ci`, `log`, or both:

```toml
[[channels]]
name = "ci"
description = "Feedback about tests, builds, CI, or automation failures."
sinks = ["log", "discord-ci"]
```

The default `log` sink writes JSONL events to `vents.jsonl` beside the config
file. If `[logging].jsonl_dir` is set, JSONL events are written there instead.
Every sink must have a unique `name`, and every channel must reference at least
one defined sink.

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
10 second timeout; set `timeout_ms` to override that per sink. Discord incoming
webhook URLs normally do not need extra headers.

### Route CI Vents To Discord

Start from the generated config or [configs/config.sample.toml](configs/config.sample.toml).
Keep the `log` sink, add one Discord sink, then route the `ci` channel to both:

```diff
 [[channels]]
 name = "ci"
 description = "Feedback about tests, builds, CI, or automation failures."
-sinks = ["log"]
+sinks = ["log", "discord-ci"]

 [[sinks]]
 type = "jsonl"
 name = "log"
 
+[[sinks]]
+type = "webhook"
+name = "discord-ci"
+provider = "discord"
+url = "https://discord.com/api/webhooks/..."
+timeout_ms = 10000
```

The built-in `discord` provider maps `message` to Discord `content` and adds
`project` as an embed field. It does not include the channel name by default,
because the channel is routing metadata. You do not need to add a
`[providers.discord]` block unless you want to customize the payload.

With the config above, `channel = "ci"` vents are written to `vents.jsonl` and
posted to Discord. Other channels go only to the sinks they list.

The `default_channel` value must match one declared `[[channels]]` entry.

### Custom Provider Maps

Provider maps live in the same TOML config file. The left side is a vent event
field and the value is the dotted output JSON path. Numeric path segments create
arrays. If `field_label_key` is set, paths ending in `.value` also get a label
generated from the source key, such as `channel` to `Channel`.

```toml
[providers.discord]
field_label_key = "name"
message = "content"
project = "embeds.0.fields.0.value"

[[sinks]]
type = "webhook"
name = "discord-ci"
provider = "discord"
url = "https://discord.com/api/webhooks/..."
timeout_ms = 10000
```
