//! Webhook provider template validation and rendering.
//!
//! TOML keeps provider mappings as strings, but runtime delivery uses compiled
//! paths so each webhook send does only JSON rendering, not path validation.

#[cfg(feature = "webhook")]
use serde_json::{Map, Value};

use crate::config::{ConfigValidationError, WebhookProviderConfig};
#[cfg(feature = "webhook")]
use crate::config::{RuntimeConfig, WebhookSinkConfig};
#[cfg(feature = "webhook")]
use crate::types::VentEvent;

/// Upper bound for numeric array indices in provider output paths.
///
/// Indices are materialized at render time via `Vec::resize(index + 1, ..)`, so
/// an unbounded index would let a loaded config OOM or panic the process. The
/// cap is generous relative to built-in providers (which only use index `0`)
/// while keeping the largest possible array small.
const MAX_PROVIDER_ARRAY_INDEX: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderTemplate {
    field_label_key: Option<String>,
    fields: Vec<ProviderFieldMapping>,
}

impl ProviderTemplate {
    pub(crate) fn compile(
        provider_name: &str,
        provider: &WebhookProviderConfig,
    ) -> Result<Self, ConfigValidationError> {
        if provider.fields.is_empty() {
            return Err(ConfigValidationError::WebhookProviderMapMustNotBeEmpty {
                provider: provider_name.to_string(),
            });
        }

        let field_label_key = provider
            .field_label_key
            .as_deref()
            .map(parse_label_key)
            .transpose()
            .map_err(
                |label_key| ConfigValidationError::InvalidWebhookProviderLabelKey {
                    provider: provider_name.to_string(),
                    label_key,
                },
            )?;

        let mut target_paths: Vec<String> = Vec::with_capacity(provider.fields.len());
        let mut fields = Vec::with_capacity(provider.fields.len());
        for (field, path) in &provider.fields {
            if !is_event_field(field) {
                return Err(ConfigValidationError::UnknownWebhookProviderField {
                    provider: provider_name.to_string(),
                    field: field.clone(),
                });
            }

            let output_path = OutputPath::parse(path).map_err(|_| {
                ConfigValidationError::InvalidWebhookProviderPath {
                    provider: provider_name.to_string(),
                    field: field.clone(),
                    path: path.clone(),
                }
            })?;
            let canonical = output_path.canonical();
            for existing in &target_paths {
                if existing == &canonical {
                    return Err(ConfigValidationError::DuplicateWebhookProviderPath {
                        provider: provider_name.to_string(),
                        path: path.clone(),
                    });
                }
                // A parent path and one of its descendants cannot coexist: the
                // leaf insert for the parent overwrites the descendant's
                // container (or vice versa), silently dropping a mapped field.
                if is_path_ancestor(existing, &canonical)
                    || is_path_ancestor(&canonical, existing)
                {
                    return Err(ConfigValidationError::CollidingWebhookProviderPath {
                        provider: provider_name.to_string(),
                        path: path.clone(),
                    });
                }
            }
            target_paths.push(canonical);

            fields.push(ProviderFieldMapping {
                field: field.clone(),
                path: output_path,
            });
        }

        Ok(Self {
            field_label_key,
            fields,
        })
    }

