//! Delivery sink dispatch for persisted and webhook-backed vent events.
//!
//! Sinks are the last boundary before a vent leaves the MCP server. This module
//! fans one validated event out to the destinations named by the selected
//! channel, reports each result independently, and keeps delivery mechanics
//! explicit: JSONL writes are serialized to avoid interleaved records, webhook
//! headers come from environment variables, and provider mappings turn the
//! canonical event into receiver-shaped JSON only after configuration validation
//! has constrained the paths.

#[cfg(feature = "webhook")]
use std::env;
#[cfg(feature = "webhook")]
use std::future::Future;
#[cfg(feature = "webhook")]
use std::pin::Pin;
use std::sync::Arc;
#[cfg(feature = "webhook")]
use std::time::Duration;

use futures_util::future::join_all;
#[cfg(feature = "webhook")]
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
#[cfg(feature = "webhook")]
use reqwest::StatusCode;
#[cfg(feature = "webhook")]
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::OnceCell;

#[cfg(feature = "webhook")]
use crate::config::WebhookSinkConfig;
use crate::config::{RuntimeConfig, SinkConfig};
#[cfg(feature = "webhook")]
use crate::provider::webhook_payload;
use crate::types::{SinkDeliveryStatus, VentEvent};

const JSONL_FILE_NAME: &str = "vents.jsonl";

/// Fans one vent event to the sinks configured for its channel.
#[derive(Clone)]
pub(crate) struct SinkDispatcher {
    config: Arc<RuntimeConfig>,
    jsonl_dir_ready: Arc<OnceCell<()>>,
    #[cfg(feature = "webhook")]
    webhook_sender: Arc<dyn WebhookSender>,
    jsonl_lock: Arc<tokio::sync::Mutex<()>>,
}

