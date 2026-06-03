//! Binary entry point for running vent-mcp as either an MCP server or CLI sender.
//!
//! This file owns process-level concerns: command-line parsing, tracing setup,
//! configuration loading, graceful shutdown, and choosing whether to serve MCP
//! tools over STDIO or send one immediate vent from the command line. The
//! operational guardrails live here at the edge of the process: invalid
//! configuration exits before serving, CLI arguments are rejected when they would
//! be ambiguous, and messages are checked against configured channels before any
//! sink receives them.

#[cfg(feature = "cli")]
use clap::{CommandFactory, Parser, Subcommand};
use rmcp::ServiceExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
#[cfg(feature = "cli")]
use vent_mcp::VentRequest;
use vent_mcp::{project_name_from_current_dir, LoadedConfig, VentMcpServer, VentService};

#[derive(Debug, Clone, PartialEq, Eq)]
enum RunMode {
    Mcp,
    #[cfg(feature = "cli")]
    ListChannels,
    #[cfg(feature = "cli")]
    Vent {
        message: String,
        channel: Option<String>,
    },
}

#[cfg(feature = "cli")]
#[derive(Debug, Parser)]
#[command(
    name = "vent",
    version,
    about = "Run the vent-mcp STDIO server or send a vent from the command line.",
    disable_help_subcommand = true
)]
struct CliArgs {
    #[arg(long, help = "Run the MCP STDIO server.")]
    mcp: bool,

    #[arg(
        short,
        long,
        value_name = "CHANNEL",
        help = "Send the vent to a configured channel."
    )]
    channel: Option<String>,

    #[command(subcommand)]
    command: Option<CliCommand>,

    #[arg(
        value_name = "MESSAGE",
        num_args = 0..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Message to vent. Without arguments, the binary starts the MCP server."
    )]
    message: Vec<String>,
}

#[cfg(feature = "cli")]
#[derive(Debug, Subcommand)]
enum CliCommand {
    #[command(about = "List configured vent channels.")]
    List,
}

#[cfg(feature = "cli")]
impl CliArgs {
    /// Converts parsed CLI flags and positional words into the binary run mode.
    ///
    /// The parser keeps the default invocation as MCP mode while treating any
    /// real CLI input as an explicit request. Conflicting combinations exit via
    /// clap so users get standard error formatting and the process never starts
    /// the wrong behavior.
    fn into_run_mode(self) -> RunMode {
        let has_cli_args = self.mcp
            || self.channel.is_some()
            || self.command.is_some()
            || !self.message.is_empty();
        if !has_cli_args {
            return RunMode::Mcp;
        }

        if self.mcp {
            if self.channel.is_some() || self.command.is_some() || !self.message.is_empty() {
                CliArgs::command()
                    .error(
                        clap::error::ErrorKind::ArgumentConflict,
                        "--mcp cannot be combined with CLI arguments",
                    )
                    .exit();
            }
            return RunMode::Mcp;
        }

        match self.command {
            Some(CliCommand::List) => {
                if self.channel.is_some() || !self.message.is_empty() {
                    CliArgs::command()
                        .error(
                            clap::error::ErrorKind::ArgumentConflict,
                            "list cannot be combined with --channel or MESSAGE",
                        )
                        .exit();
                }
                RunMode::ListChannels
            }
            None => {
                if self.message.is_empty() {
                    CliArgs::command()
                        .error(
                            clap::error::ErrorKind::MissingRequiredArgument,
                            "MESSAGE is required when using CLI options",
                        )
                        .exit();
                }
                RunMode::Vent {
                    message: self.message.join(" "),
                    channel: self.channel,
                }
            }
        }
    }
}

/// Installs stderr tracing when `RUST_LOG` or another tracing env filter is set.
///
/// The server speaks MCP over stdout, so diagnostics must stay on stderr to avoid
/// corrupting protocol frames. If no filter is configured, tracing remains silent.
fn init_tracing() {
    if let Ok(filter) = EnvFilter::try_from_default_env() {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .init();
    }
}

/// Parses process arguments into the mode selected by the CLI-enabled binary.
#[cfg(feature = "cli")]
fn parse_run_mode() -> RunMode {
    CliArgs::parse().into_run_mode()
}

/// Accepts only a bare MCP server invocation when the CLI feature is disabled.
///
/// Arguments are treated as a user-facing build mismatch rather than ignored, so
/// a reduced binary cannot accidentally discard a requested vent command.
#[cfg(not(feature = "cli"))]
fn parse_run_mode() -> RunMode {
    if std::env::args_os().len() > 1 {
        eprintln!(
            "CLI mode is disabled in this build. Rebuild with `--features cli` to use command arguments."
        );
        std::process::exit(2);
    }
    RunMode::Mcp
}

