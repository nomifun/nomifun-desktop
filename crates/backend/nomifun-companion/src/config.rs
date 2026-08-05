//! Persisted companion configuration: opt-in collection switches, learning model,
//! persona, appearance and quiet-hours. Stored as `config.json` under the companion
//! dir with atomic temp+rename writes (same pattern as cron skill files).

use nomifun_common::ProviderWithModel;
use serde::{Deserialize, Serialize};

/// The roster character every companion falls back to when none is configured.
pub(crate) const DEFAULT_CHARACTER: &str = "mochi";

pub const DEFAULT_EVENT_RETENTION_DAYS: u32 = 30;
pub const MIN_EVENT_RETENTION_DAYS: u32 = 7;
pub const MAX_EVENT_RETENTION_DAYS: u32 = 365;
pub const DEFAULT_EVENT_MAX_STORAGE_MB: u32 = 64;
pub const MIN_EVENT_MAX_STORAGE_MB: u32 = 16;
pub const MAX_EVENT_MAX_STORAGE_MB: u32 = 512;

const fn default_event_retention_days() -> u32 {
    DEFAULT_EVENT_RETENTION_DAYS
}

const fn default_event_max_storage_mb() -> u32 {
    DEFAULT_EVENT_MAX_STORAGE_MB
}

/// Which event sources the user has opted into collecting. The work-event
/// sources all default OFF; `companion_dialogues` (direct conversations with the
/// companions) defaults ON — talking to the companion is itself the opt-in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CollectConfig {
    pub chat_user_messages: bool,
    pub requirements: bool,
    pub terminal_sessions: bool,
    /// Tool-call capture from owner work sessions: tool NAME + normalized param
    /// SHAPE only (sorted top-level arg keys + JSON types), never values. The
    /// primary mining signal for skill self-evolution (design §5.1).
    pub tool_calls: bool,
    /// Companion-dialogue capture: owner messages + companion replies inside companion
    /// (companion / Channel Agent) conversations.
    pub companion_dialogues: bool,
    /// Number of local calendar days kept in the raw event spool. Old files
    /// are removed only after every currently-enabled background consumer has
    /// advanced past them; the hard byte cap below always wins.
    #[serde(default = "default_event_retention_days")]
    pub event_retention_days: u32,
    /// Hard upper bound for the complete raw event spool. When space is needed,
    /// the oldest day file is removed first, even if a consumer has not read it.
    #[serde(default = "default_event_max_storage_mb")]
    pub event_max_storage_mb: u32,
}

impl Default for CollectConfig {
    fn default() -> Self {
        Self {
            chat_user_messages: false,
            requirements: false,
            terminal_sessions: false,
            tool_calls: false,
            companion_dialogues: true,
            event_retention_days: DEFAULT_EVENT_RETENTION_DAYS,
            event_max_storage_mb: DEFAULT_EVENT_MAX_STORAGE_MB,
        }
    }
}

impl CollectConfig {
    /// Whether any of the opt-in *work-event* sources is enabled (UI
    /// onboarding hint). Deliberately excludes `companion_dialogues`, which is on
    /// by default and would make this vacuously true.
    pub fn any_enabled(&self) -> bool {
        self.chat_user_messages
            || self.requirements
            || self.terminal_sessions
            || self.tool_calls
    }

