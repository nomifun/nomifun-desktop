//! `PublicAgentService` — bundles the roster [`PublicAgentRegistry`] with the
//! per-agent [`crate::audit`] log and resolves data-dir paths. This is the
//! single handle the API routes and the runtime provider talk to.

use std::path::PathBuf;
use std::sync::Arc;

use nomifun_common::{AppError, ProviderId, PublicAgentId};
use serde_json::Value;

use crate::audit::{self, AuditEntry, AuditPage, AuditQuery};
use crate::config::PublicAgentConfig;
use crate::registry::PublicAgentRegistry;

pub struct PublicAgentService {
    registry: Arc<PublicAgentRegistry>,
    dir: PathBuf,
}

impl PublicAgentService {
    /// Scan `{data_dir}/public-agents/` into a live service.
    pub fn start(data_dir: &std::path::Path) -> Arc<Self> {
        let dir = data_dir.join(crate::PUBLIC_AGENTS_REL_DIR);
        Arc::new(Self {
            registry: Arc::new(PublicAgentRegistry::scan(dir.clone())),
            dir,
        })
    }

    fn agent_dir(&self, id: &PublicAgentId) -> PathBuf {
        self.dir.join(id.as_str())
    }

    // ---- roster CRUD ----

    pub async fn list(&self) -> Vec<PublicAgentConfig> {
        self.registry.list().await
    }

    pub async fn get(&self, id: &str) -> Result<PublicAgentConfig, AppError> {
        let id = parse_public_agent_id(id)?;
        self.registry
            .get(&id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("public agent {id} not found")))
    }

    pub async fn exists(&self, id: &str) -> bool {
        let Ok(id) = PublicAgentId::parse(id) else {
            return false;
        };
        self.registry.exists(&id).await
    }

    pub async fn create(&self, name: &str) -> Result<PublicAgentConfig, AppError> {
        let created = self.registry.create(name).await?;
        self.record_event(created.id.as_str(), "lifecycle", "created").await;
        Ok(created)
    }

    /// RFC 7396 merge-patch. Logs a lifecycle audit event when `enabled` flips
    /// (owner-visible change trail).
    pub async fn patch(&self, id: &str, patch: Value) -> Result<PublicAgentConfig, AppError> {
        let id = parse_public_agent_id(id)?;
        let prev_enabled = self.registry.get(&id).await.map(|a| a.enabled);
        let next = self.registry.patch(&id, patch).await?;
        if let Some(prev) = prev_enabled {
            if prev != next.enabled {
                let detail = if next.enabled { "enabled" } else { "disabled" };
                self.record_event(id.as_str(), "lifecycle", detail).await;
            }
        }
        Ok(next)
    }

    /// Apply a resolved preset while preserving the public companion's brand,
    /// greeting, service policy, audit history and serving state. Security
    /// clamps are enforced later by the agent factory and are never sourced
    /// from the preset.
    pub async fn apply_preset_snapshot(
        &self,
        id: &str,
        snapshot: nomifun_api_types::ResolvedPresetSnapshot,
    ) -> Result<PublicAgentConfig, AppError> {
        if snapshot.target != nomifun_api_types::PresetTarget::PublicCompanion {
            return Err(AppError::BadRequest(
                "preset snapshot target must be public_companion".into(),
            ));
        }
        let mut patch = serde_json::json!({ "applied_preset": snapshot });
        if let Some(model) = patch
            .get("applied_preset")
            .and_then(|value| value.get("resolved_model"))
            .filter(|value| !value.is_null())
        {
            if let (Some(provider_id), Some(model_name)) = (
                model.get("provider_id").and_then(Value::as_str),
                model.get("model").and_then(Value::as_str),
            ) {
                patch["model"] = serde_json::json!({
                    "provider_id": provider_id,
                    "model": model_name,
                    "use_model": model_name,
                });
            }
        }
        if let Some(snapshot) = patch.get("applied_preset").cloned() {
            if let Some(ids) = snapshot.get("knowledge_base_ids") {
                patch["knowledge_base_ids"] = ids.clone();
            }
            if snapshot
                .get("knowledge_policy")
                .and_then(|policy| policy.get("grounded"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                // A strict preset can tighten a public companion. A non-strict
                // preset may never weaken an existing grounded service.
                patch["grounded_mode"] = Value::Bool(true);
            }
        }
        self.patch(id, patch).await
    }

    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        let id = parse_public_agent_id(id)?;
        self.registry.remove(&id).await.map(|_| ())
    }

    // ---- provider usage ----

    /// Report every public agent whose model is backed by `provider_id`
    /// (feeds the provider-deletion guard). Each hit is labelled by the agent
    /// name and deep-links via its id.
    pub async fn providers_in_use(&self, provider_id: &str) -> Vec<nomifun_common::ProviderUsage> {
        let Ok(provider_id) = ProviderId::parse(provider_id) else {
            return Vec::new();
        };
        self.list()
            .await
            .into_iter()
            .filter(|a| a.model.provider_id.as_ref() == Some(&provider_id))
            .map(|a| nomifun_common::ProviderUsage {
                feature: nomifun_common::ProviderUsageFeature::PublicCompanion,
                label: a.name,
                target_id: Some(a.id.into_string()),
            })
            .collect()
    }

    // ---- audit ----

    /// Record an inbound turn (best-effort; never fails the caller). Retention
    /// is read from the agent's own config; unknown agent → no-op.
    pub async fn record_turn(&self, id: &str, surface: &str, platform: Option<&str>, text: &str) {
        let Ok(id) = PublicAgentId::parse(id) else { return };
        let Some(cfg) = self.registry.get(&id).await else { return };
        let entry = AuditEntry::turn(surface, platform.map(str::to_owned), text);
        if let Err(e) = audit::append(&self.agent_dir(&id), &entry, cfg.audit_retention_days) {
            tracing::warn!(error = %e, id = %id, "public-agent audit append failed");
        }
    }

    /// Record a lifecycle / config event (best-effort).
    pub async fn record_event(&self, id: &str, kind: &str, detail: impl Into<String>) {
        let Ok(id) = PublicAgentId::parse(id) else { return };
        let retention = self.registry.get(&id).await.map(|a| a.audit_retention_days).unwrap_or(0);
        let entry = AuditEntry::event(kind, detail);
        if let Err(e) = audit::append(&self.agent_dir(&id), &entry, retention) {
            tracing::warn!(error = %e, id = %id, "public-agent audit event append failed");
        }
    }

    /// Search / paginate the audit log. Errors only if the agent is unknown.
    pub async fn search_audit(&self, id: &str, query: AuditQuery) -> Result<AuditPage, AppError> {
        let id = parse_public_agent_id(id)?;
        if !self.registry.exists(&id).await {
            return Err(AppError::NotFound(format!("public agent {id} not found")));
        }
        Ok(audit::search(&self.agent_dir(&id), &query))
    }

    /// Delete audit day-files older than `older_than_days`; returns the count.
    pub async fn delete_audit(&self, id: &str, older_than_days: u32) -> Result<usize, AppError> {
        let id = parse_public_agent_id(id)?;
        if !self.registry.exists(&id).await {
            return Err(AppError::NotFound(format!("public agent {id} not found")));
        }
        Ok(audit::delete_older_than(&self.agent_dir(&id), older_than_days))
    }
}

