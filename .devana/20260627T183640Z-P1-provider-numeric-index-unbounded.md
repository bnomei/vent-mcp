DEVANA-FINDING: v1
DEVANA-STATE: fixed | P1 | high | security=no
DEVANA-KEY: src/provider.rs:246 | provider-numeric-index-unbounded

# Provider numeric path indices are unbounded and can abort the MCP process

## Finding

Provider output paths accept any `usize` numeric segment at config validation time. At render time, `insert_path_segments` calls `array.resize(index + 1, Value::Null)` with no upper bound. A large index can exhaust memory; `usize::MAX` wraps `index + 1` to zero in release builds and then indexes out of bounds, panicking the server process.

## Violated Invariant Or Contract

Config validation should reject provider mappings that cannot be rendered safely. Vent dispatch must not crash the MCP/CLI process from a loaded configuration alone.

## Oracle

`OutputPath::parse` accepts any successful `segment.parse::<usize>()` (`src/provider.rs:133-137`). No magnitude check exists in `ProviderTemplate::compile` or `WebhookProviderConfig::validate`.

## Counterexample

Large index OOM:

```toml
[providers.bad]
message = "items.5000000"
```

First vent to a webhook sink using `provider = "bad"` allocates millions of `serde_json::Value` slots synchronously inside the MCP handler.

`usize::MAX` panic (64-bit):

```toml
[providers.bad]
message = "embeds.18446744073709551615.content"
```

`array.resize(usize::MAX + 1, ...)` wraps to `resize(0)` in release; subsequent `array[usize::MAX]` panics.

## Why It Might Matter

A mistyped provider path or malicious config can hang or crash the vent MCP server during an otherwise normal `vent` tool call, taking down the agent feedback channel.

## Proof

Control-flow trace: valid TOML load → `VentService::send` → `webhook_payload` → `ProviderTemplate::render` → `insert_path_segments` (`src/provider.rs:243-246`) → unbounded `Vec::resize` or wrapping arithmetic → OOM kill or panic instead of structured `failed_status`.

## Counterevidence Checked

Leading numeric segments are rejected (`index == 0` guard). Non-numeric segments become object keys without resize. Default built-in providers only use small indices like `embeds.0` and `attachments.0`. Absurd indices are user-authored in config, but the server has no runtime bound or graceful error path.

## Suggested Next Step

Cap numeric path segments during `ProviderTemplate::compile` (for example reject indices above a small constant aligned with built-in providers), and use checked arithmetic before `resize`.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. Confirmed: `OutputPath::parse` accepted any `usize` index and `insert_path_segments` did `array.resize(index + 1, ..)` unbounded, so `items.5000000` OOMs and `embeds.18446744073709551615.content` overflows `index + 1` (wraps to 0 in release) then panics on the out-of-bounds `array[usize::MAX]`. Fix: cap indices at compile time with `MAX_PROVIDER_ARRAY_INDEX = 64` in `OutputPath::parse` (oversized → `InvalidWebhookProviderPath` at load), plus a defense-in-depth bound check in `insert_path_segments` returning a structured error instead of resizing/overflowing. Added a load-time regression test for `items.5000000`. Full `cargo test` green.

DEVANA-KEY: src/provider.rs:246 | provider-numeric-index-unbounded
DEVANA-SUMMARY: fixed | P1 | high | Unbounded provider array indices can OOM or panic the vent MCP process on a normal vent call.