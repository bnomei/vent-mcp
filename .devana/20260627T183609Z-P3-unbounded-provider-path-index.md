DEVANA-FINDING: v1
DEVANA-STATE: duplicate | P3 | medium | security=no
DEVANA-KEY: src/provider.rs:133 | unbounded-provider-path-index

# Unbounded numeric provider path segment passes validation, then resizes/panics at webhook render

## Finding

Webhook provider maps accept dotted output paths whose numeric segments become
array indices. `OutputPath::parse` parses any numeric segment as `usize` with no
upper bound (`src/provider.rs:133-137`). The same `parse` runs during config
validation through `WebhookProviderConfig::validate -> ProviderTemplate::compile`
(`src/config.rs:529-531`, `src/provider.rs:54`), so a config containing a huge
index is accepted at startup.

At delivery time `insert_path_segments` materializes that index directly:

```rust
let array = ensure_array(current)?;
if array.len() <= *index {
    array.resize(*index + 1, Value::Null);   // src/provider.rs:245-246
}
...
array[*index] = value;                        // src/provider.rs:249
```

A large-but-valid index forces a multi-gigabyte `Vec<Value>` allocation (OOM /
process abort). `usize::MAX` overflows `*index + 1` (wraps to 0 in release),
`resize(0)` empties the array, then `array[usize::MAX]` panics with an
out-of-bounds index. Either way the failure happens on the first vent routed to
that webhook sink, not at config load.

## Violated Invariant Or Contract

The config module documents validation as the policy boundary that runs "before
the server starts or the CLI sends a message" so callers cannot configure
"ambiguous provider mappings" or "sink settings that would fail later in less
predictable ways" (`src/config.rs:1-8`, `:288-292`). A provider path that passes
validation but then OOMs or panics at render time is exactly the "fails later in
less predictable ways" case the module promises to prevent.

## Oracle

Module doc-comment contract in `src/config.rs` (validation prevents
configurations that fail unpredictably later), plus the implicit safety
expectation that a started server does not crash on the first valid request.

## Counterexample

```toml
default_channel = "feedback"

[providers.custom]
message = "embeds.50000000.value"   # or "embeds.18446744073709551615.value"

[[channels]]
name = "feedback"
description = "x"
sinks = ["hook"]

[[sinks]]
type = "webhook"
name = "hook"
url = "https://example.com/vent"
provider = "custom"
```

Server starts cleanly (validation passes). The first `vent` to channel
`feedback` renders the payload, hits `array.resize(50_000_001, Null)` and
allocates ~1.5 GB for one webhook (OOM on constrained hosts). With the
`usize::MAX` variant it panics at `array[usize::MAX]` instead.

## Why It Might Matter

Availability: a single configuration typo with a large numeric path segment turns
the first real vent into an OOM kill or a panicking dispatch task, rather than a
clear startup error. The crash surfaces only once the offending sink is exercised,
which makes it harder to attribute.

## Proof

- Control-flow trace: `OutputPath::parse` (no index bound) -> accepted by
  `ProviderTemplate::compile` during `validate()` -> at render
  `insert_path_segments` Index arm `resize(*index + 1, ...)` / `array[*index]`.
- Counterexample value: index `50000000` (OOM) or `18446744073709551615`
  (`*index + 1` wraps, then out-of-bounds index panic).

## Counterevidence Checked

- Provider paths come only from the local config file, which is operator-trusted;
  no agent/CLI message input flows into a path index. This bounds real-world
  reachability and is why this is P3, not a security issue.
- I checked for any clamp or bound: `OutputPath::parse` accepts any `usize`,
  `ProviderTemplate::compile` only dedupes canonical paths, and
  `insert_path_segments` resizes unconditionally. No guard prevents it.
- Strongest reason it might be false: "nobody writes a huge index." True for
  normal use, but validation explicitly claims to catch mappings that fail later,
  and the failure is a hard process crash on first use rather than a config-time
  rejection, so the gap is real and actionable.

## Suggested Next Step

Bound numeric segments in `OutputPath::parse` (e.g. reject indices above a small
cap such as 64) so an oversized index is rejected at config validation with a
clear `InvalidWebhookProviderPath` error instead of crashing at render time.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:`
stable unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: duplicate of P1 provider-numeric-index-unbounded (20260627T183640Z) — same root cause and same suggested fix (cap indices at 64). Already fixed: `OutputPath::parse` now rejects `array_index > MAX_PROVIDER_ARRAY_INDEX` (64) with `InvalidWebhookProviderPath` at config load, and `insert_path_segments` has a defense-in-depth bound check. Both counterexamples (`embeds.50000000.value` and the `usize::MAX` variant) are rejected at startup rather than crashing at render. No further change needed.

DEVANA-KEY: src/provider.rs:133 | unbounded-provider-path-index
DEVANA-SUMMARY: duplicate | P3 | medium | Duplicate of P1 provider-numeric-index-unbounded; oversized numeric provider path indices are now capped at 64 and rejected at config load via InvalidWebhookProviderPath.
