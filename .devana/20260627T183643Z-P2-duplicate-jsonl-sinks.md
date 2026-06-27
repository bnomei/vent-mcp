DEVANA-FINDING: v1
DEVANA-STATE: open | P2 | medium | security=no
DEVANA-KEY: src/sinks.rs:142 | duplicate-jsonl-sinks

# Multiple JSONL sinks on one channel write duplicate identical records

## Finding

A channel may reference multiple distinct `type = "jsonl"` sinks. `SinkDispatcher::dispatch` fans out to every configured sink, and each JSONL sink appends to the same `jsonl_dir/vents.jsonl` path. One accepted vent therefore produces multiple identical JSONL lines differing only in per-sink status labels.

## Violated Invariant Or Contract

One `VentEvent` with a single `event_id` should correspond to one persisted JSONL record for a given log file destination. Channel validation forbids duplicate sink name references but does not prevent multiple JSONL sink definitions targeting the same file.

## Oracle

`dispatch` uses `join_all` over all channel sinks (`src/sinks.rs:93-101`). `write_jsonl` always opens `self.config.jsonl_dir().join("vents.jsonl")` (`src/sinks.rs:142`) regardless of sink `name`.

## Counterexample

```toml
[[channels]]
name = "feedback"
description = "Feedback."
sinks = ["log", "audit"]

[[sinks]]
type = "jsonl"
name = "log"

[[sinks]]
type = "jsonl"
name = "audit"
```

One `vent` call returns one `event_id`, but `vents.jsonl` contains two lines with the same `id`, `message`, `channel`, and `project`.

## Why It Might Matter

Downstream consumers tailing `vents.jsonl` can double-count vents, and operators may believe the extra sink name creates a separate audit destination when it only duplicates the same file.

## Proof

Control-flow trace: `VentService::send` creates one `VentEvent` → `dispatch` → two `dispatch_one` calls for `SinkConfig::Jsonl` → both `write_jsonl` append the same serialized event to `vents.jsonl` under the shared `jsonl_lock`.

## Counterevidence Checked

`jsonl_lock` prevents interleaved writes but not duplication. README documents one shared `vents.jsonl` per `jsonl_dir` but does not state that multiple JSONL sinks should duplicate records. Separate sink names affect `SinkDeliveryStatus.sink`, not the output path.

## Suggested Next Step

Either reject channels that reference more than one JSONL sink, or include sink name in the JSONL filename/path so distinct JSONL sinks write to distinct files.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.

DEVANA-KEY: src/sinks.rs:142 | duplicate-jsonl-sinks
DEVANA-SUMMARY: open | P2 | medium | Two JSONL sinks on one channel append duplicate identical lines to the same `vents.jsonl` file.