    pub fn validate_storage_policy(&self) -> Result<(), String> {
        if !(MIN_EVENT_RETENTION_DAYS..=MAX_EVENT_RETENTION_DAYS)
            .contains(&self.event_retention_days)
        {
            return Err(format!(
                "event_retention_days must be between {MIN_EVENT_RETENTION_DAYS} and {MAX_EVENT_RETENTION_DAYS}"
            ));
        }
        if !(MIN_EVENT_MAX_STORAGE_MB..=MAX_EVENT_MAX_STORAGE_MB)
            .contains(&self.event_max_storage_mb)
        {
            return Err(format!(
                "event_max_storage_mb must be between {MIN_EVENT_MAX_STORAGE_MB} and {MAX_EVENT_MAX_STORAGE_MB}"
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedProviderModel {
    #[serde(deserialize_with = "deserialize_provider_id")]
    provider_id: String,
    model: String,
}

fn deserialize_provider_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    nomifun_common::ProviderId::parse(raw)
        .map(nomifun_common::ProviderId::into_string)
        .map_err(serde::de::Error::custom)
}

/// Deserialize the only persisted Provider-reference shape accepted by the
/// companion side store: exactly `{provider_id, model}`. `use_model` is a
/// runtime DTO concern and is deliberately not a side-store field.
pub(crate) fn deserialize_optional_model<'de, D>(
    deserializer: D,
) -> Result<Option<ProviderWithModel>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let model = Option::<PersistedProviderModel>::deserialize(deserializer)?;
    model
        .map(|model| {
            let model = ProviderWithModel {
                provider_id: model.provider_id,
                model: model.model,
                use_model: None,
            };
            model.validate().map_err(serde::de::Error::custom)?;
            Ok(model)
        })
        .transpose()
}

pub(crate) fn serialize_optional_model<S>(
    model: &Option<ProviderWithModel>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match model {
        None => serializer.serialize_none(),
        Some(model) => {
            model.validate().map_err(serde::ser::Error::custom)?;
            if model.use_model.is_some() {
                return Err(serde::ser::Error::custom(
                    "companion side-store model must use exactly {provider_id, model}",
                ));
            }
            Some(PersistedProviderModel {
                provider_id: model.provider_id.clone(),
                model: model.model.clone(),
            })
            .serialize(serializer)
        }
    }
}

/// Persona settings injected into the chat/learn system prompts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PersonaConfig {
    /// One of `lively` | `calm` | `sassy`.
    pub preset: String,
    /// Free-form extra persona instructions appended by the user.
    pub custom: String,
}

impl Default for PersonaConfig {
    fn default() -> Self {
        Self {
            preset: "lively".into(),
            custom: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_defaults_are_explicit() {
        let config = CollectConfig::default();
        assert!(config.companion_dialogues);
        assert!(!config.any_enabled());
        assert_eq!(config.event_retention_days, DEFAULT_EVENT_RETENTION_DAYS);
        assert_eq!(config.event_max_storage_mb, DEFAULT_EVENT_MAX_STORAGE_MB);
        assert!(config.validate_storage_policy().is_ok());
        let wire = serde_json::to_value(config).unwrap();
        assert!(wire.get("chat_assistant_replies").is_none());
        assert!(wire.get("cron_runs").is_none());
        assert!(wire.get("conversation_lifecycle").is_none());
    }

    #[test]
    fn legacy_collect_config_gets_storage_defaults_without_resetting_switches() {
        let legacy = serde_json::json!({
            "chat_user_messages": true,
            "requirements": false,
            "terminal_sessions": true,
            "tool_calls": true,
            "companion_dialogues": false
        });
        let config: CollectConfig = serde_json::from_value(legacy).unwrap();
        assert!(config.chat_user_messages);
        assert!(config.terminal_sessions);
        assert!(config.tool_calls);
        assert!(!config.companion_dialogues);
        assert_eq!(config.event_retention_days, DEFAULT_EVENT_RETENTION_DAYS);
        assert_eq!(config.event_max_storage_mb, DEFAULT_EVENT_MAX_STORAGE_MB);
    }

    #[test]
    fn storage_policy_accepts_only_documented_boundaries() {
        for (days, megabytes, valid) in [
            (7, 16, true),
            (365, 512, true),
            (6, 64, false),
            (366, 64, false),
            (30, 15, false),
            (30, 513, false),
        ] {
            let config = CollectConfig {
                event_retention_days: days,
                event_max_storage_mb: megabytes,
                ..CollectConfig::default()
            };
            assert_eq!(
                config.validate_storage_policy().is_ok(),
                valid,
                "days={days}, megabytes={megabytes}"
            );
        }
    }
}
