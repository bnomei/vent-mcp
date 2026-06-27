//! Internal library for the shipped `vent` binary.
//!
//! The crate exposes only the small facade the binary needs. Configuration DTOs,
//! sink mechanics, provider rendering, and MCP router details stay private so the
//! implementation can evolve without treating those internals as a stable library
//! API.

mod config;
mod delivery;
mod provider;
mod server;
mod sinks;
mod types;

/// Configuration load errors and the validated runtime policy derived from TOML.
pub use config::{ConfigError, LoadedConfig, RuntimeConfig};
/// Shared vent submission service used by the MCP server and CLI.
pub use delivery::{VentRequest, VentService};
/// MCP tool router that exposes channel listing and vent submission.
pub use server::VentMcpServer;
/// MCP response types and the privacy-preserving project label helper.
pub use types::{project_name_from_current_dir, ChannelInfo, ListChannelsOutput, VentOutput};
