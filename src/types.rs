//! Shared data model for vent-mcp requests, responses, and stored events.
//!
//! This module defines the narrow contract between MCP tools, the optional CLI,
//! and delivery sinks. The types keep the tool surface intentionally small: a
//! caller can send a message, optionally choose a configured channel, and receive
//! a concise acknowledgement or first delivery error. Event construction also
//! limits project context to the directory name so feedback can be useful without
//! recording a full local path.

use std::env;

use chrono::{DateTime, Utc};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ChannelInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListChannelsOutput {
    pub default_channel: String,
    pub channels: Vec<ChannelInfo>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct VentInput {
    pub message: String,
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct VentDefaultChannelInput {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VentOutput {
    pub ok: bool,
    /// Short trace id for the accepted event, not a deduplication key.
    pub event_id: String,
    pub channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SinkDeliveryStatus {
    pub sink: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct EventId(String);

impl EventId {
    #[must_use]
    fn new_random() -> Self {
        Self(short_event_id())
    }

    #[cfg(test)]
    #[must_use]
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VentEvent {
    pub id: EventId,
    pub timestamp: DateTime<Utc>,
    pub channel: String,
    pub message: String,
    pub project: String,
}

impl VentEvent {
    /// Creates a timestamped event with a fresh ID for dispatch to all sinks.
    ///
    /// The caller supplies only the already-validated channel, message, and
    /// project label. IDs and timestamps are assigned here so every sink receives
    /// the same immutable record.
    #[must_use]
    pub fn new(channel: String, message: String, project: String) -> Self {
        Self {
            id: EventId::new_random(),
            timestamp: Utc::now(),
            channel,
            message,
            project,
        }
    }
}

#[must_use]
pub fn first_delivery_error(statuses: &[SinkDeliveryStatus]) -> Option<String> {
    statuses.iter().find(|status| !status.ok).map(|status| {
        status
            .message
            .clone()
            .unwrap_or_else(|| format!("{} failed", status.sink))
    })
}

fn short_event_id() -> String {
    const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    const ID_LEN: usize = 8;

    let mut value = Uuid::new_v4().as_u128();
    let mut id = String::with_capacity(ID_LEN);
    for _ in 0..ID_LEN {
        let index = (value % ALPHABET.len() as u128) as usize;
        id.push(ALPHABET[index] as char);
        value /= ALPHABET.len() as u128;
    }
    id
}

/// Returns a privacy-preserving project label based on the current directory.
///
/// Only the final path component is used, which gives receivers enough context
/// to understand where feedback came from without exposing the full workspace
/// path. If the directory cannot be read, the label falls back to `unknown`.
#[must_use]
pub fn project_name_from_current_dir() -> String {
    env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::VentEvent;

    #[test]
    fn event_ids_are_short_base62_strings() {
        let event = VentEvent::new(
            "general".to_string(),
            "Something happened.".to_string(),
            "vent-mcp".to_string(),
        );

        assert_eq!(event.id.as_str().len(), 8);
        assert!(event
            .id
            .as_str()
            .chars()
            .all(|character| character.is_ascii_alphanumeric()));
    }
}
