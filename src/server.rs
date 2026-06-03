//! MCP tool implementation and dynamic tool-surface shaping.
//!
//! This module turns a validated application configuration into the tools exposed
//! to MCP clients. The server keeps the external surface deliberately constrained:
//! clients can inspect channels only when there is something meaningful to choose
//! from, vents are rejected before dispatch when the message or channel is
//! invalid, and returned sink statuses make delivery outcomes explicit.

use rmcp::handler::server::tool::{schema_for_type, ToolRouter};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};

use crate::config::RuntimeConfig;
use crate::delivery::{VentRequest, VentService};
use crate::types::{ListChannelsOutput, VentDefaultChannelInput, VentInput, VentOutput};

#[derive(Clone)]
pub struct VentMcpServer {
    service: VentService,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl VentMcpServer {
    /// Builds a server from validated configuration, sinks, and a project label.
    ///
    /// The tool router is derived from the configuration at construction time so
    /// clients see the smallest useful schema for the current channel setup.
    #[must_use]
    pub fn new(service: VentService) -> Self {
        let tool_router = configured_tool_router(service.config());
        Self {
            service,
            tool_router,
        }
    }

    /// Returns all configured channels and identifies the default channel.
    ///
    /// This tool is removed from the router when the configuration has only one
    /// default channel, because then the channel choice is not useful context for
    /// an MCP client.
    #[tool(
        name = "list_channels",
        description = "List the configured vent channels with short names and descriptions.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn list_channels(&self) -> Json<ListChannelsOutput> {
        Json(self.service.list_channels())
    }

