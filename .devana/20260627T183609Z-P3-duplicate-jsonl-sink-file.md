DEVANA-FINDING: v1
DEVANA-STATE: duplicate | P3 | medium | security=no
DEVANA-KEY: src/sinks.rs:142 | duplicate-jsonl-sink-file

# Multiple JSONL sinks share one fixed file, so a channel routing to two of them double-writes each event

## Finding

The JSONL sink always writes to a fixed filename in a single directory:

```rust
const JSONL_FILE_NAME: &str = "vents.jsonl";          // src/sinks.rs:38
...
let path = self.config.jsonl_dir().join(JSONL_FILE_NAME);  // src/sinks.rs:142
```

The sink's own `name` is never used to derive its destination, and
`JsonlSinkConfig` has no path/filename field (`src/config.rs:420-424`). Config
validation only enforces that sink names are unique and that a channel does not
list the *same* sink name twice (`DuplicateSinkName`, `DuplicateChannelSink`); it
does not prevent two distinct JSONL sinks. The maintainers themselves define two
JSONL sinks (`log` and `audit`) in tests (`src/sinks.rs:521-528`).

Because every JSONL sink resolves to the same `vents.jsonl`, two differently
named JSONL sinks are not distinct destinations. A channel that routes to both
writes the identical event (same event id) to the same file twice.

## Violated Invariant Or Contract

The README states "A **sink** is a concrete delivery destination" and "Every sink
must have a unique `name`", implying distinctly named sinks deliver to distinct
destinations. For JSONL sinks this does not hold: name has no effect on the
destination, so "two sinks" collapse to one file. `dispatch` also reports each as
an independent successful delivery (`src/sinks.rs:93-102`), masking that they are
the same target.

## Oracle

README "Channels, Sinks, Providers" section (sink = concrete delivery
destination; unique sink names) versus the implementation's fixed
`JSONL_FILE_NAME` that ignores the sink name.

## Counterexample

```toml
default_channel = "feedback"

[[channels]]
name = "feedback"
description = "x"
sinks = ["log", "audit"]

[[sinks]]
type = "jsonl"
name = "log"

[[sinks]]
type = "jsonl"
name = "audit"
```

One `vent` to `feedback` fans out to both sinks; both append to
`<jsonl_dir>/vents.jsonl`. The resulting file contains two lines with the same
`id` for a single vent, and the response reports two successful sink deliveries.

## Why It Might Matter

Data correctness: a single vent produces duplicate records sharing one trace id,
which corrupts counts and any downstream consumer that treats lines as distinct
events. Operators who define separate JSONL sinks (e.g. `log` and `audit`)
expecting separate files silently get one merged, duplicated file with no error.

## Proof

- Contract mismatch: README "sink = concrete delivery destination / unique name"
  vs `src/sinks.rs:142` resolving every JSONL sink to the same fixed path.
- Counterexample value: the two-JSONL-sink channel above yields two identical
  lines (same `id`) in `vents.jsonl` from one vent.

## Counterevidence Checked

- `JsonlSinkConfig` exposes only `name` (no `path`/`file`), so there is no
  configuration that could make two JSONL sinks distinct; this is a real gap, not
  user error in choosing paths.
- Validation guards checked: `DuplicateSinkName` blocks reused names,
  `DuplicateChannelSink` blocks the same name twice in one channel, but neither
  blocks two distinct JSONL sinks or a channel referencing both.
- Strongest reason it might be false: maybe one shared log file is intended. The
  sample config and README only ever show a single `log` sink, but the code and
  tests still permit multiple JSONL sinks, and there is no warning or dedupe, so
  the duplicate-write behavior is reachable and unguarded.

## Suggested Next Step

Decide the intended model and enforce it: either reject more than one JSONL sink
(or more than one JSONL sink per channel) during validation, or give
`JsonlSinkConfig` an optional per-sink filename so distinct JSONL sinks write to
distinct files.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:`
stable unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: duplicate of P2 duplicate-jsonl-sinks (20260627T183643Z, same DEVANA-KEY base src/sinks.rs:142, identical log+audit counterexample). The actionable data-duplication bug — one channel routing one event to two jsonl sinks producing two identical lines — is now fixed there via `ConfigValidationError::MultipleJsonlSinksInChannel` in `AppConfig::validate`. The broader suggestion (give each jsonl sink a distinct file so distinct sink names = distinct destinations) is intentionally NOT adopted: the single shared `vents.jsonl` is the intended model, as evidenced by the maintainer test `dispatch_uses_selected_channel_sinks`, which defines two jsonl sinks (`log`, `audit`) across two separate channels writing to the same file and asserts success (passes validation via `from_app_config`). Cross-channel jsonl sinks do not duplicate any single event, so no further change is warranted.

DEVANA-KEY: src/sinks.rs:142 | duplicate-jsonl-sink-file
DEVANA-SUMMARY: duplicate | P3 | medium | Duplicate of P2 duplicate-jsonl-sinks; the per-channel double-write is fixed via MultipleJsonlSinksInChannel, and the shared single vents.jsonl is the intended model (cross-channel jsonl sinks do not duplicate events).