fn parse_public_agent_id(id: &str) -> Result<PublicAgentId, AppError> {
    PublicAgentId::parse(id)
        .map_err(|error| AppError::BadRequest(format!("invalid public-agent id: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_patch_audit_flow() {
        let d = tempfile::tempdir().unwrap();
        let svc = PublicAgentService::start(d.path());
        let a = svc.create("客服").await.unwrap();
        assert!(a.enabled);

        // A turn is audited under the agent.
        svc.record_turn(a.id.as_str(), "channel", Some("telegram"), "请问怎么退货").await;
        let page = svc
            .search_audit(a.id.as_str(), AuditQuery { limit: 50, ..Default::default() })
            .await
            .unwrap();
        assert!(page.entries.iter().any(|e| e.kind == "turn" && e.detail == "请问怎么退货"));
        // create() logged a lifecycle event too.
        assert!(page.entries.iter().any(|e| e.kind == "lifecycle" && e.detail == "created"));

        // Disabling logs a lifecycle event.
        let patched = svc.patch(a.id.as_str(), serde_json::json!({ "enabled": false })).await.unwrap();
        assert!(!patched.enabled);
        let page2 = svc
            .search_audit(a.id.as_str(), AuditQuery { limit: 50, kind: Some("lifecycle".into()), ..Default::default() })
            .await
            .unwrap();
        assert!(page2.entries.iter().any(|e| e.detail == "disabled"));

        // Unknown agent → NotFound on search.
        assert!(svc.search_audit("pubagent_nope", AuditQuery::default()).await.is_err());
    }

    #[tokio::test]
    async fn providers_in_use_detects_public_agent_model() {
        let d = tempfile::tempdir().unwrap();
        let svc = PublicAgentService::start(d.path());
        let a = svc.create("客服").await.unwrap();
        let provider_id = ProviderId::new();
        svc.patch(
            a.id.as_str(),
            serde_json::json!({"model":{"provider_id":provider_id,"model":"m"}}),
        )
        .await
        .unwrap();

        let hits = svc.providers_in_use(provider_id.as_str()).await;
        assert!(hits.iter().any(|u| u.label == "客服" && u.target_id.as_deref() == Some(a.id.as_str())));
        assert!(svc.providers_in_use("prov_none").await.is_empty());
    }
}
