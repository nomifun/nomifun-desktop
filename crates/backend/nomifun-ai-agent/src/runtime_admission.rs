//! Compile-time engine compatibility, not a second capability authority.
//!
//! Every bundled engine registers this policy alongside its factory. The host
//! checks it before saving an Agent or changing a Session; the Kernel still
//! decides whether each actual operation is authorized.
use std::collections::BTreeSet;

use nomifun_agent_contracts::{ContributionSourceKind, ResolvedSnapshotEnvelope};
use nomifun_common::AppError;
use serde_json::Value;

use crate::RuntimeEngineBinding;

pub trait RuntimeEngineAdmission: Send + Sync {
    /// Opt in only when conversational state is rebuilt from the platform
    /// history ports each turn, with no independent/private prompt history.
    /// Maintenance retires the live runtime and advances the owner's history
    /// floor; durable safety obligations must remain outside that projection.
    fn uses_platform_history_context(&self, _binding: &RuntimeEngineBinding) -> bool {
        false
    }

    /// Trusted source declaration for the exact build/profile, never inferred
    /// from a family name or client JSON. Opt in only when the factory uses
    /// Nomi's Plugin Tool scope AND its private Session persistence/recovery
    /// contract. The returned runtime must also report uses_nomi_recovery().
    /// Independent engines keep the default and own their recovery codec.
    fn uses_nomi_session(&self, _binding: &RuntimeEngineBinding) -> bool {
        false
    }

    fn validate_snapshot(
        &self,
        binding: &RuntimeEngineBinding,
        snapshot: &ResolvedSnapshotEnvelope,
    ) -> Result<(), AppError>;

    /// Validate effective Session overlays as well as saved Agent selections.
    /// Values are host projections, never a source of additional permissions.
    fn validate_session_extra(
        &self,
        binding: &RuntimeEngineBinding,
        extra: &Value,
    ) -> Result<(), AppError>;
}

/// A convenience policy for engines that do not need custom profile-dependent
/// validation. `None` accepts any platform-admitted selected capability; an
/// empty set accepts none. The allowlist covers all enabled capabilities,
/// including Plugin Products. These declarations never install or grant tools.
pub struct RuntimeEngineSupport {
    pub enabled_capabilities: Option<BTreeSet<String>>,
    pub skills: bool,
    pub mcp: bool,
    pub plugin_products: bool,
}

impl RuntimeEngineSupport {
    pub fn platform() -> Self {
        Self {
            enabled_capabilities: None,
            skills: true,
            mcp: true,
            plugin_products: true,
        }
    }

    pub fn enabled_only(capabilities: impl IntoIterator<Item = String>) -> Self {
        Self {
            enabled_capabilities: Some(capabilities.into_iter().collect()),
            skills: false,
            mcp: false,
            plugin_products: false,
        }
    }

    /// Source-compatible constructor for existing engine registrations.
    /// This is an enabled-capability allowlist, not an initial/on-demand split.
    pub fn initial_only(capabilities: impl IntoIterator<Item = String>) -> Self {
        Self::enabled_only(capabilities)
    }
}

fn unsupported(binding: &RuntimeEngineBinding, feature: &str) -> AppError {
    AppError::Conflict(format!(
        "Engine {} / {} / {} does not support {feature}",
        binding.family_id, binding.build_id, binding.profile
    ))
}

impl RuntimeEngineAdmission for RuntimeEngineSupport {
    fn validate_snapshot(
        &self,
        binding: &RuntimeEngineBinding,
        snapshot: &ResolvedSnapshotEnvelope,
    ) -> Result<(), AppError> {
        let content = &snapshot.content;
        if let Some(supported) = &self.enabled_capabilities {
            for item in &content.enabled_capabilities {
                if !supported.contains(item.capability.id.as_ref()) {
                    return Err(unsupported(binding, item.capability.id.as_ref()));
                }
            }
        }
        for (allowed, selected, feature) in [
            (self.skills, !content.skill_locks.is_empty(), "Skills"),
            (self.mcp, !content.mcp_tool_locks.is_empty()
                || content.enabled_capabilities.iter()
                    .any(|entry| entry.capability.id.as_ref() == "mcp.resource"
                        || entry.contribution_lock.source_kind == ContributionSourceKind::McpBinding), "MCP"),
            (self.plugin_products, content.enabled_capabilities.iter()
                .any(|entry| entry.contribution_lock.source_kind == ContributionSourceKind::PluginProductActiveRelease), "Plugin Products"),
        ] {
            if !allowed && selected {
                return Err(unsupported(binding, feature));
            }
        }
        Ok(())
    }

    fn validate_session_extra(
        &self,
        binding: &RuntimeEngineBinding,
        extra: &Value,
    ) -> Result<(), AppError> {
        for (allowed, keys, feature) in [
            (self.skills, &["skills", "session_enabled_skills"][..], "session Skills"),
            (self.mcp, &["mcp_server_ids", "mcp_servers", "selected_mcp_server_ids"][..], "session MCP"),
        ] {
            if !allowed && keys.iter().any(|key| extra.get(*key)
                .is_some_and(|value| !value.as_array().is_some_and(Vec::is_empty))) {
                return Err(unsupported(binding, feature));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> RuntimeEngineBinding {
        RuntimeEngineBinding {
            family_id: "community.planner".into(), build_id: "1".into(),
            build_digest: "a".repeat(64), host_contract_version: 1, profile: "plan".into(),
        }
    }

    #[test]
    fn arbitrary_engine_rejects_all_overlay_aliases_and_malformed_values() {
        let policy = RuntimeEngineSupport::enabled_only([]);
        for key in ["skills", "session_enabled_skills", "mcp_server_ids", "mcp_servers", "selected_mcp_server_ids"] {
            for value in [serde_json::json!(["selected"]), Value::Null, serde_json::json!({})] {
                let extra = serde_json::json!({key: value});
                assert!(policy.validate_session_extra(&binding(), &extra).is_err(), "{extra}");
            }
            policy.validate_session_extra(&binding(), &serde_json::json!({key: []})).unwrap();
        }
        policy.validate_session_extra(&binding(), &serde_json::json!({})).unwrap();
        RuntimeEngineSupport::platform().validate_session_extra(&binding(), &serde_json::json!({"skills":["pdf"]})).unwrap();
    }
}
