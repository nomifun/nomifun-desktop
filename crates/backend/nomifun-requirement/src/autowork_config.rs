use nomifun_common::AppError;
use serde::Serialize;
use serde_json::{Map, Value};

const TERMINAL_REVISION_FIELD: &str = "_revision";
const OPERATION_ID_FIELD: &str = "_operation_id";

/// Canonical AutoWork configuration shared by REST, Gateway, boot recovery,
/// and the live runner.
///
/// Tags are normalized exactly once at the command boundary. Persisted values
/// are parsed through the same rules, so a restart cannot silently change the
/// binding identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AutoWorkConfig {
    pub enabled: bool,
    pub tag: Option<String>,
    pub max_requirements: Option<u32>,
}

impl AutoWorkConfig {
    pub fn normalize(
        enabled: bool,
        tag: Option<&str>,
        max_requirements: Option<u32>,
    ) -> Result<Self, AppError> {
        let tag = match tag {
            Some(raw) => {
                let normalized = raw.trim();
                if normalized.is_empty() {
                    return Err(AppError::BadRequest(
                        "AutoWork tag must not be empty or whitespace".to_owned(),
                    ));
                }
                Some(normalized.to_owned())
            }
            None => None,
        };
        if enabled && tag.is_none() {
            return Err(AppError::BadRequest(
                "tag is required when enabling autowork".to_owned(),
            ));
        }
        Ok(Self {
            enabled,
            tag,
            max_requirements,
        })
    }

    pub fn enabled_tag(&self) -> Result<&str, AppError> {
        if !self.enabled {
            return Err(AppError::Conflict(
                "AutoWork cannot start from a disabled configuration".to_owned(),
            ));
        }
        self.tag.as_deref().ok_or_else(|| {
            AppError::Conflict("enabled AutoWork configuration has no tag".to_owned())
        })
    }

    fn semantic_fingerprint(&self) -> String {
        // serde_json gives a deterministic representation for this struct and
        // safely length-delimits arbitrary tag text.
        serde_json::to_string(self).expect("AutoWorkConfig serialization cannot fail")
    }
}

/// Owner-scoped persisted AutoWork configuration plus its optimistic
/// concurrency token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoWorkConfigSnapshot {
    pub config: AutoWorkConfig,
    pub revision: String,
    pub operation_id: Option<String>,
}

impl AutoWorkConfigSnapshot {
    pub fn new(
        config: AutoWorkConfig,
        revision: impl Into<String>,
        operation_id: Option<String>,
    ) -> Result<Self, AppError> {
        let revision = revision.into();
        if revision.trim().is_empty() {
            return Err(AppError::Conflict(
                "AutoWork config revision must not be empty".to_owned(),
            ));
        }
        if operation_id
            .as_deref()
            .is_some_and(|operation_id| operation_id.trim().is_empty())
        {
            return Err(AppError::Conflict(
                "AutoWork config operation identity must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            config,
            revision,
            operation_id,
        })
    }
}

/// Atomic Session-side config mutation requested by Requirement.
///
/// The host must verify `owner_id`, compare `expected_revision`, and merge only
/// its AutoWork-owned metadata in one storage transaction. Replaying the same
/// `operation_id` with a different `config` must return `Conflict`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoWorkSessionConfigCommand {
    pub owner_id: String,
    pub session_id: String,
    pub config: AutoWorkConfig,
    pub expected_revision: String,
    pub operation_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct DecodedTerminalAutoWorkConfig {
    pub snapshot: AutoWorkConfigSnapshot,
    pub sequence: u64,
}

pub(crate) fn decode_terminal_autowork_config(
    raw: Option<&Value>,
    target_id: &str,
) -> Result<DecodedTerminalAutoWorkConfig, AppError> {
    let Some(raw) = raw else {
        return Ok(DecodedTerminalAutoWorkConfig {
            snapshot: AutoWorkConfigSnapshot {
                config: AutoWorkConfig::default(),
                revision: "terminal:0".to_owned(),
                operation_id: None,
            },
            sequence: 0,
        });
    };
    let object = raw.as_object().ok_or_else(|| {
        AppError::Internal(format!(
            "AutoWork config for terminal {target_id} must be a JSON object"
        ))
    })?;
    let enabled = optional_bool(object, "enabled", target_id)?.unwrap_or(false);
    let tag = optional_string(object, "tag", target_id)?;
    let max_requirements = optional_u32(object, "max_requirements", target_id)?;
    let config = AutoWorkConfig::normalize(enabled, tag.as_deref(), max_requirements).map_err(
        |error| {
            AppError::Internal(format!(
                "terminal {target_id} has an invalid persisted AutoWork config: {error}"
            ))
        },
    )?;
    let sequence = optional_u64(object, TERMINAL_REVISION_FIELD, target_id)?.unwrap_or(0);
    let operation_id = optional_string(object, OPERATION_ID_FIELD, target_id)?;
    let revision = if sequence == 0 && !object.contains_key(TERMINAL_REVISION_FIELD) {
        format!("terminal:legacy:{}", config.semantic_fingerprint())
    } else {
        format!("terminal:{sequence}")
    };
    Ok(DecodedTerminalAutoWorkConfig {
        snapshot: AutoWorkConfigSnapshot {
            config,
            revision,
            operation_id,
        },
        sequence,
    })
}

pub(crate) fn encode_terminal_autowork_config(
    config: &AutoWorkConfig,
    sequence: u64,
    operation_id: Option<&str>,
) -> Value {
    serde_json::json!({
        "enabled": config.enabled,
        "tag": config.tag,
        "max_requirements": config.max_requirements,
        (TERMINAL_REVISION_FIELD): sequence,
        (OPERATION_ID_FIELD): operation_id,
    })
}

fn optional_bool(
    object: &Map<String, Value>,
    field: &str,
    target_id: &str,
) -> Result<Option<bool>, AppError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(invalid_field(target_id, field, "a boolean")),
    }
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
    target_id: &str,
) -> Result<Option<String>, AppError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Err(invalid_field(target_id, field, "a non-empty string"))
            } else {
                Ok(Some(value.to_owned()))
            }
        }
        Some(_) => Err(invalid_field(target_id, field, "a string")),
    }
}

