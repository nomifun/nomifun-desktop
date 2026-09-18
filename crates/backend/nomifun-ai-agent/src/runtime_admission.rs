//! Compile-time compatibility policy for the single official Runtime.
//!
//! Capability authority remains with the Kernel. This policy only checks that
//! the source-integrated implementation can consume a compiled Snapshot and
//! its effective Session overlays.

use std::collections::BTreeSet;

use nomifun_agent_contracts::{ContributionSourceKind, ResolvedSnapshotEnvelope};
use nomifun_common::AppError;
use serde_json::Value;

pub trait RuntimeAdmission: Send + Sync {
    fn supports_tool_hooks(&self) -> bool {
        false
    }

    fn validate_snapshot(&self, snapshot: &ResolvedSnapshotEnvelope) -> Result<(), AppError>;

    fn validate_session_extra(&self, extra: &Value) -> Result<(), AppError>;
}

pub struct RuntimeSupport {
    pub enabled_capabilities: Option<BTreeSet<String>>,
    pub skills: bool,
    pub mcp: bool,
    pub plugin_products: bool,
}

impl RuntimeSupport {
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

    pub fn initial_only(capabilities: impl IntoIterator<Item = String>) -> Self {
        Self::enabled_only(capabilities)
    }
}

fn unsupported(feature: &str) -> AppError {
    AppError::Conflict(format!("The official Runtime does not support {feature}"))
}

impl RuntimeAdmission for RuntimeSupport {
    fn supports_tool_hooks(&self) -> bool {
        self.plugin_products
    }

    fn validate_snapshot(&self, snapshot: &ResolvedSnapshotEnvelope) -> Result<(), AppError> {
        let content = &snapshot.content;
        if let Some(supported) = &self.enabled_capabilities {
            for item in &content.enabled_capabilities {
                if !supported.contains(item.capability.id.as_ref()) {
                    return Err(unsupported(item.capability.id.as_ref()));
                }
            }
        }
        for (allowed, selected, feature) in [
            (self.skills, !content.skill_locks.is_empty(), "Skills"),
            (
                self.mcp,
                !content.mcp_tool_locks.is_empty()
                    || content.enabled_capabilities.iter().any(|entry| {
                        entry.contribution_lock.source_kind == ContributionSourceKind::McpBinding
                    }),
                "MCP",
            ),
            (
                self.plugin_products,
                content.enabled_capabilities.iter().any(|entry| {
                    entry.contribution_lock.source_kind
                        == ContributionSourceKind::PluginProductActiveRelease
                }),
                "Plugin Products",
            ),
        ] {
            if !allowed && selected {
                return Err(unsupported(feature));
            }
        }
        Ok(())
    }

    fn validate_session_extra(&self, extra: &Value) -> Result<(), AppError> {
        for (allowed, keys, feature) in [
            (
                self.skills,
                &["skills", "session_enabled_skills"][..],
                "session Skills",
            ),
            (
                self.mcp,
                &["mcp_server_ids", "mcp_servers", "selected_mcp_server_ids"][..],
                "session MCP",
            ),
        ] {
            if !allowed
                && keys.iter().any(|key| {
                    extra
                        .get(*key)
                        .is_some_and(|value| !value.as_array().is_some_and(Vec::is_empty))
                })
            {
                return Err(unsupported(feature));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_support_rejects_overlay_aliases() {
        let policy = RuntimeSupport::enabled_only([]);
        for key in [
            "skills",
            "session_enabled_skills",
            "mcp_server_ids",
            "mcp_servers",
            "selected_mcp_server_ids",
        ] {
            for value in [serde_json::json!(["selected"]), Value::Null, serde_json::json!({})] {
                let extra = serde_json::json!({key: value});
                assert!(policy.validate_session_extra(&extra).is_err(), "{extra}");
            }
            policy
                .validate_session_extra(&serde_json::json!({key: []}))
                .unwrap();
        }
    }
}