impl SinkDispatcher {
    /// Creates a dispatcher that writes JSONL and, when enabled, sends webhooks.
    ///
    /// The JSONL directory is resolved once from config and config location, while
    /// directory creation itself is deferred until the first JSONL write.
    #[must_use]
    pub(crate) fn new(config: Arc<RuntimeConfig>) -> Self {
        Self {
            config,
            jsonl_dir_ready: Arc::new(OnceCell::new()),
            #[cfg(feature = "webhook")]
            webhook_sender: Arc::new(ReqwestWebhookSender::new(reqwest::Client::new())),
            jsonl_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    #[must_use]
    pub(crate) fn config(&self) -> Arc<RuntimeConfig> {
        self.config.clone()
    }

    /// Creates a dispatcher using a caller-provided webhook sender implementation.
    ///
    /// Tests use this to record webhook payloads without network I/O, and any
    /// alternate sender still receives the same validated sink configuration.
    #[must_use]
    #[cfg(all(test, feature = "webhook"))]
    pub(crate) fn with_webhook_sender(
        config: Arc<RuntimeConfig>,
        webhook_sender: Arc<dyn WebhookSender>,
    ) -> Self {
        Self {
            config,
            jsonl_dir_ready: Arc::new(OnceCell::new()),
            webhook_sender,
            jsonl_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Sends an event to the sinks configured for its channel.
    ///
    /// Dispatch is concurrent across the channel's sink entries, but each JSONL
    /// write still uses an internal lock so records remain one complete line at a
    /// time.
    pub async fn dispatch(&self, event: &VentEvent) -> Vec<SinkDeliveryStatus> {
        let Some(sinks) = self.config.sinks_for_channel(&event.channel) else {
            return vec![failed_status(
                event.channel.clone(),
                format!("unknown channel: {}", event.channel),
            )];
        };

        join_all(sinks.into_iter().map(|sink| self.dispatch_one(sink, event))).await
    }

    /// Sends an event to one resolved sink definition.
    async fn dispatch_one(&self, sink: &SinkConfig, event: &VentEvent) -> SinkDeliveryStatus {
        let label = sink_label(sink);
        match sink {
            SinkConfig::Jsonl(_) => self.write_jsonl(event, label).await,
            #[cfg(feature = "webhook")]
            SinkConfig::Webhook(config) => self.post_webhook(config, event, label).await,
        }
    }

    /// Appends a serialized event to the configured JSONL file.
    ///
    /// The directory is created once, the event is written as exactly one newline-
    /// terminated JSON object, and every I/O error becomes a structured sink
    /// status rather than a panic.
    async fn write_jsonl(&self, event: &VentEvent, sink: String) -> SinkDeliveryStatus {
        let _guard = self.jsonl_lock.lock().await;
        let dir = self.config.jsonl_dir().to_path_buf();
        if let Err(error) = self
            .jsonl_dir_ready
            .get_or_try_init(|| async {
                tokio::fs::create_dir_all(&dir)
                    .await
                    .map_err(|error| format!("failed to create jsonl directory: {error}"))
            })
            .await
        {
            return failed_status(sink, error.clone());
        }

        let mut line = match serde_json::to_vec(event) {
            Ok(line) => line,
            Err(error) => {
                return failed_status(sink, format!("failed to serialize event: {error}"))
            }
        };
        line.push(b'\n');

        let path = self.config.jsonl_dir().join(JSONL_FILE_NAME);
        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
        {
            Ok(file) => file,
            Err(error) => {
                return failed_status(sink, format!("failed to open jsonl file: {error}"))
            }
        };

        if let Err(error) = file.write_all(&line).await {
            return failed_status(sink, format!("failed to write jsonl event: {error}"));
        }
        if let Err(error) = file.flush().await {
            return failed_status(sink, format!("failed to flush jsonl event: {error}"));
        }

        SinkDeliveryStatus {
            sink,
            ok: true,
            message: None,
        }
    }

    /// Renders and posts one webhook sink payload for an event.
    ///
    /// Header lookup and provider payload rendering happen before network I/O so
    /// configuration or environment problems are reported as local sink failures.
    #[cfg(feature = "webhook")]
    async fn post_webhook(
        &self,
        config: &WebhookSinkConfig,
        event: &VentEvent,
        sink: String,
    ) -> SinkDeliveryStatus {
        let headers = match webhook_headers(config) {
            Ok(headers) => headers,
            Err(error) => return failed_status(sink, error),
        };

        let payload = match webhook_payload(&self.config, config, event) {
            Ok(payload) => payload,
            Err(error) => return failed_status(sink, error),
        };

        match self.webhook_sender.post(config, headers, &payload).await {
            Ok(()) => SinkDeliveryStatus {
                sink,
                ok: true,
                message: None,
            },
            Err(error) => failed_status(sink, error),
        }
    }
}

#[cfg(feature = "webhook")]
pub(crate) trait WebhookSender: Send + Sync {
    /// Posts a rendered webhook payload and returns a delivery error message on failure.
    ///
    /// The trait keeps tests and alternate transports decoupled from reqwest while
    /// preserving the dispatcher's timeout, header, and payload decisions.
    fn post<'a>(
        &'a self,
        config: &'a WebhookSinkConfig,
        headers: HeaderMap,
        payload: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

#[cfg(feature = "webhook")]
struct ReqwestWebhookSender {
    client: reqwest::Client,
}

#[cfg(feature = "webhook")]
impl ReqwestWebhookSender {
    /// Wraps a reqwest client for use by the dispatcher.
    fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[cfg(feature = "webhook")]
impl WebhookSender for ReqwestWebhookSender {
    /// Sends the webhook request with configured timeout and JSON body.
    ///
    /// Non-success HTTP statuses are treated as delivery failures so callers can
    /// see that the event did not land cleanly.
    fn post<'a>(
        &'a self,
        config: &'a WebhookSinkConfig,
        headers: HeaderMap,
        payload: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let secret_values = webhook_secret_values(config, &headers);
            let response = self
                .client
                .post(config.url.trim())
                .headers(headers)
                .timeout(Duration::from_millis(config.timeout_ms))
                .json(payload)
                .send()
                .await
                .map_err(|error| webhook_request_error(&error, config.timeout_ms))?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.ok();
                return Err(webhook_status_error(
                    status,
                    body.as_deref(),
                    &secret_values,
                ));
            }

            Ok(())
        })
    }
}

#[cfg(feature = "webhook")]
fn webhook_request_error(error: &reqwest::Error, timeout_ms: u64) -> String {
    if error.is_timeout() {
        return format!("webhook timed out after {timeout_ms}ms");
    }
    if error.is_connect() {
        return "webhook connection failed".to_string();
    }
    "webhook request failed".to_string()
}

#[cfg(feature = "webhook")]
fn webhook_status_error(
    status: StatusCode,
    body: Option<&str>,
    secret_values: &[String],
) -> String {
    let mut message = format!("webhook HTTP {status}");
    if let Some(preview) = body
        .map(|body| sanitize_webhook_error_body(body, secret_values))
        .filter(|preview| !preview.is_empty())
    {
        message.push_str(": ");
        message.push_str(&preview);
    }
    message
}

#[cfg(feature = "webhook")]
fn webhook_secret_values(config: &WebhookSinkConfig, headers: &HeaderMap) -> Vec<String> {
    let mut values = Vec::new();
    let url = config.url.trim();
    if !url.is_empty() {
        values.push(url.to_string());
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(password) = parsed.password() {
                values.push(password.to_string());
            }
            if let Some(query) = parsed.query() {
                for pair in query.split('&') {
                    if let Some((_, value)) = pair.split_once('=') {
                        values.push(value.to_string());
                    }
                }
            }
            if let Some(segments) = parsed.path_segments() {
                values.extend(
                    segments
                        .filter(|segment| segment.len() >= 8)
                        .map(str::to_string),
                );
            }
        }
    }

    values.extend(
        headers
            .values()
            .filter_map(|value| value.to_str().ok())
            .map(str::to_string),
    );
    values
}

/// Redacts known webhook secrets from error-body previews before returning them to callers.
#[cfg(feature = "webhook")]
fn sanitize_webhook_error_body(body: &str, secret_values: &[String]) -> String {
    const MAX_PREVIEW_CHARS: usize = 512;

    let mut sanitized = body.to_string();
    // Redact longest secrets first so shorter substrings cannot survive replacement.
    let mut secrets: Vec<&String> = secret_values.iter().filter(|s| !s.is_empty()).collect();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    for secret in secrets {
        sanitized = sanitized.replace(secret.as_str(), "[redacted]");
    }

    let normalized = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_PREVIEW_CHARS {
        return normalized;
    }

    let mut preview = normalized
        .chars()
        .take(MAX_PREVIEW_CHARS)
        .collect::<String>();
    preview.push_str("...");
    preview
}

/// Builds webhook headers from configured environment-variable references.
///
/// Header names and values are parsed with reqwest's header types so invalid
/// names, missing variables, or invalid values fail before the request is sent.
#[cfg(feature = "webhook")]
fn webhook_headers(config: &WebhookSinkConfig) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for header in &config.headers {
        let name = HeaderName::from_bytes(header.name.trim().as_bytes())
            .map_err(|error| format!("invalid webhook header name {}: {error}", header.name))?;
        let value = env::var(header.env.trim())
            .map_err(|_| format!("missing environment variable {}", header.env.trim()))?;
        let value = HeaderValue::from_str(&value)
            .map_err(|error| format!("invalid value for header {}: {error}", header.name))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

/// Produces the stable sink label used in delivery status.
fn sink_label(sink: &SinkConfig) -> String {
    sink.name().to_string()
}

/// Creates a failed sink delivery status with a human-readable message.
fn failed_status(sink: String, message: String) -> SinkDeliveryStatus {
    SinkDeliveryStatus {
        sink,
        ok: false,
        message: Some(message),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "webhook")]
    use std::env;
    #[cfg(feature = "webhook")]
    use std::future::Future;
    #[cfg(feature = "webhook")]
    use std::pin::Pin;
    use std::sync::Arc;
    #[cfg(feature = "webhook")]
    use std::sync::Mutex;

    use tempfile::tempdir;

    use crate::config::{
        AppConfig, ChannelConfig, JsonlSinkConfig, LoggingConfig, RuntimeConfig, SinkConfig,
    };
    #[cfg(feature = "webhook")]
    use crate::config::{WebhookHeaderConfig, WebhookSinkConfig};
    use crate::types::VentEvent;

    #[cfg(feature = "webhook")]
    use reqwest::header::HeaderMap;

    use super::SinkDispatcher;
    #[cfg(feature = "webhook")]
    use super::WebhookSender;

    fn runtime_config(config: AppConfig, config_dir: &std::path::Path) -> Arc<RuntimeConfig> {
        Arc::new(
            RuntimeConfig::from_app_config(config, config_dir.to_path_buf())
                .expect("runtime config"),
        )
    }

    #[cfg(feature = "webhook")]
    #[derive(Debug, Clone)]
    struct RecordedWebhook {
        url: String,
        headers: Vec<(String, String)>,
        payload: serde_json::Value,
        timeout_ms: u64,
    }

    #[cfg(feature = "webhook")]
    #[derive(Debug, Default)]
    struct RecordingWebhookSender {
        calls: Arc<Mutex<Vec<RecordedWebhook>>>,
    }

    #[cfg(feature = "webhook")]
    impl WebhookSender for RecordingWebhookSender {
        /// Records webhook request details instead of sending network traffic.
        fn post<'a>(
            &'a self,
            config: &'a WebhookSinkConfig,
            headers: HeaderMap,
            payload: &'a serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
            let calls = self.calls.clone();
            Box::pin(async move {
                let headers = headers
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.as_str().to_string(),
                            value.to_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
                calls.lock().expect("calls lock").push(RecordedWebhook {
                    url: config.url.clone(),
                    headers,
                    payload: payload.clone(),
                    timeout_ms: config.timeout_ms,
                });
                Ok(())
            })
        }
    }