fn optional_u32(
    object: &Map<String, Value>,
    field: &str,
    target_id: &str,
) -> Result<Option<u32>, AppError> {
    optional_u64(object, field, target_id)?
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                invalid_field(target_id, field, "an unsigned 32-bit integer")
            })
        })
        .transpose()
}

fn optional_u64(
    object: &Map<String, Value>,
    field: &str,
    target_id: &str,
) -> Result<Option<u64>, AppError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| invalid_field(target_id, field, "an unsigned integer")),
        Some(_) => Err(invalid_field(target_id, field, "an unsigned integer")),
    }
}

fn invalid_field(target_id: &str, field: &str, expected: &str) -> AppError {
    AppError::Internal(format!(
        "AutoWork config for terminal {target_id} field '{field}' must be {expected}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_normalization_trims_once_and_rejects_blank_tags() {
        assert_eq!(
            AutoWorkConfig::normalize(true, Some("  release  "), Some(3)).unwrap(),
            AutoWorkConfig {
                enabled: true,
                tag: Some("release".to_owned()),
                max_requirements: Some(3),
            }
        );
        assert!(AutoWorkConfig::normalize(true, Some(" \t "), None).is_err());
        assert!(AutoWorkConfig::normalize(false, Some(" \n "), None).is_err());
        assert!(AutoWorkConfig::normalize(true, None, None).is_err());
    }

    #[test]
    fn legacy_terminal_config_gets_a_stable_semantic_revision() {
        let raw = serde_json::json!({
            "enabled": true,
            "tag": " release ",
            "max_requirements": 2,
        });
        let first = decode_terminal_autowork_config(Some(&raw), "terminal").unwrap();
        let second = decode_terminal_autowork_config(Some(&raw), "terminal").unwrap();
        assert_eq!(first.snapshot, second.snapshot);
        assert_eq!(first.snapshot.config.tag.as_deref(), Some("release"));
        assert!(first.snapshot.revision.starts_with("terminal:legacy:"));
    }

    #[test]
    fn terminal_envelope_roundtrips_revision_and_operation_identity() {
        let config = AutoWorkConfig::normalize(true, Some("alpha"), Some(5)).unwrap();
        let raw = encode_terminal_autowork_config(&config, 9, Some("gateway:op"));
        let decoded = decode_terminal_autowork_config(Some(&raw), "terminal").unwrap();
        assert_eq!(decoded.sequence, 9);
        assert_eq!(decoded.snapshot.config, config);
        assert_eq!(decoded.snapshot.revision, "terminal:9");
        assert_eq!(
            decoded.snapshot.operation_id.as_deref(),
            Some("gateway:op")
        );
    }
}
