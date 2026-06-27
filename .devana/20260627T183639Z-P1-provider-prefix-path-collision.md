DEVANA-FINDING: v1
DEVANA-STATE: fixed | P1 | high | security=no
DEVANA-KEY: src/provider.rs:61 | provider-prefix-path-collision

# Provider prefix paths can silently drop mapped webhook fields

## Finding

Webhook provider validation rejects only exact duplicate output paths, not ancestor/descendant prefix pairs. Depending on field iteration order, a shorter parent path can overwrite a longer nested path that was already rendered, producing a successful webhook delivery whose JSON body omits an earlier mapped field.

## Violated Invariant Or Contract

`WebhookProviderConfig::validate` documents that provider paths are constrained so payloads "cannot collide or silently drop fields" (`src/config.rs`). A vent accepted after config load should not deliver a webhook payload missing a configured field mapping.

## Oracle

`ProviderTemplate::compile` deduplication via `target_paths.insert(output_path.canonical())` (`src/provider.rs:61-66`) and the validate docstring (`src/config.rs:527-528`).

## Counterexample

```toml
[providers.bad]
message = "payload.body"
project = "payload"
```

Channel routes to a webhook sink with `provider = "bad"`. Event `{ message: "blocked", project: "vent-mcp", ... }`.

`ProviderTemplate::compile` stores fields in BTreeMap iteration order (`message` before `project`). `render` inserts `payload.body`, then `object.insert("payload", <project string>)` at the leaf replaces the nested object. Webhook returns `ok: true` with `payload` equal to the project string only; the message field is gone.

## Why It Might Matter

Automation webhooks (Discord, Slack, Zapier) can receive incomplete payloads without any delivery error, causing operators to miss vent content they configured explicitly.

## Proof

Dataflow trace: config load accepts both canonical paths `"payload.body"` and `"payload"` → `VentService::send` → `webhook_payload` → `ProviderTemplate::render` → nested insert then parent leaf overwrite → `post_webhook` succeeds → sink status `ok: true`.

Control-flow asymmetry: reversed mapping (`project = "payload"`, `message = "payload.body"`) fails at render with "path collides with non-object value" instead of succeeding, so the bug depends on field-name ordering rather than failing consistently at validation.

## Counterevidence Checked

Exact duplicate paths are rejected (`DuplicateWebhookProviderPath`). Built-in providers use non-overlapping paths (`embeds.0.fields.0.value` vs `content`). Reverse field order fails loudly rather than silently, which does not prevent the forward-order silent-drop case.

## Suggested Next Step

Add compile-time prefix-collision detection between all provider output paths (reject parent/child pairs), or change leaf insertion to fail when overwriting a non-scalar container.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. Confirmed: `ProviderTemplate::compile` only rejected exact-duplicate canonical paths, so a parent path (`payload`) and a descendant (`payload.body`) both passed validation; at render time the parent's leaf insert overwrites the descendant's container (BTreeMap field order `message` < `project`), silently dropping the message field while the webhook returns ok. Fix: `compile` now also rejects parent/child path pairs via segment-aware prefix check (`is_path_ancestor`, dotted canonical form with trailing-`.` guard so siblings like `a.1`/`a.10` are not flagged), returning new `ConfigValidationError::CollidingWebhookProviderPath`. Added load-time regression test for the counterexample config. Full `cargo test` green.

DEVANA-KEY: src/provider.rs:61 | provider-prefix-path-collision
DEVANA-SUMMARY: fixed | P1 | high | Prefix provider output paths pass validation but can silently drop earlier mapped fields on successful webhook delivery.