    /// Verifies the JSONL sink writes exactly one parseable event record.
    #[tokio::test]
    async fn jsonl_sink_writes_one_parseable_line() {
        let dir = tempdir().expect("temp dir");
        let config = AppConfig {
            logging: LoggingConfig {
                jsonl_dir: Some(dir.path().to_string_lossy().to_string()),
            },
            ..AppConfig::default()
        };
        let dispatcher = SinkDispatcher::new(runtime_config(config, dir.path()));
        let event = VentEvent::new(
            "feedback".to_string(),
            "This tool could use clearer progress.".to_string(),
            "vent-mcp".to_string(),
        );

        let statuses = dispatcher.dispatch(&event).await;

        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].ok);

        let path = dir.path().join("vents.jsonl");
        let raw = std::fs::read_to_string(path).expect("jsonl file");
        let lines: Vec<_> = raw.lines().collect();
        assert_eq!(lines.len(), 1);
        let decoded: VentEvent = serde_json::from_str(lines[0]).expect("event json");
        assert_eq!(decoded.id, event.id);
        assert_eq!(decoded.project, "vent-mcp");
    }

    /// Verifies channel routes dispatch only to the sinks named by that channel.
    #[tokio::test]
    async fn dispatch_uses_selected_channel_sinks() {
        let dir = tempdir().expect("temp dir");
        let config = AppConfig {
            default_channel: "general".to_string(),
            logging: LoggingConfig {
                jsonl_dir: Some(dir.path().to_string_lossy().to_string()),
            },
            channels: vec![
                ChannelConfig {
                    name: "general".to_string(),
                    description: "General feedback.".to_string(),
                    sinks: vec!["log".to_string()],
                },
                ChannelConfig {
                    name: "ci".to_string(),
                    description: "CI feedback.".to_string(),
                    sinks: vec!["audit".to_string()],
                },
            ],
            sinks: vec![
                SinkConfig::Jsonl(JsonlSinkConfig {
                    name: "log".to_string(),
                }),
                SinkConfig::Jsonl(JsonlSinkConfig {
                    name: "audit".to_string(),
                }),
            ],
            ..AppConfig::default()
        };
        let dispatcher = SinkDispatcher::new(runtime_config(config, dir.path()));
        let event = VentEvent::new(
            "ci".to_string(),
            "The CI logs were hard to correlate.".to_string(),
            "vent-mcp".to_string(),
        );

        let statuses = dispatcher.dispatch(&event).await;

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].sink, "audit");
        assert!(statuses[0].ok);
        assert!(dir.path().join("vents.jsonl").exists());
    }

    /// Verifies webhook sinks send raw event JSON and env-backed headers.
    #[tokio::test]
    #[cfg(feature = "webhook")]
    async fn webhook_sink_sends_json_and_env_backed_header() {
        env::set_var("VENT_MCP_TEST_TOKEN", "secret-token");
        let sender = Arc::new(RecordingWebhookSender::default());

        let config = AppConfig {
            default_channel: "general".to_string(),
            channels: vec![ChannelConfig {
                name: "general".to_string(),
                description: "General feedback.".to_string(),
                sinks: vec!["webhook".to_string()],
            }],
            sinks: vec![SinkConfig::Webhook(WebhookSinkConfig {
                name: "webhook".to_string(),
                url: "https://example.com/vent".to_string(),
                provider: None,
                timeout_ms: 2500,
                headers: vec![WebhookHeaderConfig {
                    name: "X-Test-Token".to_string(),
                    env: "VENT_MCP_TEST_TOKEN".to_string(),
                }],
            })],
            ..AppConfig::default()
        };
        let config_dir = tempdir().expect("temp dir");
        let dispatcher = SinkDispatcher::with_webhook_sender(
            runtime_config(config, config_dir.path()),
            sender.clone(),
        );
        let event = VentEvent::new(
            "general".to_string(),
            "The workflow stalled after a network failure.".to_string(),
            "vent-mcp".to_string(),
        );

        let statuses = dispatcher.dispatch(&event).await;
        env::remove_var("VENT_MCP_TEST_TOKEN");

        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].ok);
        let calls = sender.calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].url, "https://example.com/vent");
        assert_eq!(
            calls[0].payload,
            serde_json::to_value(&event).expect("event json")
        );
        assert_eq!(calls[0].timeout_ms, 2500);
        assert!(calls[0]
            .headers
            .iter()
            .any(|(name, value)| name == "x-test-token" && value == "secret-token"));
    }

    /// Verifies provider mappings shape payloads and add human field labels.
    #[tokio::test]
    #[cfg(feature = "webhook")]
    async fn webhook_provider_maps_event_fields_to_dotted_paths_and_labels() {
        let sender = Arc::new(RecordingWebhookSender::default());

        let config = AppConfig {
            default_channel: "ci".to_string(),
            channels: vec![ChannelConfig {
                name: "ci".to_string(),
                description: "CI feedback.".to_string(),
                sinks: vec!["discord-ci".to_string()],
            }],
            sinks: vec![SinkConfig::Webhook(WebhookSinkConfig {
                name: "discord-ci".to_string(),
                url: "https://example.com/discord".to_string(),
                provider: Some("discord".to_string()),
                timeout_ms: 2500,
                headers: Vec::new(),
            })],
            ..AppConfig::default()
        };
        let config_dir = tempdir().expect("temp dir");
        let dispatcher = SinkDispatcher::with_webhook_sender(
            runtime_config(config, config_dir.path()),
            sender.clone(),
        );
        let event = VentEvent::new(
            "ci".to_string(),
            "The workflow stalled after a network failure.".to_string(),
            "vent-mcp".to_string(),
        );

        let statuses = dispatcher.dispatch(&event).await;

        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].ok);
        let calls = sender.calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].payload["content"], event.message);
        assert_eq!(
            calls[0].payload["embeds"][0]["fields"][0]["name"],
            "Project"
        );
        assert_eq!(
            calls[0].payload["embeds"][0]["fields"][0]["value"],
            "vent-mcp"
        );
        assert!(!calls[0].payload.to_string().contains("ci"));
    }

    /// Verifies non-2xx webhook errors redact known secret values and cap body previews.
    #[test]
    #[cfg(feature = "webhook")]
    fn webhook_status_errors_are_sanitized_and_short() {
        let secret = "secret-token-value".to_string();
        let long_body = format!("request failed for secret-token-value\n{}", "x".repeat(700));

        let message = super::webhook_status_error(
            reqwest::StatusCode::BAD_GATEWAY,
            Some(&long_body),
            &[secret],
        );

        assert!(message.starts_with("webhook HTTP 502 Bad Gateway: "));
        assert!(message.contains("[redacted]"));
        assert!(!message.contains("secret-token-value"));
        assert!(message.len() < 600);
    }

    /// Short URL and header secrets are still redacted from webhook error previews.
    #[test]
    #[cfg(feature = "webhook")]
    fn webhook_status_errors_redact_short_secrets() {
        let secrets = vec!["abc".to_string(), "xy".to_string(), String::new()];
        let message = super::webhook_status_error(
            reqwest::StatusCode::UNAUTHORIZED,
            Some("invalid token abc for header xy"),
            &secrets,
        );

        assert!(!message.contains("abc"));
        assert!(!message.contains("xy"));
        assert!(message.contains("[redacted]"));
    }
}
