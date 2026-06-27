DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | medium | security=no
DEVANA-KEY: src/config.rs:725 | jsonl-dir-tilde-missing-home

# Tilde `jsonl_dir` paths fall back to literal `~/…` when `HOME` is unset

## Finding

`expand_tilde` only joins `~/…` against the home directory when `HOME` is set. If `HOME` is missing or empty, the function returns the original tilde path unchanged. JSONL writes then land under a cwd-relative directory whose name literally starts with `~`, not under the user's home directory.

## Violated Invariant Or Contract

Sample config and README present `jsonl_dir = "~/.local/state/vent-mcp"` as a home-relative logging location. Config path resolution fails closed with `HomeDirectoryNotSet` when home is required, but `jsonl_dir` tilde expansion silently degrades instead of erroring.

## Oracle

`configs/config.sample.toml` documents `jsonl_dir = "~/.local/state/vent-mcp"`. `expand_tilde` (`src/config.rs:720-731`) and `home_dir` (`src/config.rs:734-737`) only consult `HOME`.

## Counterexample

`VENT_MCP_CONFIG=/etc/vent-mcp/config.toml` with:

```toml
[logging]
jsonl_dir = "~/.local/state/vent-mcp"
```

Process runs with `HOME` unset and CWD `/srv/agent/workspace`. First vent creates `/srv/agent/workspace/~/.local/state/vent-mcp/vents.jsonl`.

## Why It Might Matter

Vent feedback intended for a private home-state directory can appear inside a shared workspace tree, including directories tracked by version control.

## Proof

Dataflow trace: `jsonl_dir = "~/…"` → `resolve_jsonl_dir` → `expand_tilde` sees `home_dir() == None` → `Path::new("~/…")` → `write_jsonl` → `create_dir_all` under cwd-relative `~/…` path.

## Counterevidence Checked

`jsonl_dir = "~"` alone also falls back to literal `~` when home is missing. Omitted `jsonl_dir` still uses `config_dir` and does not depend on `HOME`. This mainly affects explicit tilde paths in environments where config loading succeeded without needing `HOME` (for example `VENT_MCP_CONFIG` or `XDG_CONFIG_HOME`).

## Suggested Next Step

Return a config validation error when `jsonl_dir` contains `~` and home cannot be resolved, matching the fail-closed behavior used for default config path resolution.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. Confirmed: `expand_tilde` returned the literal `~/…` path when `home_dir()` was `None`, so a home-relative `jsonl_dir` landed under the CWD. Fix: `expand_tilde` now fails closed (returns `Err`) when a tilde path can't resolve home, and `resolve_jsonl_dir` is now fallible, surfacing `ConfigValidationError::JsonlDirHomeNotSet` at config load — matching the fail-closed `HomeDirectoryNotSet` behavior of default config path resolution. Refactored expansion into an injectable `expand_tilde_with(value, home)` (mirrors existing `resolve_config_path_with`) so the no-home case is tested without mutating global `HOME`. Full `cargo test` green.

DEVANA-KEY: src/config.rs:725 | jsonl-dir-tilde-missing-home
DEVANA-SUMMARY: fixed | P2 | medium | Tilde `jsonl_dir` values write to a literal `~/…` path under CWD when `HOME` is unset.