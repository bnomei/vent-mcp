DEVANA-FINDING: v1
DEVANA-STATE: open | P2 | medium | security=no
DEVANA-KEY: src/config.rs:712 | jsonl-dir-empty-string-cwd

# Empty `jsonl_dir` writes vents to the process working directory

## Finding

When `logging.jsonl_dir` is set to an empty string, `resolve_jsonl_dir` treats it as a present value and resolves it to an empty relative path. JSONL events are then written to `vents.jsonl` in the process current working directory instead of beside the config file.

## Violated Invariant Or Contract

README and `resolve_jsonl_dir` imply that omitted `jsonl_dir` falls back to the config directory. Empty environment variables are treated as unset elsewhere (`non_empty_os_string`), so an empty `jsonl_dir` string should behave like omission rather than selecting an unrelated directory.

## Oracle

`resolve_jsonl_dir` uses `.as_deref().map(expand_tilde)` and only uses `config_dir` in the `None` branch (`src/config.rs:712-717`). `AppConfig::validate` does not inspect `logging.jsonl_dir`.

## Counterexample

```toml
[logging]
jsonl_dir = ""
```

Config loaded from `/home/user/.config/vent-mcp/config.toml`. Process started with CWD `/tmp/project`. After `vent`, events appear at `/tmp/project/vents.jsonl`, not `/home/user/.config/vent-mcp/vents.jsonl`.

## Why It Might Matter

Operators can lose vent history in an unexpected directory, or leak feedback into a repository working tree when the MCP server inherits a project CWD.

## Proof

Dataflow trace: TOML `jsonl_dir = ""` → `Some("")` → `expand_tilde("")` → `PathBuf::from("")` → `RuntimeConfig.jsonl_dir` → `write_jsonl` opens `jsonl_dir().join("vents.jsonl")` as relative `vents.jsonl` from CWD.

## Counterevidence Checked

Omitted/`None` `jsonl_dir` correctly anchors to `config_dir`. Non-empty absolute paths work as expected. Tests always set explicit non-empty `jsonl_dir` in temp directories; no test covers the empty-string case.

## Suggested Next Step

Treat empty or whitespace-only `jsonl_dir` like `None` during `resolve_jsonl_dir`, and add a validation error if an explicit value cannot resolve to a usable directory.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.

DEVANA-KEY: src/config.rs:712 | jsonl-dir-empty-string-cwd
DEVANA-SUMMARY: open | P2 | medium | Empty `jsonl_dir` resolves to CWD-relative `vents.jsonl` instead of the config directory.