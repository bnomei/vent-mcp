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

pub use config::{ConfigError, LoadedConfig, RuntimeConfig};
pub use delivery::{VentRequest, VentService};
pub use server::VentMcpServer;
pub use types::{project_name_from_current_dir, ChannelInfo, ListChannelsOutput, VentOutput};
