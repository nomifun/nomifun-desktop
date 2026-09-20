//! Versioned Agent runtime policies that do not grant model capabilities.
//!
//! These settings are frozen into an AgentPreset Revision and copied into a
//! newly created AgentSession.  They are deliberately separate from the
//! Capability Catalog: a supervisor can keep a turn alive, but it never grants
//! the model a Tool, Context source, permission, or resource binding.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CanonicalErrorCode, PresetContractViolation};

pub const PRESET_RUNTIME_POLICY_INVALID: &str = "PRESET_RUNTIME_POLICY_INVALID";

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentIdmmMode {
    #[default]
    Off,
    RuleOnly,
    RulePlusModel,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentIdmmScanScope {
    LastTurn,
    #[default]
    LastMessages,
    FullSession,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentIdmmBypassModelRef {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentIdmmPolicy {
    #[serde(default)]
    pub mode: AgentIdmmMode,
    #[serde(default = "default_scan_interval_secs")]
    pub scan_interval_secs: u32,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u32,
    #[serde(default)]
    pub scan_scope: AgentIdmmScanScope,
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
    pub bypass_model: AgentIdmmBypassModelRef,
}

impl Default for AgentIdmmPolicy {
    fn default() -> Self {
        Self {
            mode: AgentIdmmMode::Off,
            scan_interval_secs: default_scan_interval_secs(),
            idle_timeout_secs: default_idle_timeout_secs(),
            scan_scope: AgentIdmmScanScope::default(),
            max_context_messages: default_max_context_messages(),
            max_context_chars: default_max_context_chars(),
            recover_provider_failures: true,
            recover_stalled_turns: true,
            auto_select_options: true,
            prefer_recommended: true,
            max_retries: default_max_retries(),
            max_interventions_per_hour: default_max_interventions_per_hour(),
            min_interval_secs: default_min_interval_secs(),
            bypass_model: AgentIdmmBypassModelRef::default(),
        }
    }
}

impl AgentIdmmPolicy {
    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        if !(5..=300).contains(&self.scan_interval_secs) {
            return Err(invalid("IDMM scan_interval_secs must be between 5 and 300"));
        }
        if !(30..=1_800).contains(&self.idle_timeout_secs) {
            return Err(invalid("IDMM idle_timeout_secs must be between 30 and 1800"));
        }
        if !(1..=100).contains(&self.max_context_messages)
            || !(1_000..=64_000).contains(&self.max_context_chars)
            || !(1..=10).contains(&self.max_retries)
            || !(1..=100).contains(&self.max_interventions_per_hour)
            || self.min_interval_secs > 600
        {
            return Err(invalid("IDMM limits are outside their supported bounds"));
        }
        let provider = self.bypass_model.provider_id.as_deref();
        let model = self.bypass_model.model.as_deref();
        if provider.is_some() != model.is_some() {
            return Err(invalid(
                "IDMM bypass provider and model must be selected together",
            ));
        }
        if provider.is_some_and(|value| value.trim().is_empty() || value.trim() != value) {
            return Err(invalid(
                "IDMM bypass provider must be trimmed and non-empty",
            ));
        }
        if model.is_some_and(|value| value.trim().is_empty() || value.trim() != value) {
            return Err(invalid("IDMM bypass model must be trimmed and non-empty"));
        }
        if self.mode == AgentIdmmMode::RulePlusModel && provider.is_none() {
            return Err(invalid(
                "rules plus bypass model mode requires an explicit bypass model",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentRuntimePolicy {
    #[serde(default)]
    pub idmm: AgentIdmmPolicy,
}

impl AgentRuntimePolicy {
    pub fn is_default(value: &Self) -> bool {
        value == &Self::default()
    }

    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        self.idmm.validate()
    }
}

fn invalid(message: impl Into<String>) -> PresetContractViolation {
    PresetContractViolation {
        code: CanonicalErrorCode::from(PRESET_RUNTIME_POLICY_INVALID),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_policy_receives_safe_defaults() {
        let policy: AgentRuntimePolicy = serde_json::from_str(r#"{"idmm":{"mode":"rule_only"}}"#)
            .unwrap();
        assert_eq!(policy.idmm.mode, AgentIdmmMode::RuleOnly);
        assert_eq!(policy.idmm.idle_timeout_secs, 90);
        assert!(policy.idmm.recover_provider_failures);
        policy.validate().unwrap();
    }

    #[test]
    fn bypass_tier_requires_an_exact_pair() {
        let policy: AgentRuntimePolicy = serde_json::from_str(
            r#"{"idmm":{"mode":"rule_plus_model","bypass_model":{"provider_id":"provider"}}}"#,
        )
        .unwrap();
        let error = policy.validate().unwrap_err();
        assert_eq!(error.code.as_ref(), PRESET_RUNTIME_POLICY_INVALID);
    }
}
