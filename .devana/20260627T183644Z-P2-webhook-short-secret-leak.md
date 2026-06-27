DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | medium | security=yes
DEVANA-KEY: src/sinks.rs:337 | webhook-short-secret-leak

# Webhook error previews skip redaction for secrets shorter than four characters

## Finding

When a webhook returns a non-success HTTP status, `sanitize_webhook_error_body` replaces known secret substrings only if `secret.len() >= 4`. Short URL query tokens and env-backed header values can therefore appear verbatim in `VentOutput.error`, which is returned to the MCP STDIO client.

## Violated Invariant Or Contract

Webhook delivery collects URL and header material into `webhook_secret_values` specifically to keep credentials out of caller-visible error text. Sub-four-character secrets are collected but not redacted, breaking that contract for short tokens.

## Oracle

`sanitize_webhook_error_body` (`src/sinks.rs:332-339`) and `webhook_secret_values` (`src/sinks.rs:296-328`). Existing test `webhook_status_errors_are_sanitized_and_short` only covers a long secret.

## Counterexample

Webhook sink URL `https://hooks.example.com/vent?token=abc` with header env `KEY=xy` (length 2). Remote returns `401` with body `invalid token abc`. `VentOutput.error` contains the literal `abc`; header value `xy` would also pass through if echoed and length `< 4`.

## Why It Might Matter

Short API keys and webhook tokens can be exposed to the MCP client (and agent logs) even though longer secrets in the same code path are redacted.

## Proof

Dataflow trace: webhook POST with env header / URL query secret → non-2xx response body echoing short token → `webhook_status_error` → `sanitize_webhook_error_body` skips replacement → `first_delivery_error` → MCP `vent` tool returns error string containing the secret.

## Counterevidence Checked

Full trimmed URL string is added to the secret list and is redacted when echoed wholesale. Request-path errors (`webhook_request_error`) do not include header values. Missing-env header errors expose only the variable name, not its value. MCP STDIO is usually local/trusted, but the redaction path is explicitly intended to hide credentials from returned errors.

## Suggested Next Step

Redact all entries in `webhook_secret_values` regardless of length, or apply a minimum-length threshold only to non-credential path segments while always redacting header and query values.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. Confirmed: `sanitize_webhook_error_body` only redacted secrets with `len() >= 4`, so a len-3 query token (`abc`) or len-2 header value (`xy`) in an echoed non-2xx body reached `VentOutput.error`. Fix (report option 1): redact every collected credential regardless of length, guarding only against empty strings (an empty query value like `?token=` would corrupt output via `String::replace` with an empty needle — this is also what the old `>= 4` guard incidentally prevented). Also sort secrets longest-first so a short secret that is a substring of a longer one doesn't leave fragments. Added a regression test for short + empty secrets. Full `cargo test` green (webhook is a default feature).

DEVANA-KEY: src/sinks.rs:337 | webhook-short-secret-leak
DEVANA-SUMMARY: fixed | P2 | medium | Short webhook URL/query/header secrets can leak through MCP-visible delivery error messages.