    /// Validates and dispatches a feedback message to the requested channel.
    ///
    /// Empty messages and unknown channels are converted into structured failure
    /// outputs instead of reaching configured sinks. Successful requests share a
    /// single event record across all sinks so their delivery statuses describe
    /// the same vent.
    #[tool(
        name = "vent",
        description = "Escalate workflow feedback to a human when something failed or caused friction. Summarize what you tried to achieve, where it failed, and what you expected.",
        annotations(destructive_hint = false)
    )]
    async fn vent(&self, input: Parameters<VentInput>) -> Json<VentOutput> {
        let input = input.0;
        Json(
            self.service
                .send(VentRequest {
                    message: input.message,
                    channel: input.channel,
                })
                .await,
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VentMcpServer {
    /// Describes the MCP server capabilities and client-facing usage guidance.
    ///
    /// Instructions are kept in sync with the configured router so clients are
    /// not told to call a channel-listing tool that has intentionally been hidden.
    fn get_info(&self) -> rmcp::model::ServerInfo {
        let instructions = if self.tool_router.has_route("list_channels") {
            "Use list_channels to inspect available feedback channels. Use vent when something in the workflow needs to be escalated upstream to a human as a complaint or feedback message. Summarize what you tried to achieve, where it failed, and what you expected. The server records only the project directory name, not the full working directory path."
        } else {
            "Use vent when something in the workflow needs to be escalated upstream to a human as a complaint or feedback message. Summarize what you tried to achieve, where it failed, and what you expected. The server records only the project directory name, not the full working directory path."
        };

        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "vent-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(instructions)
    }
}

/// Builds the tool router and hides unnecessary channel metadata for one-channel configs.
///
/// When only the default channel exists, the `vent` schema is narrowed to message
/// input only. That keeps clients from inventing channel values that cannot be
/// useful while still preserving the full tool when multiple channels exist.
fn configured_tool_router(config: &RuntimeConfig) -> ToolRouter<VentMcpServer> {
    let mut router = VentMcpServer::tool_router();
    if config.has_only_default_channel() {
        router.remove_route("list_channels");
        if let Some(route) = router.map.get_mut("vent") {
            route.attr.input_schema = schema_for_type::<Parameters<VentDefaultChannelInput>>();
        }
    }

    router
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use tempfile::tempdir;

    use crate::config::{AppConfig, ChannelConfig, LoggingConfig, RuntimeConfig};
    use crate::delivery::VentService;

    use super::VentMcpServer;

    /// Builds a test server with a temp-backed dispatcher and stable project label.
    fn test_server(config: AppConfig) -> VentMcpServer {
        let dir = tempdir().expect("temp dir");
        let config = RuntimeConfig::from_app_config(config, dir.path().to_path_buf())
            .expect("runtime config");
        let service = VentService::new(config, "vent-mcp".to_string());
        VentMcpServer::new(service)
    }

    /// Checks whether the current `vent` input schema exposes a property.
    fn vent_schema_has_property(server: &VentMcpServer, property: &str) -> bool {
        server
            .tool_router
            .get("vent")
            .expect("vent tool")
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.contains_key(property))
    }

    /// Verifies one-channel configs hide channel choice and list metadata.
    #[test]
    fn single_default_channel_hides_channel_metadata() {
        let server = test_server(AppConfig::default());

        assert!(server.tool_router.has_route("vent"));
        assert!(!server.tool_router.has_route("list_channels"));
        assert_eq!(server.tool_router.list_all().len(), 1);
        assert!(vent_schema_has_property(&server, "message"));
        assert!(!vent_schema_has_property(&server, "channel"));

        let instructions = rmcp::ServerHandler::get_info(&server)
            .instructions
            .expect("instructions");
        assert!(!instructions.contains("list_channels"));
    }

    /// Verifies multi-channel configs expose both the list tool and channel input.
    #[test]
    fn multiple_channels_expose_channel_metadata() {
        let server = test_server(AppConfig {
            default_channel: "general".to_string(),
            channels: vec![
                ChannelConfig {
                    name: "general".to_string(),
                    description: "General feedback.".to_string(),
                },
                ChannelConfig {
                    name: "ux".to_string(),
                    description: "Workflow friction.".to_string(),
                },
            ],
            ..AppConfig::default()
        });

        assert!(server.tool_router.has_route("vent"));
        assert!(server.tool_router.has_route("list_channels"));
        assert_eq!(server.tool_router.list_all().len(), 2);
        assert!(vent_schema_has_property(&server, "message"));
        assert!(vent_schema_has_property(&server, "channel"));

        let instructions = rmcp::ServerHandler::get_info(&server)
            .instructions
            .expect("instructions");
        assert!(instructions.contains("list_channels"));
    }

    /// Verifies the list tool returns the configured default and channel set.
    #[tokio::test]
    async fn list_channels_returns_configured_channels() {
        let server = test_server(AppConfig {
            default_channel: "general".to_string(),
            channels: vec![
                ChannelConfig {
                    name: "general".to_string(),
                    description: "General feedback.".to_string(),
                },
                ChannelConfig {
                    name: "ux".to_string(),
                    description: "Workflow friction.".to_string(),
                },
            ],
            ..AppConfig::default()
        });

        let output = server.list_channels().await.0;

        assert_eq!(output.default_channel, "general");
        assert_eq!(output.channels.len(), 2);
        assert_eq!(output.channels[1].name, "ux");
    }

    /// Verifies omitted channel input falls back to the configured default.
    #[tokio::test]
    async fn vent_uses_default_channel_when_omitted() {
        let dir = tempdir().expect("temp dir");
        let config = AppConfig {
            logging: LoggingConfig {
                jsonl_dir: Some(dir.path().to_string_lossy().to_string()),
            },
            ..AppConfig::default()
        };
        let server = test_server(config);

        let output = server
            .vent(rmcp::handler::server::wrapper::Parameters(
                crate::types::VentInput {
                    message: "The task queue kept changing mid-run.".to_string(),
                    channel: None,
                },
            ))
            .await
            .0;

        assert!(output.ok);
        assert_eq!(output.channel, "general");
        assert_eq!(output.event_id.len(), 8);
        assert!(output
            .event_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric()));
        assert_eq!(output.error, None);
    }

    /// Verifies unknown channels are rejected before sink dispatch.
    #[tokio::test]
    async fn vent_rejects_unknown_channel() {
        let server = test_server(AppConfig::default());

        let output = server
            .vent(rmcp::handler::server::wrapper::Parameters(
                crate::types::VentInput {
                    message: "Something useful.".to_string(),
                    channel: Some("missing".to_string()),
                },
            ))
            .await
            .0;

        assert!(!output.ok);
        assert_eq!(output.channel, "missing");
        assert_eq!(output.error.as_deref(), Some("unknown channel: missing"));
    }
}
