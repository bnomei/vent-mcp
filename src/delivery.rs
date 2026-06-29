//! Shared delivery service used by MCP tools and CLI mode.
//!
//! This module owns the request policy that used to be split across adapters:
//! trim input, choose the default channel, reject unknown channels, construct one
//! event, dispatch it, and reduce sink statuses to the terse acknowledgement the
//! caller receives.

use std::sync::Arc;

use crate::config::RuntimeConfig;
use crate::sinks::SinkDispatcher;
use crate::types::{first_delivery_error, ListChannelsOutput, VentEvent, VentOutput};

/// Trimmed vent message and optional channel override for delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VentRequest {
    pub message: String,
    pub channel: Option<String>,
}

/// Validates vent input and dispatches one event through configured channel sinks.
#[derive(Clone)]
pub struct VentService {
    config: Arc<RuntimeConfig>,
    dispatcher: SinkDispatcher,
    project: Arc<String>,
}

impl VentService {
    /// Builds a service from validated runtime config and a project directory label.
    #[must_use]
    pub fn new(config: RuntimeConfig, project: String) -> Self {
        let dispatcher = SinkDispatcher::new(Arc::new(config));
        let config = dispatcher.config();
        Self {
            config,
            dispatcher,
            project: Arc::new(project),
        }
    }

    #[must_use]
    pub(crate) fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Returns configured channels and the default channel name.
    #[must_use]
    pub fn list_channels(&self) -> ListChannelsOutput {
        self.config.channel_list()
    }

    /// Trims input, enforces channel policy, dispatches one event, and reports the first sink failure.
    pub async fn send(&self, request: VentRequest) -> VentOutput {
        let message = request.message.trim();
        if message.is_empty() {
            return failure_output(
                self.config.default_channel().to_string(),
                "message must not be empty".to_string(),
            );
        }

        let channel = request
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|channel| !channel.is_empty())
            .unwrap_or_else(|| self.config.default_channel());

        if !self.config.has_channel(channel) {
            return failure_output(channel.to_string(), format!("unknown channel: {channel}"));
        }

        let event = VentEvent::new(
            channel.to_string(),
            message.to_string(),
            self.project.as_ref().clone(),
        );
        let statuses = self.dispatcher.dispatch(&event).await;
        let ok = statuses.iter().all(|status| status.ok);

        VentOutput {
            ok,
            event_id: event.id.to_string(),
            channel: event.channel,
            error: first_delivery_error(&statuses),
        }
    }
}

fn failure_output(channel: String, message: String) -> VentOutput {
    VentOutput {
        ok: false,
        event_id: String::new(),
        channel,
        error: Some(message),
    }
}