/// Loads configuration or terminates the process with a concise stderr error.
///
/// Configuration is required for both server and CLI modes because it defines the
/// allowed channels and delivery sinks that constrain where vents may go.
fn load_config_or_exit() -> LoadedConfig {
    match LoadedConfig::load() {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("Error loading config: {error}");
            std::process::exit(1);
        }
    }
}

/// Waits for the first supported process shutdown signal.
///
/// The MCP server uses this future to cancel the running service cleanly on
/// Ctrl-C and, on Unix, SIGTERM. If SIGTERM registration fails, Ctrl-C remains
/// available as the fallback.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut term = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(_) => {
                let _ = ctrl_c.await;
                return;
            }
        };

        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}

/// Selects the requested run mode and delegates to the server or CLI path.
///
/// All modes share the same loaded configuration, which keeps channel validation
/// and sink delivery behavior consistent regardless of how a vent is submitted.
#[tokio::main]
async fn main() {
    init_tracing();

    let run_mode = parse_run_mode();
    let loaded = load_config_or_exit();

    match run_mode {
        RunMode::Mcp => run_mcp_server(loaded).await,
        #[cfg(feature = "cli")]
        RunMode::ListChannels => list_channels(&loaded),
        #[cfg(feature = "cli")]
        RunMode::Vent { message, channel } => vent_from_cli(loaded, message, channel).await,
    }
}

/// Runs the MCP STDIO service until it exits or receives a shutdown signal.
///
/// The service receives a dispatcher built from the validated configuration and a
/// project label derived from the current directory. Cancellation is wired so the
/// process can stop without leaving the protocol task waiting forever.
async fn run_mcp_server(loaded: LoadedConfig) {
    let project = project_name_from_current_dir();
    let service = VentService::new(loaded.into_config(), project);
    let server = VentMcpServer::new(service);

    tracing::info!("Starting vent-mcp server with stdio transport");

    let transport = rmcp::transport::io::stdio();
    match server.serve(transport).await {
        Ok(service) => {
            let cancel_token = service.cancellation_token();
            let mut wait = Box::pin(service.waiting());

            tokio::select! {
                result = &mut wait => {
                    if let Err(error) = result {
                        eprintln!("Server error: {error}");
                        std::process::exit(1);
                    }
                }
                _ = shutdown_signal() => {
                    cancel_token.cancel();
                    if let Err(error) = wait.await {
                        eprintln!("Server error: {error}");
                        std::process::exit(1);
                    }
                }
            }
        }
        Err(error) => {
            eprintln!("Failed to start server: {error}");
            std::process::exit(1);
        }
    }
}

/// Prints configured channels, marking the default for human CLI users.
#[cfg(feature = "cli")]
fn list_channels(loaded: &LoadedConfig) {
    let output = loaded.config().channel_list();
    for channel in &output.channels {
        if channel.name == output.default_channel {
            println!("{} (default) - {}", channel.name, channel.description);
        } else {
            println!("{} - {}", channel.name, channel.description);
        }
    }
}

/// Sends one validated CLI vent through the same dispatcher used by the server.
///
/// The command trims input, rejects empty messages, enforces configured channels,
/// and reports per-sink failures on stderr. This gives the CLI the same safety
/// boundaries as the MCP tool without requiring an MCP client.
#[cfg(feature = "cli")]
async fn vent_from_cli(loaded: LoadedConfig, message: String, channel: Option<String>) {
    let service = VentService::new(loaded.into_config(), project_name_from_current_dir());
    let output = service.send(VentRequest { message, channel }).await;

    if output.ok {
        println!("vented {} to {}", output.event_id, output.channel);
        return;
    }

    let error = output
        .error
        .unwrap_or_else(|| "delivery failed".to_string());
    eprintln!("Error: {error}");
    if output.event_id.is_empty() {
        std::process::exit(2);
    }
    std::process::exit(1);
}

#[cfg(all(test, feature = "cli"))]
mod tests {
    use clap::Parser;

    use super::{CliArgs, RunMode};

    /// Parses synthetic argv values through the same conversion used by the CLI.
    fn parse(args: &[&str]) -> RunMode {
        CliArgs::try_parse_from(args)
            .expect("parse args")
            .into_run_mode()
    }

    /// Verifies a bare invocation starts the MCP server mode.
    #[test]
    fn no_args_run_mcp_server() {
        assert_eq!(parse(&["vent"]), RunMode::Mcp);
    }

    /// Verifies the list subcommand selects channel listing mode.
    #[test]
    fn list_args_run_list_channels() {
        assert_eq!(parse(&["vent", "list"]), RunMode::ListChannels);
    }

    /// Verifies message words and channel flags become one CLI vent request.
    #[test]
    fn message_args_run_cli_vent() {
        assert_eq!(
            parse(&["vent", "--channel", "ux", "Workflow", "friction"]),
            RunMode::Vent {
                message: "Workflow friction".to_string(),
                channel: Some("ux".to_string()),
            }
        );
    }
}
