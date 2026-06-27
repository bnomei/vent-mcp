DEVANA-FINDING: v1
DEVANA-STATE: open | P2 | medium | security=yes
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

DEVANA-KEY: src/sinks.rs:337 | webhook-short-secret-leak
DEVANA-SUMMARY: open | P2 | medium | Short webhook URL/query/header secrets can leak through MCP-visible delivery error messages.