//! Persisted companion configuration: opt-in collection switches, learning model,
//! persona, appearance and quiet-hours. Stored as `config.json` under the companion
//! dir with atomic temp+rename writes (same pattern as cron skill files).

use std::path::{Path, PathBuf};

use nomifun_common::ProviderWithModel;
use serde::{Deserialize, Serialize};

/// The roster character every companion falls back to when none is configured.
pub(crate) const DEFAULT_CHARACTER: &str = "mochi";

/// Which event sources the user has opted into collecting. The work-event
/// sources all default OFF; `companion_dialogues` (direct conversations with the
/// companions) defaults ON — talking to the companion is itself the opt-in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CollectConfig {
    pub chat_user_messages: bool,
    pub chat_assistant_replies: bool,
    pub requirements: bool,
    pub cron_runs: bool,
    pub conversation_lifecycle: bool,
    pub terminal_sessions: bool,
    /// Tool-call capture from owner work sessions: tool NAME + normalized param
    /// SHAPE only (sorted top-level arg keys + JSON types), never values. The
    /// primary mining signal for skill self-evolution (design §5.1).
    pub tool_calls: bool,
    /// Companion-dialogue capture: owner messages + companion replies inside companion
    /// (companion / Channel Agent) conversations. The field-level serde
    /// default keeps it ON for legacy `config.json` files written before the
    /// field existed.
    #[serde(default = "default_true")]
    pub companion_dialogues: bool,
}

fn default_true() -> bool {
    true
}

impl Default for CollectConfig {
    fn default() -> Self {
        Self {
            chat_user_messages: false,
            chat_assistant_replies: false,
            requirements: false,
            cron_runs: false,
            conversation_lifecycle: false,
            terminal_sessions: false,
            tool_calls: false,
            companion_dialogues: true,
        }
    }
}

impl CollectConfig {
    /// Whether any of the opt-in *work-event* sources is enabled (UI
    /// onboarding hint). Deliberately excludes `companion_dialogues`, which is on
    /// by default and would make this vacuously true.
    pub fn any_enabled(&self) -> bool {
        self.chat_user_messages
            || self.chat_assistant_replies
            || self.requirements
            || self.cron_runs
            || self.conversation_lifecycle
            || self.terminal_sessions
            || self.tool_calls
    }
}

pub(crate) fn deserialize_optional_model<'de, D>(deserializer: D) -> Result<Option<ProviderWithModel>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let model = Option::<ProviderWithModel>::deserialize(deserializer)?;
    if let Some(model) = model.as_ref() {
        model.validate().map_err(serde::de::Error::custom)?;
    }
    Ok(model)
}

/// Scheduled learning settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LearnConfig {
    pub enabled: bool,
    /// Minutes between learning runs.
    pub interval_minutes: u32,
}

impl Default for LearnConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 60,
        }
    }
}

/// Desktop-companion appearance + notification behaviour.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppearanceConfig {
    /// Whether the desktop companion window should be visible.
    pub companion_enabled: bool,
    /// Which character renders in the companion window (see the UI character
    /// roster: mochi/ink/roux/pixel/bolt/boo). Unknown values fall back to
    /// the default character on the renderer side.
    pub character: String,
    /// Saved companion window position (physical px), if the user dragged it.
    pub companion_x: Option<i32>,
    pub companion_y: Option<i32>,
    /// Quiet hours "HH:mm" — within this window the companion only accrues badges
    /// and never pops bubbles. Empty strings disable quiet hours.
    pub quiet_start: String,
    pub quiet_end: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            companion_enabled: false,
            character: DEFAULT_CHARACTER.into(),
            companion_x: None,
            companion_y: None,
            quiet_start: String::new(),
            quiet_end: String::new(),
        }
    }
}

/// Persona settings injected into the chat/learn system prompts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
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

