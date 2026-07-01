# vent-mcp

[![Crates.io Version](https://img.shields.io/crates/v/vent-mcp)](https://crates.io/crates/vent-mcp)
[![CI](https://img.shields.io/github/actions/workflow/status/bnomei/vent-mcp/ci.yml?branch=main)](https://github.com/bnomei/vent-mcp/actions/workflows/ci.yml)
[![Crates.io Downloads](https://img.shields.io/crates/d/vent-mcp)](https://crates.io/crates/vent-mcp)
[![License](https://img.shields.io/crates/l/vent-mcp)](https://crates.io/crates/vent-mcp)
[![Discord](https://flat.badgen.net/badge/discord/bnomei?color=7289da&icon=discord&label)](https://discordapp.com/users/bnomei)
[![Buymecoffee](https://flat.badgen.net/badge/icon/donate?icon=buymeacoffee&color=FF813F&label)](https://www.buymeacoffee.com/bnomei)

Allow your agent to complain before the same paper cut becomes tomorrow's bug.

`vent-mcp` is a small STDIO MCP server that gives agents a non-destructive
place to send actionable feedback while they work. Agents can report blocked
work, repeated failures, missing capabilities, confusing workflows, or
operational friction without interrupting the task flow.

The crate is named `vent-mcp`; the installed binary is named `vent`. The Rust
library surface supports that binary and is not a stable embedding API.

The idea pairs well with Benjamin Verbeek's talk,
[The agent that files its own bug reports](https://www.youtube.com/watch?v=KA5kPbdkK2E)
and the [official Lovable blog post](https://lovable.dev/blog/we-gave-our-agent-a-vent-tool).

## Quickstart

Use this path when you want `vent` available to a local MCP client and a default
JSONL feedback log.

### Prerequisites

- Rust 1.88 or newer and Cargo, when installing from crates.io or source.
- An MCP client that can run a local STDIO server, such as Codex or Claude.

### Install

```bash
cargo install vent-mcp
```

This installs the `vent` binary with the default `cli` and `webhook` features.

### Create the default config

Run:

```bash
vent list
```

On first run, `vent` creates a default config at
`$XDG_CONFIG_HOME/vent-mcp/config.toml` or
`~/.config/vent-mcp/config.toml`. The default config contains one `feedback`
channel and one local JSONL sink.

Expected output:

```txt
feedback (default) - Blocked work, repeated failures, or confusing workflows. Avoid routine progress updates.
```

JSONL events are written to `vents.jsonl` beside the config file unless you set
`[logging].jsonl_dir`.

### Register the MCP server

If `vent` is on your `PATH`, add it as a local STDIO MCP server:

```bash
codex mcp add vent -- vent
claude mcp add --transport stdio vent -- vent
```

Use an absolute path to `vent` if your MCP client does not inherit your shell
`PATH`.

## Installation

### Cargo

```bash
cargo install vent-mcp
```

### GitHub Releases

Download a prebuilt archive from the
[GitHub Releases](https://github.com/bnomei/vent-mcp/releases) page, extract it,
and place `vent` on your `PATH`.

### From source

```bash
git clone https://github.com/bnomei/vent-mcp.git
cd vent-mcp
cargo build --release
```

The binary is written to `target/release/vent`.

### Feature builds

Build without webhook and HTTP dependencies:

```bash
cargo build --release --no-default-features
```

Build JSONL-only delivery while keeping the shell CLI:

```bash
cargo build --release --no-default-features --features cli
```

When the `cli` feature is disabled, the binary only accepts a bare MCP server
invocation. Any CLI arguments exit with an error.

## CLI

With no arguments, `vent` starts the STDIO MCP server:

```bash
vent
```

Use the same binary from a shell when the `cli` feature is enabled:

```bash
vent list
vent "The queue changed mid-run."
vent --channel automation "The failing check output was hard to correlate."
vent --mcp
```

Successful CLI delivery prints the event id and channel:

```txt
vented aZ8pQ2xK to feedback
```

Message text is trimmed before delivery. Empty messages and unknown channels are
rejected before any sink receives an event.

## MCP tools

`vent-mcp` exposes a small tool surface:

| Tool | Purpose |
| --- | --- |
| `vent` | Send actionable feedback to the configured default channel or to a named channel. |
| `list_channels` | List configured channel names and descriptions when multiple channels are available. |

When the config contains only the default channel, `list_channels` is hidden and
the `vent` input schema only contains `message`. When multiple channels exist,
`list_channels` is exposed and `vent` accepts an optional `channel`.

Example multi-channel `vent` input:

```json
{
  "message": "The failing check output was hard to correlate with the changed file.",
  "channel": "automation"
}
```

The `vent` response is an acknowledgement:

```json
{
  "ok": true,
  "eventId": "aZ8pQ2xK",
  "channel": "automation"
}
```

If delivery fails, `ok` is `false` and `error` contains the first sink failure.
The `eventId` is a short trace id, not a deduplication key. Agents should not
send repeated vents for the same issue unless they have new root-cause evidence.

## Configuration

`vent` resolves config in this order:

1. `VENT_MCP_CONFIG`
2. `$XDG_CONFIG_HOME/vent-mcp/config.toml`
3. `~/.config/vent-mcp/config.toml`

Implicit XDG or home-directory configs are created when missing. If
`VENT_MCP_CONFIG` points to a missing file, startup fails instead of creating it.

Start from [configs/config.sample.toml](configs/config.sample.toml) or the
generated default config.

### Minimal config

```toml
default_channel = "feedback"

[[channels]]
name = "feedback"
description = "Blocked work, repeated failures, or confusing workflows. Avoid routine progress updates."
sinks = ["log"]

[[sinks]]
type = "jsonl"
name = "log"
```

### Config reference

| Setting | Required | Description |
| --- | --- | --- |
| `default_channel` | Yes | Channel used when callers omit `channel`. It must match one `[[channels]]` entry. |
| `[logging].jsonl_dir` | No | Directory for `vents.jsonl`. Empty or omitted values use the config directory. `~` and `~/...` expand from `HOME`. |
| `[[channels]].name` | Yes | Channel name agents may choose. Names must be lowercase ASCII letters, digits, underscores, or dashes, up to 64 characters. |
| `[[channels]].description` | Yes | Short description exposed to MCP clients and `vent list`. |
| `[[channels]].sinks` | Yes | One or more sink names. Every referenced sink must exist. A channel may reference at most one JSONL sink. |
| `[[sinks]].type` | Yes | `jsonl` or, with the `webhook` feature, `webhook`. |
| `[[sinks]].name` | Yes | Unique sink name referenced by channels. |
| `[[sinks]].url` | Webhook only | HTTP or HTTPS endpoint. |
| `[[sinks]].provider` | No | Built-in or custom provider map. Omit it or use `raw` to send the canonical event JSON. |
| `[[sinks]].headers` | No | Environment-backed webhook headers. Header values are read when the event is sent. |
| `[[sinks]].timeout_ms` | Webhook only | Positive timeout in milliseconds. Defaults to `10000`. |
| `[providers.<name>]` | No | Maps canonical event fields onto webhook JSON output paths. |

Every vent event contains:

```json
{
  "id": "aZ8pQ2xK",
  "timestamp": "2026-06-03T12:34:56Z",
  "channel": "automation",
  "message": "The failing check output was hard to correlate with the changed file.",
  "project": "my-repo"
}
```

The `project` value is only the current directory name. `vent-mcp` does not
record the full local workspace path.

## Channels, sinks, and providers

`vent-mcp` keeps routing deliberately simple:

- A channel is the route the agent can choose, or omit to use `default_channel`.
- A sink is a concrete destination, such as local JSONL logging or a webhook.
- A provider is a webhook payload shape.

Sink names and channel names do not have to match. For example, an `automation`
channel can write to the default log and post to Discord:

```toml
default_channel = "feedback"

[[channels]]
name = "feedback"
description = "General feedback."
sinks = ["log"]

[[channels]]
name = "automation"
description = "Build, test, CI/CD, deployment, scheduler, or pipeline failures that blocked progress."
sinks = ["log", "discord-automation"]

[[sinks]]
type = "jsonl"
name = "log"

[[sinks]]
type = "webhook"
name = "discord-automation"
provider = "discord"
url = "https://discord.com/api/webhooks/..."
timeout_ms = 10000
```

With this config, `channel = "automation"` vents are written to `vents.jsonl`
and posted to Discord. Other channels go only to the sinks they list.

## Webhook providers

Webhook sinks POST JSON. With no provider, or with `provider = "raw"`, the raw
vent event is sent unchanged.

Built-in provider maps include:

| Provider | Shape |
| --- | --- |
| `zapier`, `make`, `n8n`, `pipedream`, `workato` | Raw canonical event fields. |
| `ifttt` | `message`, `channel`, and `project` mapped to `value1`, `value2`, and `value3`. |
| `slack`, `mattermost` | Text plus attachment-style project field. |
| `discord` | `content` plus an embed field for project. |
| `microsoft_teams`, `google_chat`, `webex` | Text-only message field. |

Custom provider maps live in the same TOML config file. The left side is a
canonical event field and the value is a dotted JSON output path. Numeric path
segments create arrays. If `field_label_key` is set, paths ending in `.value`
also get a generated label such as `Project`.

```toml
[providers.discord]
field_label_key = "name"
message = "content"
project = "embeds.0.fields.0.value"

[[sinks]]
type = "webhook"
name = "discord-automation"
provider = "discord"
url = "https://discord.com/api/webhooks/..."
timeout_ms = 10000
```

Webhook headers read values from environment variables:

```toml
[[sinks]]
type = "webhook"
name = "private-endpoint"
url = "https://example.test/vent"

[[sinks.headers]]
name = "Authorization"
env = "VENT_WEBHOOK_AUTH"
```

If a webhook returns a non-2xx response, the error preview is shortened and
known URL or header secrets are redacted before the caller sees it.

## Troubleshooting

### `config file not found`

Cause: `VENT_MCP_CONFIG` points to a path that does not exist.

Fix: Create the file at that path, unset `VENT_MCP_CONFIG`, or point it at an
existing TOML config.

### `unknown channel: <name>`

Cause: The CLI or MCP caller requested a channel that is not declared in
`[[channels]]`.

Fix: Run `vent list`, choose one of the configured names, or add the channel and
its sink route to the config.

### `message must not be empty`

Cause: The message was empty after trimming whitespace.

Fix: Send a specific, actionable message that says what failed and what would
unblock the work.

### `missing environment variable <NAME>`

Cause: A webhook header references an environment variable that is not set in
the `vent` process environment.

Fix: Export the variable before starting the MCP client or remove the header from
the sink.

### `CLI mode is disabled`

Cause: The binary was built without the `cli` feature and received CLI
arguments.

Fix: Use the binary only as an MCP server or rebuild with `--features cli`.

## Development

Run the test suite:

```bash
cargo test
```

Build a release binary:

```bash
cargo build --release
```

Source anchors:

- [src/main.rs](src/main.rs): binary mode selection, config loading, CLI output,
  and MCP server startup.
- [src/config.rs](src/config.rs): config path resolution, defaults,
  validation, and built-in provider maps.
- [src/server.rs](src/server.rs): MCP tool definitions and dynamic tool-surface
  shaping.
- [src/delivery.rs](src/delivery.rs): message trimming, channel selection,
  event construction, and acknowledgement output.
- [src/sinks.rs](src/sinks.rs): JSONL writing, webhook delivery, env-backed
  headers, timeout handling, and error redaction.
- [src/provider.rs](src/provider.rs): provider path validation and webhook JSON
  rendering.
- [tests/cli.rs](tests/cli.rs): process-level CLI behavior and config
  bootstrapping coverage.
