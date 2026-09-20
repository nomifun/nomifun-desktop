//! Public contracts for Intelligent Decision-Making Mode (IDMM).
//!
//! IDMM is a per-AgentSession supervisor.  It has no Session identity or
//! execution loop of its own: the rule tier observes canonical Session facts,
//! and the optional bypass-model tier is used only to resolve an otherwise
//! ambiguous decision.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdmmMode {
    #[default]
    Off,
    RuleOnly,
    RulePlusModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdmmScanScope {
    LastTurn,
    #[default]
    LastMessages,
    FullSession,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct IdmmBypassModelRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

const fn default_true() -> bool {
    true
}

const fn default_scan_interval_secs() -> u32 {
    15
}

const fn default_idle_timeout_secs() -> u32 {
    90
}

const fn default_max_context_messages() -> u32 {
    12
}

const fn default_max_context_chars() -> u32 {
    8_000
}

const fn default_max_retries() -> u32 {
    3
}

const fn default_max_interventions_per_hour() -> u32 {
    20
}

const fn default_min_interval_secs() -> u32 {
    10
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdmmConfig {
    #[serde(default)]
    pub mode: IdmmMode,
    #[serde(default = "default_scan_interval_secs")]
    pub scan_interval_secs: u32,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u32,
    #[serde(default)]
    pub scan_scope: IdmmScanScope,
    #[serde(default = "default_max_context_messages")]
    pub max_context_messages: u32,
    #[serde(default = "default_max_context_chars")]
    pub max_context_chars: u32,
    #[serde(default = "default_true")]
    pub recover_provider_failures: bool,
    #[serde(default = "default_true")]
    pub recover_stalled_turns: bool,
    #[serde(default = "default_true")]
    pub auto_select_options: bool,
    #[serde(default = "default_true")]
    pub prefer_recommended: bool,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_max_interventions_per_hour")]
    pub max_interventions_per_hour: u32,
    #[serde(default = "default_min_interval_secs")]
    pub min_interval_secs: u32,
    #[serde(default)]
    pub bypass_model: IdmmBypassModelRef,
}

impl Default for IdmmConfig {
    fn default() -> Self {
        Self {
            mode: IdmmMode::Off,
            scan_interval_secs: default_scan_interval_secs(),
            idle_timeout_secs: default_idle_timeout_secs(),
            scan_scope: IdmmScanScope::default(),
            max_context_messages: default_max_context_messages(),
            max_context_chars: default_max_context_chars(),
            recover_provider_failures: true,
            recover_stalled_turns: true,
            auto_select_options: true,
            prefer_recommended: true,
            max_retries: default_max_retries(),
            max_interventions_per_hour: default_max_interventions_per_hour(),
            min_interval_secs: default_min_interval_secs(),
            bypass_model: IdmmBypassModelRef::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdmmRunState {
    Off,
    Monitoring,
    Intervening,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdmmInterventionKind {
    ProviderFailure,
    StalledTurn,
    OptionDecision,
    OpenQuestion,
    SafetyHalt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdmmInterventionStatus {
    Pending,
    Succeeded,
    Failed,
    Halted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdmmIntervention {
    pub intervention_id: String,
    pub fingerprint: String,
    pub kind: IdmmInterventionKind,
    pub status: IdmmInterventionStatus,
    pub action: String,
    pub tier: IdmmMode,
    pub reason: String,
    pub attempt: u32,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdmmState {
    pub agent_session_id: String,
    pub revision: u64,
    pub config: IdmmConfig,
    pub run_state: IdmmRunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,
    #[serde(default)]
    pub recent_interventions: Vec<IdmmIntervention>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_rule_config_receives_bounded_defaults() {
        let config: IdmmConfig = serde_json::from_str(r#"{"mode":"rule_only"}"#).unwrap();
        assert_eq!(config.mode, IdmmMode::RuleOnly);
        assert_eq!(config.scan_interval_secs, 15);
        assert_eq!(config.idle_timeout_secs, 90);
        assert!(config.recover_provider_failures);
        assert!(config.auto_select_options);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(serde_json::from_str::<IdmmConfig>(r#"{"legacy_tier":"smart"}"#).is_err());
    }
}