    #[cfg(feature = "webhook")]
    pub(crate) fn render(&self, event: &VentEvent) -> Result<Value, String> {
        let event_json = serde_json::to_value(event)
            .map_err(|error| format!("failed to serialize webhook event: {error}"))?;
        let event_object = event_json
            .as_object()
            .ok_or_else(|| "failed to serialize webhook event as object".to_string())?;

        let mut payload = Value::Object(Map::new());
        for mapping in &self.fields {
            let value = event_object
                .get(&mapping.field)
                .cloned()
                .ok_or_else(|| format!("unknown webhook provider field: {}", mapping.field))?;
            let label = human_field_label(&mapping.field);
            mapping
                .path
                .insert(&mut payload, value, self.field_label_key.as_deref(), &label)
                .map_err(|error| {
                    format!(
                        "{error} in webhook provider path: {}",
                        mapping.path.canonical()
                    )
                })?;
        }

        Ok(payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderFieldMapping {
    field: String,
    path: OutputPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputPath {
    segments: Vec<PathSegment>,
}

impl OutputPath {
    fn parse(path: &str) -> Result<Self, ()> {
        let path = path.trim();
        if path.is_empty() {
            return Err(());
        }

        let mut segments = Vec::new();
        for (index, segment) in path.split('.').enumerate() {
            if segment.is_empty() {
                return Err(());
            }
            if let Ok(array_index) = segment.parse::<usize>() {
                if index == 0 || array_index > MAX_PROVIDER_ARRAY_INDEX {
                    return Err(());
                }
                segments.push(PathSegment::Index(array_index));
            } else {
                segments.push(PathSegment::Key(parse_path_key(segment)?));
            }
        }

        Ok(Self { segments })
    }

    fn canonical(&self) -> String {
        self.segments
            .iter()
            .map(PathSegment::as_str)
            .collect::<Vec<_>>()
            .join(".")
    }

    #[cfg(feature = "webhook")]
    fn insert(
        &self,
        root: &mut Value,
        value: Value,
        label_key: Option<&str>,
        label: &str,
    ) -> Result<(), String> {
        let Some(PathSegment::Key(_)) = self.segments.first() else {
            return Err("invalid webhook provider path".to_string());
        };
        insert_path_segments(root, &self.segments, value, label_key, label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSegment {
    Key(String),
    Index(usize),
}

impl PathSegment {
    fn as_str(&self) -> String {
        match self {
            PathSegment::Key(key) => key.clone(),
            PathSegment::Index(index) => index.to_string(),
        }
    }
}

fn parse_label_key(label_key: &str) -> Result<String, String> {
    parse_path_key(label_key).map_err(|_| label_key.to_string())
}

fn parse_path_key(segment: &str) -> Result<String, ()> {
    let valid = !segment.is_empty()
        && segment
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'));
    if valid {
        Ok(segment.to_string())
    } else {
        Err(())
    }
}

/// Reports whether `ancestor` is a strict path prefix of `descendant`, i.e. the
/// segments of `ancestor` are a leading subsequence of `descendant`'s segments.
///
/// Compares on the dotted canonical form: the trailing `.` requirement keeps
/// sibling segments that merely share a textual prefix (`a.1` vs `a.10`) from
/// being treated as a parent/child pair.
fn is_path_ancestor(ancestor: &str, descendant: &str) -> bool {
    descendant.len() > ancestor.len()
        && descendant.starts_with(ancestor)
        && descendant.as_bytes()[ancestor.len()] == b'.'
}

fn is_event_field(field: &str) -> bool {
    matches!(
        field,
        "id" | "timestamp" | "channel" | "message" | "project"
    )
}

#[cfg(feature = "webhook")]
pub(crate) fn webhook_payload(
    runtime_config: &RuntimeConfig,
    sink_config: &WebhookSinkConfig,
    event: &VentEvent,
) -> Result<Value, String> {
    let Some(provider_name) = sink_config.provider.as_deref().map(str::trim) else {
        return serde_json::to_value(event)
            .map_err(|error| format!("failed to serialize webhook event: {error}"));
    };

    if provider_name == "raw" {
        return serde_json::to_value(event)
            .map_err(|error| format!("failed to serialize webhook event: {error}"));
    }

    let provider = runtime_config
        .webhook_provider(provider_name)
        .ok_or_else(|| format!("unknown webhook provider: {provider_name}"))?;

    provider.render(event)
}

#[cfg(feature = "webhook")]
fn insert_path_segments(
    current: &mut Value,
    segments: &[PathSegment],
    value: Value,
    label_key: Option<&str>,
    label: &str,
) -> Result<(), String> {
    let (segment, rest) = segments
        .split_first()
        .ok_or_else(|| "missing segment".to_string())?;

    match segment {
        PathSegment::Index(index) => {
            // Defense in depth: compile-time validation already caps indices, so
            // a value above the bound means a path bypassed `OutputPath::parse`.
            // Fail with a structured error rather than risk an OOM or the
            // `index + 1` overflow that `Vec::resize` would hit at `usize::MAX`.
            if *index > MAX_PROVIDER_ARRAY_INDEX {
                return Err("webhook provider path array index out of bounds".to_string());
            }
            let array = ensure_array(current)?;
            if array.len() <= *index {
                array.resize(*index + 1, Value::Null);
            }
            if rest.is_empty() {
                array[*index] = value;
                return Ok(());
            }

            ensure_container_for_next(&mut array[*index], &rest[0])?;
            insert_path_segments(&mut array[*index], rest, value, label_key, label)
        }
        PathSegment::Key(key) => {
            let object = ensure_object(current)?;
            if rest.is_empty() {
                object.insert(key.clone(), value);
                if key == "value" {
                    if let Some(label_key) = label_key {
                        object
                            .entry(label_key.to_string())
                            .or_insert_with(|| Value::String(label.to_string()));
                    }
                }
                return Ok(());
            }

            let next = object.entry(key.clone()).or_insert(Value::Null);
            ensure_container_for_next(next, &rest[0])?;
            insert_path_segments(next, rest, value, label_key, label)
        }
    }
}

#[cfg(feature = "webhook")]
fn ensure_container_for_next(value: &mut Value, next_segment: &PathSegment) -> Result<(), String> {
    match next_segment {
        PathSegment::Index(_) => ensure_array(value).map(|_| ()),
        PathSegment::Key(_) => ensure_object(value).map(|_| ()),
    }
}

#[cfg(feature = "webhook")]
fn ensure_object(value: &mut Value) -> Result<&mut Map<String, Value>, String> {
    if value.is_null() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .ok_or_else(|| "path collides with non-object value".to_string())
}

#[cfg(feature = "webhook")]
fn ensure_array(value: &mut Value) -> Result<&mut Vec<Value>, String> {
    if value.is_null() {
        *value = Value::Array(Vec::new());
    }
    value
        .as_array_mut()
        .ok_or_else(|| "path collides with non-array value".to_string())
}

#[cfg(feature = "webhook")]
fn human_field_label(field: &str) -> String {
    let mut label = String::new();
    let mut capitalize_next = true;

    for character in field.chars() {
        if matches!(character, '_' | '-') {
            if !label.ends_with(' ') {
                label.push(' ');
            }
            capitalize_next = true;
            continue;
        }

        if capitalize_next {
            label.extend(character.to_uppercase());
            capitalize_next = false;
        } else {
            label.push(character);
        }
    }

    label
}
