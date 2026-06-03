//! Integration coverage for the installed `vent` binary behavior.
//!
//! These tests exercise the process boundary rather than calling library helpers:
//! they resolve the compiled binary, isolate config-related environment variables,
//! and verify the CLI/server startup paths that protect users from missing or
//! invalid configuration. CLI-feature tests also confirm channel listing and JSONL
//! delivery so the command-line path stays aligned with the MCP server guardrails.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::{NamedTempFile, TempDir};

/// Finds the built test binary for the `vent` executable.
///
/// Cargo usually exposes an exact env var for integration tests, but the fallback
/// derives the release/test target directory so the tests remain robust across
/// package and binary naming.
fn bin_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_vent") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_vent-mcp") {
        return PathBuf::from(path);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(Path::parent).expect("target dir");
    let mut bin = target_dir.join("vent");
    if cfg!(windows) {
        bin.set_extension("exe");
    }
    bin
}

/// Creates a command for the binary with config environment overrides removed.
///
/// Each test opts into only the environment it needs, preventing a developer's
/// shell config from affecting the guardrail behavior under test.
fn command() -> Command {
    let mut command = Command::new(bin_path());
    command.env_remove("VENT_MCP_CONFIG");
    command.env_remove("XDG_CONFIG_HOME");
    command
}

/// Escapes a filesystem path for insertion into a TOML string literal.
#[cfg(feature = "cli")]
fn toml_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Writes a temporary two-channel config used by CLI integration tests.
///
/// The config points JSONL output at the supplied directory so the tests can
/// verify delivery without touching a user's real config or log files.
#[cfg(feature = "cli")]
fn write_cli_config(jsonl_dir: &Path) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("temp config");
    writeln!(
        file,
        r#"default_channel = "general"

[logging]
jsonl_dir = "{}"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[[channels]]
name = "ux"
description = "Workflow friction."
sinks = ["log"]

[[sinks]]
type = "jsonl"
name = "log"
"#,
        toml_path(jsonl_dir)
    )
    .expect("write config");
    file.flush().expect("flush config");
    file
}

/// Runs a command with stdin closed to make the STDIO MCP server exit promptly.
///
/// This lets the default server-start path be tested without leaving a child
/// process blocked on input.
fn run_with_stdin_closed(mut command: Command) -> std::process::Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn binary");
    drop(child.stdin.take());
    child.wait_with_output().expect("wait for output")
}

/// Verifies an explicit missing config path fails instead of auto-creating a file.
#[test]
fn cli_rejects_missing_env_config() {
    let dir = TempDir::new().expect("temp dir");
    let missing = dir.path().join("missing.toml");

    let output = command()
        .env("VENT_MCP_CONFIG", &missing)
        .output()
        .expect("run binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("config file not found"));
}

/// Verifies invalid TOML is rejected at process startup.
#[test]
fn cli_rejects_invalid_env_config() {
    let mut file = NamedTempFile::new().expect("temp config");
    writeln!(file, "not = = valid").expect("write config");

    let output = command()
        .env("VENT_MCP_CONFIG", file.path())
        .output()
        .expect("run binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to parse config file"));
}

/// Verifies the default MCP path bootstraps a config then exits when STDIO closes.
#[test]
fn cli_auto_creates_default_config_and_exits_when_stdio_closes() {
    let dir = TempDir::new().expect("temp dir");
    let xdg = dir.path().join("xdg");
    let config = xdg.join("vent-mcp").join("config.toml");

    let mut cmd = command();
    cmd.env("XDG_CONFIG_HOME", &xdg);

    let output = run_with_stdin_closed(cmd);

    assert!(config.exists());
    assert!(!output.status.success());
}

/// Verifies the CLI list command prints configured channels and default marker.
#[test]
#[cfg(feature = "cli")]
fn cli_lists_channels() {
    let dir = TempDir::new().expect("temp dir");
    let config = write_cli_config(dir.path());

    let output = command()
        .env("VENT_MCP_CONFIG", config.path())
        .arg("list")
        .output()
        .expect("run binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("general (default) - General feedback."));
    assert!(stdout.contains("ux - Workflow friction."));
}

/// Verifies CLI message input vents to the default channel and writes JSONL.
#[test]
#[cfg(feature = "cli")]
fn cli_vents_to_default_channel() {
    let dir = TempDir::new().expect("temp dir");
    let config = write_cli_config(dir.path());

    let output = command()
        .env("VENT_MCP_CONFIG", config.path())
        .arg("The queue changed mid-run.")
        .output()
        .expect("run binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("vented "));
    assert!(stdout.contains(" to general"));

    let raw = std::fs::read_to_string(dir.path().join("vents.jsonl")).expect("jsonl file");
    let event: serde_json::Value =
        serde_json::from_str(raw.lines().next().expect("jsonl line")).expect("event json");
    assert_eq!(event["id"].as_str().expect("id").len(), 8);
    assert_eq!(event["channel"], "general");
    assert_eq!(event["message"], "The queue changed mid-run.");
}

/// Verifies CLI channel selection routes a message to the named channel.
#[test]
#[cfg(feature = "cli")]
fn cli_vents_to_named_channel() {
    let dir = TempDir::new().expect("temp dir");
    let config = write_cli_config(dir.path());

    let output = command()
        .env("VENT_MCP_CONFIG", config.path())
        .arg("--channel")
        .arg("ux")
        .arg("The workflow hid the useful error.")
        .output()
        .expect("run binary");

    assert!(output.status.success());

    let raw = std::fs::read_to_string(dir.path().join("vents.jsonl")).expect("jsonl file");
    let event: serde_json::Value =
        serde_json::from_str(raw.lines().next().expect("jsonl line")).expect("event json");
    assert_eq!(event["id"].as_str().expect("id").len(), 8);
    assert_eq!(event["channel"], "ux");
    assert_eq!(event["message"], "The workflow hid the useful error.");
}

/// Verifies argument use is rejected in binaries compiled without CLI support.
#[test]
#[cfg(not(feature = "cli"))]
fn cli_args_report_disabled_without_cli_feature() {
    let output = command().arg("list").output().expect("run binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CLI mode is disabled"));
}
