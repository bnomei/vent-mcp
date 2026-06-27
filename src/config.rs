//! Configuration loading, defaults, and validation for vent-mcp.
//!
//! Configuration is the main policy boundary for the server. It decides which
//! channels are valid, where events may be delivered, how JSONL output is placed,
//! and how webhook payloads can be shaped. This module deliberately validates
//! those choices before the server starts or the CLI sends a message, so callers
//! cannot route feedback to undeclared channels, malformed webhook destinations,
//! or ambiguous provider mappings.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::provider::ProviderTemplate;

const DEFAULT_CONFIG_FILE_NAME: &str = "config.toml";
const DEFAULT_CONFIG_DIR_NAME: &str = "vent-mcp";
pub(crate) const DEFAULT_LOG_SINK_NAME: &str = "log";
const ENV_CONFIG: &str = "VENT_MCP_CONFIG";
#[cfg(feature = "webhook")]
const DEFAULT_WEBHOOK_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    path: PathBuf,
    config: RuntimeConfig,
}

impl LoadedConfig {
    /// Resolves, creates when appropriate, reads, and validates the active config.
    ///
    /// Environment-specified configs must already exist, while default user
    /// locations can be bootstrapped with the built-in safe defaults.
    pub fn load() -> Result<Self, ConfigError> {
        let resolved = resolve_config_path()?;
        Self::load_from_resolved_path(resolved)
    }