/// The full persisted companion configuration.
///
/// LEGACY: this is the pre-multi-companion single-config shape, kept only so boot
/// can read an old `companion/nomi/config.json` and migrate it into the new
/// per-companion [`crate::profile::CompanionProfileConfig`] + shared
/// [`crate::profile::SharedCompanionConfig`] split. Do not extend it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompanionConfig {
    pub collect: CollectConfig,
    #[serde(default, deserialize_with = "deserialize_optional_model")]
    pub model: Option<ProviderWithModel>,
    pub learn: LearnConfig,
    pub appearance: AppearanceConfig,
    pub persona: PersonaConfig,
}

impl CompanionConfig {
    pub fn config_path(companion_dir: &Path) -> PathBuf {
        companion_dir.join("config.json")
    }

    /// Load from `{companion_dir}/config.json`, falling back to defaults when the
    /// file is missing or unreadable (a corrupt config must never brick boot).
    pub fn load(companion_dir: &Path) -> Self {
        crate::fsio::load_json_or_default(&Self::config_path(companion_dir))
    }

    /// Atomically persist to `{companion_dir}/config.json` (unique temp file +
    /// rename, so two concurrent saves can never rename each other's
    /// half-written temp into place).
    pub fn save(&self, companion_dir: &Path) -> std::io::Result<()> {
        crate::fsio::save_json_atomic(companion_dir, "config.json", self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_default_on_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = CompanionConfig::load(dir.path());
        assert_eq!(loaded, CompanionConfig::default());
        assert!(!loaded.collect.any_enabled());

        let mut cfg = CompanionConfig::default();
        cfg.collect.chat_user_messages = true;
        cfg.model = Some(ProviderWithModel {
            provider_id: nomifun_common::ProviderId::new().into_string(),
            model: "claude-fable-5".into(),
            use_model: None,
        });
        cfg.learn.enabled = true;
        cfg.save(dir.path()).unwrap();

        let again = CompanionConfig::load(dir.path());
        assert_eq!(again, cfg);
        assert!(again.model.is_some());
    }

    #[test]
    fn corrupt_config_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(CompanionConfig::config_path(dir.path()), "{not json").unwrap();
        assert_eq!(CompanionConfig::load(dir.path()), CompanionConfig::default());
    }

    #[test]
    fn legacy_model_selection_is_nullable_and_strict() {
        let default_json = serde_json::to_value(CompanionConfig::default()).unwrap();
        assert!(default_json["model"].is_null(), "unconfigured model must serialize as null");

        let canonical_provider = nomifun_common::ProviderId::new().into_string();
        for model in [
            serde_json::json!({"provider_id": "", "model": "chat"}),
            serde_json::json!({"provider_id": "prov_x", "model": "chat"}),
            serde_json::json!({"provider_id": canonical_provider, "model": ""}),
            serde_json::json!({"provider_id": canonical_provider, "model": " chat "}),
            serde_json::json!({"provider_id": canonical_provider, "model": "chat", "use_model": ""}),
            serde_json::json!({"provider_id": canonical_provider, "model": "chat", "use_model": " fast "}),
        ] {
            let result = serde_json::from_value::<CompanionConfig>(serde_json::json!({"model": model}));
            assert!(result.is_err(), "partial or malformed model ref must be rejected");
        }
    }

    #[test]
    fn legacy_collect_json_defaults_companion_dialogues_on() {
        // Stored configs written before the field existed must come back ON.
        let legacy: CollectConfig = serde_json::from_str(r#"{"chat_user_messages":true}"#).unwrap();
        assert!(legacy.companion_dialogues);
        assert!(legacy.chat_user_messages);
        // …and an explicit false is respected.
        let off: CollectConfig = serde_json::from_str(r#"{"companion_dialogues":false}"#).unwrap();
        assert!(!off.companion_dialogues);

        // Full legacy config.json on disk (no companion_dialogues key) roundtrips
        // through the file loader with the field defaulted ON.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            CompanionConfig::config_path(dir.path()),
            r#"{"collect":{"requirements":true}}"#,
        )
        .unwrap();
        let loaded = CompanionConfig::load(dir.path());
        assert!(loaded.collect.companion_dialogues);
        assert!(loaded.collect.requirements);
        // companion_dialogues is excluded from the work-event onboarding hint.
        assert!(CollectConfig::default().companion_dialogues);
        assert!(!CollectConfig::default().any_enabled());
    }
}
