//! The `PublicAgentConfig` — the persisted profile of one 对外伙伴 (public
//! companion). Stored as `public-agents/{id}/config.json`. Enterprise-service
//! shaped and deliberately DISJOINT from `CompanionProfileConfig`.

use nomifun_common::{
    AppError, KnowledgeBaseId, ProviderId, PublicAgentId, now_ms,
};
use serde::{Deserialize, Serialize};

/// Default day-level audit retention (see `crate::audit`).
pub const DEFAULT_AUDIT_RETENTION_DAYS: u32 = 30;

/// The model a public companion answers with. A tiny, self-contained shape
/// (not the desktop companion's `ModelConfig`) — this domain shares no config
/// types with `nomifun-companion`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PublicAgentModel {
    /// Canonical provider row id backing the model. `None` is the only
    /// unconfigured representation; empty-string ID sentinels are rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ProviderId>,
    /// Model label shown to the owner.
    pub model: String,
    /// Concrete model id sent to the provider. `None` falls back to `model`;
    /// an explicitly empty override is invalid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_model: Option<String>,
}

impl PublicAgentModel {
    /// Whether a usable model is configured.
    pub fn is_configured(&self) -> bool {
        self.provider_id.is_some() && !self.model.trim().is_empty()
    }

    fn validate(&self) -> Result<(), AppError> {
        let model = self.model.trim();
        if model != self.model {
            return Err(AppError::BadRequest("public-agent model must be trimmed".into()));
        }
        match (&self.provider_id, model.is_empty()) {
            (None, true) => {}
            (Some(_), false) => {}
            _ => {
                return Err(AppError::BadRequest(
                    "public-agent model requires provider_id and model together".into(),
                ));
            }
        }
        if self.provider_id.is_none() && self.use_model.is_some() {
            return Err(AppError::BadRequest(
                "public-agent use_model requires a configured provider".into(),
            ));
        }
        if let Some(use_model) = self.use_model.as_deref()
            && (use_model.is_empty() || use_model.trim() != use_model)
        {
            return Err(AppError::BadRequest(
                "public-agent use_model must be a non-empty trimmed value".into(),
            ));
        }
        Ok(())
    }
}

/// One public companion's full configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PublicAgentConfig {
    /// Canonical stable entity id (`pubagent_<uuid-v7>`).
    pub id: PublicAgentId,
    /// Display-only short number (`#1`, `#2`, …). Allocated by the registry from
    /// its private high-watermark so a deleted agent's number is never reused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// Owner-facing display name / brand.
    pub name: String,
    /// Opening / welcome message shown to strangers on first contact.
    pub greeting: String,
    /// Tone & style guidelines (free-text in P1; injected into the system prompt).
    pub tone: String,
    /// The model the agent answers with.
    pub model: PublicAgentModel,
    /// Bound platform knowledge bases (grounded retrieval source of truth). The
    /// runtime bakes these into the scoped knowledge tool so a turn can never
    /// widen the base set.
    pub knowledge_base_ids: Vec<KnowledgeBaseId>,
    /// Grounded (strict) mode: only answer from the bound knowledge bases; when
    /// nothing is found, politely decline / suggest escalation — never fabricate.
    pub grounded_mode: bool,
    /// Service policy / 服务守则 (business scope, forbidden topics, compliance
    /// tone). Owner-authored; injected as a hard system directive. Free-text in
    /// P1 (structured policy is P2).
    pub service_policy: String,
    /// Frozen reusable configuration. Runtime policy remains authoritative:
    /// public-service tool clamps and grounded restrictions cannot be relaxed
    /// by a preset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_preset: Option<nomifun_api_types::ResolvedPresetSnapshot>,
    /// Day-level audit retention (see `crate::audit`).
    pub audit_retention_days: u32,
    /// Whether the agent is live (serving) or paused.
    pub enabled: bool,
    /// Creation timestamp (epoch ms).
    pub created_at: i64,
}

impl PublicAgentConfig {
    /// Fresh agent with a generated id and sensible enterprise defaults
    /// (grounded ON — anti-hallucination is the safe default; enabled ON).
    pub fn new(name: &str) -> Self {
        Self {
            id: PublicAgentId::new(),
            seq: None,
            name: name.to_owned(),
            greeting: String::new(),
            tone: String::new(),
            model: PublicAgentModel::default(),
            knowledge_base_ids: Vec::new(),
            grounded_mode: true,
            service_policy: String::new(),
            applied_preset: None,
            audit_retention_days: DEFAULT_AUDIT_RETENTION_DAYS,
            enabled: true,
            created_at: now_ms(),
        }
    }

    /// Validate cross-field invariants not expressible by typed ID serde.
    pub(crate) fn validate(&self) -> Result<(), AppError> {
        if self.name.trim().is_empty() {
            return Err(AppError::BadRequest("public-agent name must not be empty".into()));
        }
        self.model.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_agent_has_enterprise_defaults() {
        let a = PublicAgentConfig::new("客服");
        assert!(PublicAgentId::parse(a.id.as_str()).is_ok());
        assert_eq!(a.name, "客服");
        assert!(a.grounded_mode, "grounded is the anti-hallucination default");
        assert!(a.enabled);
        assert_eq!(a.audit_retention_days, DEFAULT_AUDIT_RETENTION_DAYS);
        assert!(!a.model.is_configured());
        assert!(a.seq.is_none());
    }

    #[test]
    fn config_json_roundtrips_with_canonical_ids() {
        let provider_id = ProviderId::new();
        let kb_id = KnowledgeBaseId::new();
        let mut config = PublicAgentConfig::new("A");
        config.model = PublicAgentModel {
            provider_id: Some(provider_id.clone()),
            model: "model-a".into(),
            use_model: None,
        };
        config.knowledge_base_ids = vec![kb_id.clone()];

        let decoded: PublicAgentConfig =
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
        assert_eq!(decoded, config);
        assert_eq!(decoded.model.provider_id, Some(provider_id));
        assert_eq!(decoded.knowledge_base_ids, vec![kb_id]);
    }

    #[test]
    fn config_json_rejects_noncanonical_entity_ids() {
        let canonical = serde_json::to_value(PublicAgentConfig::new("A")).unwrap();
        for (field, invalid) in [
            ("id", serde_json::json!("pubagent_x")),
            ("id", serde_json::json!(42)),
        ] {
            let mut value = canonical.clone();
            value[field] = invalid;
            assert!(serde_json::from_value::<PublicAgentConfig>(value).is_err());
        }

        let mut bad_provider = canonical.clone();
        bad_provider["model"] = serde_json::json!({
            "provider_id": "prov_x",
            "model": "model-a"
        });
        assert!(serde_json::from_value::<PublicAgentConfig>(bad_provider).is_err());

        let mut bad_kb = canonical;
        bad_kb["knowledge_base_ids"] = serde_json::json!(["kb_x"]);
        assert!(serde_json::from_value::<PublicAgentConfig>(bad_kb).is_err());
    }

    #[test]
    fn model_rejects_empty_id_sentinel_and_partial_configuration() {
        assert!(
            serde_json::from_value::<PublicAgentModel>(serde_json::json!({
                "provider_id": "",
                "model": "model-a"
            }))
            .is_err()
        );
        let partial: PublicAgentModel =
            serde_json::from_value(serde_json::json!({ "model": "model-a" })).unwrap();
        assert!(partial.validate().is_err());
    }
}