    /// Loads and validates a config from an explicit path.
    ///
    /// This bypasses automatic default creation and is used by tests and callers
    /// that already know which config file should be authoritative.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref().to_path_buf();
        let config = AppConfig::load_from_path(&path)?;
        Self::from_app_config(path, config)
    }

    /// Loads a config from a previously resolved path, creating defaults when safe.
    ///
    /// Default creation happens only for implicit XDG or home-directory paths.
    /// An explicit environment override is treated as intentional and therefore
    /// fails when the file is missing.
    fn load_from_resolved_path(resolved: ResolvedConfigPath) -> Result<Self, ConfigError> {
        if !resolved.path.exists() {
            if resolved.source == ConfigPathSource::Env {
                return Err(ConfigError::NotFound {
                    path: resolved.path,
                });
            }
            write_default_config(&resolved.path)?;
        }

        Self::load_from_path(resolved.path)
    }

    fn from_app_config(path: PathBuf, config: AppConfig) -> Result<Self, ConfigError> {
        let config_dir = config_dir_for_path(&path);
        let config = RuntimeConfig::from_app_config(config, config_dir)?;
        Ok(Self { path, config })
    }

    /// Returns the normalized runtime configuration.
    #[must_use]
    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Consumes the loaded config and returns its normalized runtime form.
    #[must_use]
    pub fn into_config(self) -> RuntimeConfig {
        self.config
    }

    /// Returns the directory that contains the active configuration file.
    ///
    /// Sinks use this as the base for default relative storage decisions.
    #[must_use]
    pub fn config_dir(&self) -> PathBuf {
        config_dir_for_path(&self.path)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfig {
    default_channel: String,
    logging: LoggingConfig,
    channels: Vec<ChannelConfig>,
    #[cfg(feature = "webhook")]
    providers: BTreeMap<String, ProviderTemplate>,
    sinks: Vec<SinkConfig>,
    sinks_by_name: BTreeMap<String, usize>,
    config_dir: PathBuf,
    jsonl_dir: PathBuf,
}

impl RuntimeConfig {
    pub(crate) fn from_app_config(
        config: AppConfig,
        config_dir: PathBuf,
    ) -> Result<Self, ConfigValidationError> {
        config.validate()?;
        let AppConfig {
            default_channel,
            logging,
            channels,
            providers: raw_providers,
            sinks,
        } = config;

        #[cfg(feature = "webhook")]
        let providers = compile_webhook_providers(&raw_providers)?;
        #[cfg(not(feature = "webhook"))]
        let _ = raw_providers;

        let sinks_by_name = sinks
            .iter()
            .enumerate()
            .map(|(index, sink)| (sink.name().to_string(), index))
            .collect();
        let jsonl_dir = resolve_jsonl_dir(&logging, &config_dir);

        Ok(Self {
            default_channel,
            logging,
            channels,
            #[cfg(feature = "webhook")]
            providers,
            sinks,
            sinks_by_name,
            config_dir,
            jsonl_dir,
        })
    }

    #[must_use]
    pub fn default_channel(&self) -> &str {
        &self.default_channel
    }

    #[must_use]
    pub fn has_channel(&self, name: &str) -> bool {
        self.channels.iter().any(|channel| channel.name == name)
    }

    #[must_use]
    pub fn has_only_default_channel(&self) -> bool {
        self.channels.len() == 1 && self.channels[0].name == self.default_channel
    }

    #[must_use]
    pub fn channel_list(&self) -> crate::types::ListChannelsOutput {
        crate::types::ListChannelsOutput {
            default_channel: self.default_channel.clone(),
            channels: self
                .channels
                .iter()
                .map(|channel| crate::types::ChannelInfo {
                    name: channel.name.clone(),
                    description: channel.description.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn sinks_for_channel(&self, channel_name: &str) -> Option<Vec<&SinkConfig>> {
        let channel = self
            .channels
            .iter()
            .find(|channel| channel.name == channel_name)?;
        Some(
            channel
                .sinks
                .iter()
                .map(|sink| {
                    let index = self
                        .sinks_by_name
                        .get(sink)
                        .expect("validated channel sink reference");
                    &self.sinks[*index]
                })
                .collect(),
        )
    }

    pub(crate) fn jsonl_dir(&self) -> &Path {
        &self.jsonl_dir
    }

    #[cfg(feature = "webhook")]
    pub(crate) fn webhook_provider(&self, name: &str) -> Option<&ProviderTemplate> {
        self.providers.get(name)
    }

    #[allow(dead_code)]
    pub(crate) fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    #[allow(dead_code)]
    pub(crate) fn logging(&self) -> &LoggingConfig {
        &self.logging
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub default_channel: String,
    pub logging: LoggingConfig,
    pub channels: Vec<ChannelConfig>,
    pub providers: BTreeMap<String, WebhookProviderConfig>,
    pub sinks: Vec<SinkConfig>,
}

impl Default for AppConfig {
    /// Builds the conservative default config with one channel and JSONL logging.
    ///
    /// The default creates a local, non-network sink so a first run has somewhere
    /// to record vents without requiring webhook credentials or provider setup.
    fn default() -> Self {
        Self {
            default_channel: "feedback".to_string(),
            logging: LoggingConfig::default(),
            channels: vec![ChannelConfig {
                name: "feedback".to_string(),
                description: "Blocked work, repeated failures, or confusing workflows. Avoid routine progress updates.".to_string(),
                sinks: vec![DEFAULT_LOG_SINK_NAME.to_string()],
            }],
            providers: default_webhook_providers(),
            sinks: vec![default_log_sink()],
        }
    }
}

impl AppConfig {
    /// Reads, parses, and validates TOML configuration from disk.
    ///
    /// The parsed value is never returned before validation succeeds, keeping
    /// downstream server and sink code free from partial-config assumptions.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(ConfigError::NotFound {
                path: path.to_path_buf(),
            });
        }

        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;

        let config: Self = toml::from_str(&raw).map_err(|source| ConfigError::ParseTomlAtPath {
            path: path.to_path_buf(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Parses and validates TOML configuration from a string.
    ///
    /// This is used by config tests without exposing raw parsing as binary API.
    #[cfg(test)]
    pub fn from_toml_str(raw: &str) -> Result<Self, ConfigError> {
        let config: Self =
            toml::from_str(raw).map_err(|source| ConfigError::ParseToml { source })?;
        config.validate()?;
        Ok(config)
    }

    /// Validates channel, provider, and sink settings as one coherent policy.
    ///
    /// The checks reject empty lists, duplicate or malformed channel names,
    /// default channels that do not exist, invalid provider maps, and sink
    /// settings that would fail later in less predictable ways.
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        validate_channel_name(&self.default_channel, "default_channel")?;

        if self.channels.is_empty() {
            return Err(ConfigValidationError::ChannelsMustNotBeEmpty);
        }
        if self.sinks.is_empty() {
            return Err(ConfigValidationError::SinksMustNotBeEmpty);
        }

        let mut channel_names = BTreeSet::new();
        for channel in &self.channels {
            validate_channel_name(&channel.name, "channels.name")?;
            if channel.description.trim().is_empty() {
                return Err(ConfigValidationError::ChannelDescriptionMustNotBeEmpty {
                    channel: channel.name.clone(),
                });
            }
            if channel.sinks.is_empty() {
                return Err(ConfigValidationError::ChannelSinksMustNotBeEmpty {
                    channel: channel.name.clone(),
                });
            }
            let mut channel_sinks = BTreeSet::new();
            for sink in &channel.sinks {
                validate_provider_name(sink, "channels.sinks")?;
                if !channel_sinks.insert(sink.as_str()) {
                    return Err(ConfigValidationError::DuplicateChannelSink {
                        channel: channel.name.clone(),
                        sink: sink.clone(),
                    });
                }
            }
            if !channel_names.insert(channel.name.as_str()) {
                return Err(ConfigValidationError::DuplicateChannel {
                    channel: channel.name.clone(),
                });
            }
        }

        if !channel_names.contains(self.default_channel.as_str()) {
            return Err(ConfigValidationError::DefaultChannelMustExist {
                channel: self.default_channel.clone(),
            });
        }

        for (provider_name, provider) in &self.providers {
            validate_provider_name(provider_name, "providers")?;
            provider.validate(provider_name)?;
        }

        let mut sink_names = BTreeSet::new();
        for sink in &self.sinks {
            sink.validate(&self.providers)?;
            let name = sink.name();
            validate_provider_name(name, "sinks.name")?;
            if !sink_names.insert(name) {
                return Err(ConfigValidationError::DuplicateSinkName {
                    sink: name.to_string(),
                });
            }
        }

        for channel in &self.channels {
            for sink in &channel.sinks {
                if !sink_names.contains(sink.as_str()) {
                    return Err(ConfigValidationError::UnknownChannelSink {
                        channel: channel.name.clone(),
                        sink: sink.clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub jsonl_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub sinks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum SinkConfig {
    Jsonl(JsonlSinkConfig),
    #[cfg(feature = "webhook")]
    Webhook(WebhookSinkConfig),
}

impl SinkConfig {
    /// Validates the concrete sink variant against the provider registry.
    ///
    /// JSONL has no extra settings, while webhook sinks must reference only
    /// defined providers and pass URL, timeout, and header checks.
    fn validate(
        &self,
        _providers: &BTreeMap<String, WebhookProviderConfig>,
    ) -> Result<(), ConfigValidationError> {
        match self {
            SinkConfig::Jsonl(_) => Ok(()),
            #[cfg(feature = "webhook")]
            SinkConfig::Webhook(config) => config.validate(_providers),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            SinkConfig::Jsonl(config) => &config.name,
            #[cfg(feature = "webhook")]
            SinkConfig::Webhook(config) => &config.name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct JsonlSinkConfig {
    pub name: String,
}

#[cfg(feature = "webhook")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WebhookSinkConfig {
    pub name: String,
    pub url: String,
    pub provider: Option<String>,
    pub headers: Vec<WebhookHeaderConfig>,
    pub timeout_ms: u64,
}

#[cfg(feature = "webhook")]
impl Default for WebhookSinkConfig {
    /// Builds a webhook sink with empty destination and default timeout.
    ///
    /// The URL remains intentionally empty so deserialization can fill it and
    /// validation can reject configs that omit a real endpoint.
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            provider: None,
            headers: Vec::new(),
            timeout_ms: DEFAULT_WEBHOOK_TIMEOUT_MS,
        }
    }
}

#[cfg(feature = "webhook")]
impl WebhookSinkConfig {
    /// Validates one webhook sink and any provider reference it contains.
    ///
    /// Webhooks must target HTTP(S), have a positive timeout, use non-empty
    /// environment-backed headers, and either request raw event JSON or a known
    /// provider mapping.
    fn validate(
        &self,
        providers: &BTreeMap<String, WebhookProviderConfig>,
    ) -> Result<(), ConfigValidationError> {
        if self.url.trim().is_empty() {
            return Err(ConfigValidationError::WebhookUrlMustNotBeEmpty);
        }

        let parsed = url::Url::parse(&self.url).map_err(|_| {
            ConfigValidationError::WebhookUrlMustBeHttp {
                url: self.url.clone(),
            }
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ConfigValidationError::WebhookUrlMustBeHttp {
                url: self.url.clone(),
            });
        }
        if self.timeout_ms == 0 {
            return Err(ConfigValidationError::WebhookTimeoutMsMustBePositive);
        }
        if let Some(provider) = self.provider.as_deref().map(str::trim) {
            if provider.is_empty() {
                return Err(ConfigValidationError::WebhookProviderNameMustNotBeEmpty);
            }
            validate_provider_name(provider, "sinks.provider")?;
            if provider != "raw" && !providers.contains_key(provider) {
                return Err(ConfigValidationError::UnknownWebhookProvider {
                    provider: provider.to_string(),
                });
            }
        }

        let mut header_names = BTreeSet::new();
        for header in &self.headers {
            if header.name.trim().is_empty() {
                return Err(ConfigValidationError::WebhookHeaderNameMustNotBeEmpty);
            }
            if header.env.trim().is_empty() {
                return Err(ConfigValidationError::WebhookHeaderEnvMustNotBeEmpty {
                    header: header.name.clone(),
                });
            }
            let normalized = header.name.trim().to_ascii_lowercase();
            if !header_names.insert(normalized) {
                return Err(ConfigValidationError::DuplicateWebhookHeader {
                    header: header.name.clone(),
                });
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct WebhookProviderConfig {
    pub field_label_key: Option<String>,
    #[serde(flatten)]
    pub fields: BTreeMap<String, String>,
}

impl WebhookProviderConfig {
    /// Validates a provider's event-field-to-output-path mapping.
    ///
    /// Providers are constrained to known event fields and unique dotted output
    /// paths so rendered webhook payloads cannot collide or silently drop fields.
    fn validate(&self, provider_name: &str) -> Result<(), ConfigValidationError> {
        ProviderTemplate::compile(provider_name, self).map(|_| ())
    }
}

#[cfg(feature = "webhook")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebhookHeaderConfig {
    pub name: String,
    pub env: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfigPath {
    pub path: PathBuf,
    pub source: ConfigPathSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPathSource {
    Env,
    Xdg,
    Home,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("home directory is not set")]
    HomeDirectoryNotSet,
    #[error("config file not found at {path}")]
    NotFound { path: PathBuf },
    #[error("failed to read config file at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config file at {path}: {source}")]
    ParseTomlAtPath {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to parse config: {source}")]
    ParseToml { source: toml::de::Error },
    #[error("{0}")]
    Validation(#[from] ConfigValidationError),
    #[error("failed to create config directory at {path}: {source}")]
    CreateConfigDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize default config: {source}")]
    SerializeDefaultConfig { source: toml::ser::Error },
    #[error("failed to write default config at {path}: {source}")]
    WriteDefaultConfig {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigValidationError {
    #[error("channels must contain at least one channel")]
    ChannelsMustNotBeEmpty,
    #[error("sinks must contain at least one sink")]
    SinksMustNotBeEmpty,
    #[error("invalid channel name in {field}: {name}")]
    InvalidChannelName { field: String, name: String },
    #[error("duplicate sink name: {sink}")]
    DuplicateSinkName { sink: String },
    #[error("duplicate channel: {channel}")]
    DuplicateChannel { channel: String },
    #[error("channel must reference at least one sink: {channel}")]
    ChannelSinksMustNotBeEmpty { channel: String },
    #[error("duplicate sink reference in channel {channel}: {sink}")]
    DuplicateChannelSink { channel: String, sink: String },
    #[error("unknown sink reference in channel {channel}: {sink}")]
    UnknownChannelSink { channel: String, sink: String },
    #[error("default channel does not exist: {channel}")]
    DefaultChannelMustExist { channel: String },
    #[error("channel description must not be empty: {channel}")]
    ChannelDescriptionMustNotBeEmpty { channel: String },
    #[error("webhook url must not be empty")]
    WebhookUrlMustNotBeEmpty,
    #[error("webhook url must be an http or https URL: {url}")]
    WebhookUrlMustBeHttp { url: String },
    #[cfg(feature = "webhook")]
    #[error("webhook timeout_ms must be positive")]
    WebhookTimeoutMsMustBePositive,
    #[cfg(feature = "webhook")]
    #[error("webhook header name must not be empty")]
    WebhookHeaderNameMustNotBeEmpty,
    #[cfg(feature = "webhook")]
    #[error("webhook header env must not be empty for header {header}")]
    WebhookHeaderEnvMustNotBeEmpty { header: String },
    #[cfg(feature = "webhook")]
    #[error("duplicate webhook header: {header}")]
    DuplicateWebhookHeader { header: String },
    #[error("webhook provider name must not be empty")]
    WebhookProviderNameMustNotBeEmpty,
    #[error("unknown webhook provider: {provider}")]
    UnknownWebhookProvider { provider: String },
    #[error("webhook provider map must not be empty: {provider}")]
    WebhookProviderMapMustNotBeEmpty { provider: String },
    #[error("invalid webhook provider label key in {provider}: {label_key}")]
    InvalidWebhookProviderLabelKey { provider: String, label_key: String },
    #[error("unknown webhook provider field in {provider}: {field}")]
    UnknownWebhookProviderField { provider: String, field: String },
    #[error("invalid webhook provider path in {provider} for {field}: {path}")]
    InvalidWebhookProviderPath {
        provider: String,
        field: String,
        path: String,
    },
    #[error("duplicate webhook provider output path in {provider}: {path}")]
    DuplicateWebhookProviderPath { provider: String, path: String },
    #[error("webhook provider output path in {provider} collides with another mapped path: {path}")]
    CollidingWebhookProviderPath { provider: String, path: String },
}

/// Resolves the active configuration path from process environment.
///
/// The lookup order is explicit config path, XDG config home, then the user's
/// home directory. Empty environment values are ignored rather than treated as
/// real paths.
pub fn resolve_config_path() -> Result<ResolvedConfigPath, ConfigError> {
    resolve_config_path_with(
        |key| env::var_os(key),
        env::var_os("HOME").map(PathBuf::from),
    )
}

/// Resolves a config path using injectable environment and home-directory inputs.
///
/// This helper keeps path precedence testable without mutating global process
/// environment. The returned source records why the path was selected so loading
/// can decide whether default creation is allowed.
pub fn resolve_config_path_with<F>(
    lookup_var: F,
    home_dir: Option<PathBuf>,
) -> Result<ResolvedConfigPath, ConfigError>
where
    F: Fn(&str) -> Option<OsString>,
{
    if let Some(path) = lookup_var(ENV_CONFIG).and_then(non_empty_os_string) {
        return Ok(ResolvedConfigPath {
            path: PathBuf::from(path),
            source: ConfigPathSource::Env,
        });
    }

    if let Some(xdg_config_home) = lookup_var("XDG_CONFIG_HOME").and_then(non_empty_os_string) {
        return Ok(ResolvedConfigPath {
            path: PathBuf::from(xdg_config_home)
                .join(DEFAULT_CONFIG_DIR_NAME)
                .join(DEFAULT_CONFIG_FILE_NAME),
            source: ConfigPathSource::Xdg,
        });
    }

    let home = home_dir.ok_or(ConfigError::HomeDirectoryNotSet)?;
    Ok(ResolvedConfigPath {
        path: home
            .join(".config")
            .join(DEFAULT_CONFIG_DIR_NAME)
            .join(DEFAULT_CONFIG_FILE_NAME),
        source: ConfigPathSource::Home,
    })
}

/// Treats empty OS strings as unset environment values.
fn non_empty_os_string(value: OsString) -> Option<OsString> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn config_dir_for_path(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_jsonl_dir(logging: &LoggingConfig, config_dir: &Path) -> PathBuf {
    logging
        .jsonl_dir
        .as_deref()
        .map(str::trim)
        // An empty or whitespace-only value behaves like omission rather than
        // resolving to the empty (CWD-relative) path; mirrors how empty env
        // values are ignored via `non_empty_os_string`.
        .filter(|value| !value.is_empty())
        .map(expand_tilde)
        .unwrap_or_else(|| config_dir.to_path_buf())
}

fn expand_tilde(value: &str) -> PathBuf {
    if value == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(value));
    }

    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }

    Path::new(value).to_path_buf()
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

#[cfg(feature = "webhook")]
fn compile_webhook_providers(
    providers: &BTreeMap<String, WebhookProviderConfig>,
) -> Result<BTreeMap<String, ProviderTemplate>, ConfigValidationError> {
    providers
        .iter()
        .map(|(name, provider)| {
            ProviderTemplate::compile(name, provider).map(|template| (name.clone(), template))
        })
        .collect()
}

/// Validates channel-like names used by configuration and sink selection.
///
/// Names are limited to lowercase ASCII letters, digits, underscores, and dashes
/// so they remain stable in CLI input, MCP schemas, status labels, and webhook
/// provider references.
fn validate_channel_name(name: &str, field: &str) -> Result<(), ConfigValidationError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'));

    if valid {
        Ok(())
    } else {
        Err(ConfigValidationError::InvalidChannelName {
            field: field.to_string(),
            name: name.to_string(),
        })
    }
}

/// Validates provider names with the same rules used for channels.
fn validate_provider_name(name: &str, field: &str) -> Result<(), ConfigValidationError> {
    validate_channel_name(name, field)
}

#[must_use]
pub(crate) fn default_log_sink() -> SinkConfig {
    SinkConfig::Jsonl(JsonlSinkConfig {
        name: DEFAULT_LOG_SINK_NAME.to_string(),
    })
}

/// Supplies built-in webhook provider mappings for common receivers.
///
/// These defaults let users send useful payloads to raw automation endpoints,
/// text-only chat endpoints, and rich chat endpoints without writing the mapping
/// from scratch.
fn default_webhook_providers() -> BTreeMap<String, WebhookProviderConfig> {
    [
        ("zapier", raw_event_provider()),
        ("make", raw_event_provider()),
        ("n8n", raw_event_provider()),
        ("pipedream", raw_event_provider()),
        ("workato", raw_event_provider()),
        ("ifttt", ifttt_provider()),
        ("slack", slack_like_provider()),
        ("mattermost", slack_like_provider()),
        (
            "discord",
            labeled_context_provider("name", "content", "embeds.0.fields.0.value"),
        ),
        ("microsoft_teams", text_provider("text")),
        ("google_chat", text_provider("text")),
        ("webex", text_provider("markdown")),
    ]
    .into_iter()
    .map(|(name, provider)| (name.to_string(), provider))
    .collect()
}

/// Maps the canonical vent event shape unchanged into a receiver-specific body.
fn raw_event_provider() -> WebhookProviderConfig {
    WebhookProviderConfig {
        field_label_key: None,
        fields: BTreeMap::from([
            ("id".to_string(), "id".to_string()),
            ("timestamp".to_string(), "timestamp".to_string()),
            ("channel".to_string(), "channel".to_string()),
            ("message".to_string(), "message".to_string()),
            ("project".to_string(), "project".to_string()),
        ]),
    }
}

/// Maps only the vent message into a provider's plain text field.
fn text_provider(message_path: &str) -> WebhookProviderConfig {
    WebhookProviderConfig {
        field_label_key: None,
        fields: BTreeMap::from([("message".to_string(), message_path.to_string())]),
    }
}

/// Maps message and project into rich fields with generated labels.
fn labeled_context_provider(
    label_key: &str,
    message_path: &str,
    project_path: &str,
) -> WebhookProviderConfig {
    WebhookProviderConfig {
        field_label_key: Some(label_key.to_string()),
        fields: BTreeMap::from([
            ("message".to_string(), message_path.to_string()),
            ("project".to_string(), project_path.to_string()),
        ]),
    }
}

/// Maps into Slack-compatible attachment fields.
fn slack_like_provider() -> WebhookProviderConfig {
    labeled_context_provider("title", "text", "attachments.0.fields.0.value")
}

/// Maps the three Maker Webhooks values IFTTT exposes to applets.
fn ifttt_provider() -> WebhookProviderConfig {
    WebhookProviderConfig {
        field_label_key: None,
        fields: BTreeMap::from([
            ("message".to_string(), "value1".to_string()),
            ("channel".to_string(), "value2".to_string()),
            ("project".to_string(), "value3".to_string()),
        ]),
    }
}

/// Writes a default config file at an implicit config path.
///
/// Parent directories are created as needed, and serialization errors remain
/// explicit so startup failures explain whether creation or rendering failed.
fn write_default_config(path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::CreateConfigDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let rendered = toml::to_string_pretty(&AppConfig::default())
        .map_err(|source| ConfigError::SerializeDefaultConfig { source })?;
    fs::write(path, rendered).map_err(|source| ConfigError::WriteDefaultConfig {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use tempfile::tempdir;

    #[cfg(feature = "webhook")]
    use super::SinkConfig;
    use super::{
        resolve_config_path_with, AppConfig, ConfigPathSource, ConfigValidationError, LoadedConfig,
    };

    /// Verifies config path lookup precedence without touching real environment variables.
    #[test]
    fn resolve_config_path_uses_expected_precedence() {
        let from_env = resolve_config_path_with(
            |name| match name {
                "VENT_MCP_CONFIG" => Some(OsString::from("/tmp/override.toml")),
                "XDG_CONFIG_HOME" => Some(OsString::from("/xdg")),
                _ => None,
            },
            Some(PathBuf::from("/Users/alice")),
        )
        .expect("env override should resolve");
        assert_eq!(from_env.path, PathBuf::from("/tmp/override.toml"));
        assert_eq!(from_env.source, ConfigPathSource::Env);

        let from_xdg = resolve_config_path_with(
            |name| match name {
                "XDG_CONFIG_HOME" => Some(OsString::from("/xdg")),
                _ => None,
            },
            Some(PathBuf::from("/Users/alice")),
        )
        .expect("xdg should resolve");
        assert_eq!(from_xdg.path, PathBuf::from("/xdg/vent-mcp/config.toml"));
        assert_eq!(from_xdg.source, ConfigPathSource::Xdg);

        let from_home = resolve_config_path_with(|_| None, Some(PathBuf::from("/Users/alice")))
            .expect("home should resolve");
        assert_eq!(
            from_home.path,
            PathBuf::from("/Users/alice/.config/vent-mcp/config.toml")
        );
        assert_eq!(from_home.source, ConfigPathSource::Home);
    }

    /// Verifies validation requires the default channel to be declared.
    #[test]
    fn config_validation_rejects_missing_default_channel() {
        let raw = r#"
default_channel = "missing"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[[sinks]]
type = "jsonl"
name = "log"
"#;

        let error = AppConfig::from_toml_str(raw).expect_err("config should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::DefaultChannelMustExist { .. })
        ));
    }

    /// Verifies duplicate channel names are rejected.
    #[test]
    fn config_validation_rejects_duplicate_channels() {
        let raw = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[[channels]]
name = "general"
description = "Duplicate feedback."
sinks = ["log"]

[[sinks]]
type = "jsonl"
name = "log"
"#;

        let error = AppConfig::from_toml_str(raw).expect_err("config should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::DuplicateChannel { .. })
        ));
    }

    /// Verifies configs must define at least one sink.
    #[test]
    fn config_validation_rejects_empty_sinks() {
        let raw = r#"
default_channel = "general"
sinks = []

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]
"#;

        let error = AppConfig::from_toml_str(raw).expect_err("config should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::SinksMustNotBeEmpty)
        ));
    }

    /// Verifies every channel must explicitly route to at least one sink.
    #[test]
    fn config_validation_rejects_channel_without_sinks() {
        let raw = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."

[[sinks]]
type = "jsonl"
name = "log"
"#;

        let error = AppConfig::from_toml_str(raw).expect_err("config should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::ChannelSinksMustNotBeEmpty { .. })
        ));
    }

    /// Verifies channel routes must reference known sink definitions.
    #[test]
    fn config_validation_rejects_unknown_channel_sink() {
        let raw = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["missing"]

[[sinks]]
type = "jsonl"
name = "log"
"#;

        let error = AppConfig::from_toml_str(raw).expect_err("config should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::UnknownChannelSink { .. })
        ));
    }

    /// Verifies named sinks are validated and cannot collide.
    #[test]
    fn config_validation_rejects_duplicate_sink_names() {
        let raw = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[[sinks]]
type = "jsonl"
name = "log"

[[sinks]]
type = "jsonl"
name = "log"
"#;

        let error = AppConfig::from_toml_str(raw).expect_err("config should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::DuplicateSinkName { .. })
        ));
    }

    /// Verifies sink definitions must have explicit names for channel routing.
    #[test]
    fn config_validation_rejects_unnamed_sinks() {
        let raw = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[[sinks]]
type = "jsonl"
"#;

        let error = AppConfig::from_toml_str(raw).expect_err("config should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::InvalidChannelName { field, .. })
                if field == "sinks.name"
        ));
    }

    /// Verifies webhook TOML parses headers, provider references, and timeout values.
    #[test]
    #[cfg(feature = "webhook")]
    fn config_parses_webhook_with_env_headers() {
        let raw = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["slack"]

[[sinks]]
type = "webhook"
name = "slack"
url = "https://example.com/vent"
provider = "slack"
timeout_ms = 2500
headers = [
  { name = "Authorization", env = "VENT_MCP_WEBHOOK_AUTHORIZATION" },
]
"#;

        let config = AppConfig::from_toml_str(raw).expect("webhook config");
        assert_eq!(config.sinks.len(), 1);
        let SinkConfig::Webhook(webhook) = &config.sinks[0] else {
            panic!("expected webhook sink");
        };
        assert_eq!(webhook.provider.as_deref(), Some("slack"));
        assert_eq!(webhook.timeout_ms, 2500);
    }

    /// Verifies webhook timeouts default to a positive value and reject zero.
    #[test]
    #[cfg(feature = "webhook")]
    fn webhook_timeout_defaults_and_must_be_positive() {
        let raw = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["webhook"]

[[sinks]]
type = "webhook"
name = "webhook"
url = "https://example.com/vent"
"#;

        let config = AppConfig::from_toml_str(raw).expect("webhook config");
        let SinkConfig::Webhook(webhook) = &config.sinks[0] else {
            panic!("expected webhook sink");
        };
        assert_eq!(webhook.timeout_ms, super::DEFAULT_WEBHOOK_TIMEOUT_MS);

        let invalid = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["webhook"]

[[sinks]]
type = "webhook"
name = "webhook"
url = "https://example.com/vent"
timeout_ms = 0
"#;
        let error = AppConfig::from_toml_str(invalid).expect_err("timeout should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::WebhookTimeoutMsMustBePositive)
        ));
    }

    /// Verifies provider mappings must reference known fields, paths, and providers.
    #[test]
    #[cfg(feature = "webhook")]
    fn webhook_provider_config_validates_sink_references_and_paths() {
        let raw = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["webhook"]

[providers.custom]
message = "data.content.message"
channel = "data.content.channel"

[[sinks]]
type = "webhook"
name = "webhook"
url = "https://example.com/vent"
provider = "custom"
"#;

        let config = AppConfig::from_toml_str(raw).expect("provider config");
        assert!(config.providers.contains_key("custom"));

        let missing = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["webhook"]

[[sinks]]
type = "webhook"
name = "webhook"
url = "https://example.com/vent"
provider = "missing"
"#;
        let error = AppConfig::from_toml_str(missing).expect_err("provider should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::UnknownWebhookProvider { .. })
        ));

        let invalid_field = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[providers.custom]
unknown = "data.content"

[[sinks]]
type = "jsonl"
name = "log"
"#;
        let error = AppConfig::from_toml_str(invalid_field).expect_err("field should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::UnknownWebhookProviderField { .. })
        ));

        let invalid_path = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[providers.custom]
message = "data..content"

[[sinks]]
type = "jsonl"
name = "log"
"#;
        let error = AppConfig::from_toml_str(invalid_path).expect_err("path should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::InvalidWebhookProviderPath { .. })
        ));

        // A parent path and one of its descendants must be rejected at load: the
        // shorter path's leaf insert would overwrite the nested field's container
        // and silently drop a mapped field on successful webhook delivery.
        let colliding_path = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[providers.bad]
message = "payload.body"
project = "payload"

[[sinks]]
type = "jsonl"
name = "log"
"#;
        let error =
            AppConfig::from_toml_str(colliding_path).expect_err("colliding path should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::CollidingWebhookProviderPath { .. })
        ));

        // Unbounded array indices must be rejected at load: at render time they
        // would `Vec::resize` to the index (OOM) or overflow `index + 1` at
        // `usize::MAX` and panic the process.
        let oversized_index = r#"
default_channel = "general"

[[channels]]
name = "general"
description = "General feedback."
sinks = ["log"]

[providers.bad]
message = "items.5000000"

[[sinks]]
type = "jsonl"
name = "log"
"#;
        let error =
            AppConfig::from_toml_str(oversized_index).expect_err("oversized index should fail");
        assert!(matches!(
            error.into_validation(),
            Some(ConfigValidationError::InvalidWebhookProviderPath { .. })
        ));
    }

    /// Verifies empty/whitespace `jsonl_dir` falls back to the config directory
    /// instead of resolving to the CWD-relative empty path.
    #[test]
    fn empty_jsonl_dir_falls_back_to_config_dir() {
        let config_dir = PathBuf::from("/home/user/.config/vent-mcp");

        for value in [None, Some(String::new()), Some("   ".to_string())] {
            let logging = super::LoggingConfig { jsonl_dir: value };
            assert_eq!(
                super::resolve_jsonl_dir(&logging, &config_dir),
                config_dir,
                "empty/omitted jsonl_dir should anchor to the config directory"
            );
        }

        let explicit = super::LoggingConfig {
            jsonl_dir: Some("/var/log/vent".to_string()),
        };
        assert_eq!(
            super::resolve_jsonl_dir(&explicit, &config_dir),
            PathBuf::from("/var/log/vent")
        );
    }

    /// Verifies implicit default config paths are created on first load.
    #[test]
    fn load_from_default_path_creates_default_config() {
        let dir = tempdir().expect("temp dir");
        let path = dir.path().join("vent-mcp").join("config.toml");

        let loaded = LoadedConfig::load_from_resolved_path(super::ResolvedConfigPath {
            path: path.clone(),
            source: ConfigPathSource::Home,
        })
        .expect("default config should be created");

        assert!(path.exists());
        assert_eq!(loaded.config().default_channel(), "feedback");
    }

    /// Verifies the default config ships the broad webhook provider set.
    #[test]
    fn default_config_includes_common_webhook_provider_maps() {
        let config = AppConfig::default();
        let expected = [
            "zapier",
            "make",
            "n8n",
            "pipedream",
            "workato",
            "ifttt",
            "slack",
            "mattermost",
            "discord",
            "microsoft_teams",
            "google_chat",
            "webex",
        ];

        for provider in expected {
            assert!(
                config.providers.contains_key(provider),
                "missing default provider {provider}"
            );
        }
        assert_eq!(
            config.providers["microsoft_teams"].fields["message"],
            "text"
        );
        assert_eq!(config.providers["ifttt"].fields["message"], "value1");
        assert_eq!(config.channels[0].sinks, [super::DEFAULT_LOG_SINK_NAME]);
        assert_eq!(config.sinks[0].name(), super::DEFAULT_LOG_SINK_NAME);
        assert!(!config.providers["discord"].fields.contains_key("channel"));
        assert!(!config.providers["slack"].fields.contains_key("channel"));
    }

    trait ConfigErrorExt {
        /// Extracts validation errors from the top-level config error wrapper.
        fn into_validation(self) -> Option<ConfigValidationError>;
    }

    impl ConfigErrorExt for super::ConfigError {
        /// Returns the inner validation error when this is a validation failure.
        fn into_validation(self) -> Option<ConfigValidationError> {
            match self {
                super::ConfigError::Validation(error) => Some(error),
                _ => None,
            }
        }
    }
}
