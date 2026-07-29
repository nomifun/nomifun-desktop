//! CRUD + validation service for customer-service agents, notes and bindings.
//!
//! Pure domain logic over `ICustomerServiceRepository`; plugin-id existence
//! for bindings is validated by the caller (route layer) against the channel
//! repository, so this crate never depends on the channel domain.

use std::sync::Arc;

use nomifun_common::{AppError, CsAgentId, CsNoteId, now_ms};
use nomifun_db::models::{CsAgentRow, CsChannelBindingRow, CsNoteRow, NewCsAgentRow};
use nomifun_db::{ICustomerServiceRepository, UpdateCsAgentParams};

/// Inclusive bounds for `cs_agents.max_concurrent`.
pub const MAX_CONCURRENT_RANGE: std::ops::RangeInclusive<i64> = 1..=64;
/// Default per-agent concurrency ceiling.
pub const DEFAULT_MAX_CONCURRENT: i64 = 8;
/// Default audit retention in days.
pub const DEFAULT_AUDIT_RETENTION_DAYS: i64 = 30;

/// Input for creating a customer-service agent.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct CreateCsAgentInput {
    pub name: String,
    #[serde(default)]
    pub greeting: String,
    #[serde(default)]
    pub persona: String,
    #[serde(default)]
    pub service_policy: String,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub knowledge_base_ids: Vec<String>,
    #[serde(default)]
    pub max_concurrent: Option<i64>,
    #[serde(default)]
    pub audit_retention_days: Option<i64>,
}

/// Patch input for updating a customer-service agent. Absent fields keep the
/// stored value; `provider_id`/`model` use double-Option to distinguish
/// "keep" from "clear".
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UpdateCsAgentInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub greeting: Option<String>,
    #[serde(default)]
    pub persona: Option<String>,
    #[serde(default)]
    pub service_policy: Option<String>,
    #[serde(default, with = "double_option")]
    pub provider_id: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    pub model: Option<Option<String>>,
    #[serde(default)]
    pub knowledge_base_ids: Option<Vec<String>>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub max_concurrent: Option<i64>,
    #[serde(default)]
    pub audit_retention_days: Option<i64>,
}

/// Serde helper: a JSON field that is absent → `None` (keep), present-null →
/// `Some(None)` (clear), present-value → `Some(Some(v))`.
mod double_option {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

/// Input for creating a customer-service note.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateCsNoteInput {
    /// `None` = shared across all agents.
    #[serde(default)]
    pub cs_agent_id: Option<String>,
    #[serde(default = "default_note_kind")]
    pub kind: String,
    pub content: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_note_kind() -> String {
    "faq".to_owned()
}

fn default_true() -> bool {
    true
}

/// CRUD + validation service over the customer-service repository.
pub struct CustomerServiceService {
    repo: Arc<dyn ICustomerServiceRepository>,
}

impl CustomerServiceService {
    pub fn new(repo: Arc<dyn ICustomerServiceRepository>) -> Self {
        Self { repo }
    }

    pub fn repo(&self) -> &Arc<dyn ICustomerServiceRepository> {
        &self.repo
    }

    // ── agents ───────────────────────────────────────────────────────

    pub async fn create_agent(&self, input: CreateCsAgentInput) -> Result<CsAgentRow, AppError> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(AppError::BadRequest("客服名称不能为空".into()));
        }
        let max_concurrent = input.max_concurrent.unwrap_or(DEFAULT_MAX_CONCURRENT);
        validate_max_concurrent(max_concurrent)?;
        let audit_retention_days =
            input.audit_retention_days.unwrap_or(DEFAULT_AUDIT_RETENTION_DAYS);
        if audit_retention_days < 1 {
            return Err(AppError::BadRequest(
                "audit_retention_days must be at least 1".into(),
            ));
        }
        let now = now_ms();
        let row = NewCsAgentRow {
            cs_agent_id: CsAgentId::new().into_string(),
            name: name.to_owned(),
            greeting: input.greeting,
            persona: input.persona,
            service_policy: input.service_policy,
            provider_id: normalize_optional(input.provider_id),
            model: normalize_optional(input.model),
            knowledge_base_ids: CsAgentRow::encode_knowledge_base_ids(&input.knowledge_base_ids),
            enabled: true,
            max_concurrent,
            audit_retention_days,
            created_at: now,
            updated_at: now,
        };
        Ok(self.repo.create_agent(&row).await?)
    }

    pub async fn get_agent(&self, cs_agent_id: &str) -> Result<CsAgentRow, AppError> {
        self.repo
            .get_agent(cs_agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("customer-service agent {cs_agent_id}")))
    }

    pub async fn list_agents(&self) -> Result<Vec<CsAgentRow>, AppError> {
        Ok(self.repo.list_agents().await?)
    }

    pub async fn update_agent(
        &self,
        cs_agent_id: &str,
        input: UpdateCsAgentInput,
    ) -> Result<CsAgentRow, AppError> {
        if let Some(name) = &input.name
            && name.trim().is_empty()
        {
            return Err(AppError::BadRequest("客服名称不能为空".into()));
        }
        if let Some(max_concurrent) = input.max_concurrent {
            validate_max_concurrent(max_concurrent)?;
        }
        if let Some(days) = input.audit_retention_days
            && days < 1
        {
            return Err(AppError::BadRequest(
                "audit_retention_days must be at least 1".into(),
            ));
        }
        let params = UpdateCsAgentParams {
            name: input.name.map(|value| value.trim().to_owned()),
            greeting: input.greeting,
            persona: input.persona,
            service_policy: input.service_policy,
            provider_id: input.provider_id.map(normalize_optional),
            model: input.model.map(normalize_optional),
            knowledge_base_ids: input
                .knowledge_base_ids
                .map(|ids| CsAgentRow::encode_knowledge_base_ids(&ids)),
            enabled: input.enabled,
            max_concurrent: input.max_concurrent,
            audit_retention_days: input.audit_retention_days,
        };
        Ok(self.repo.update_agent(cs_agent_id, &params, now_ms()).await?)
    }

    pub async fn delete_agent(&self, cs_agent_id: &str) -> Result<(), AppError> {
        Ok(self.repo.delete_agent(cs_agent_id).await?)
    }

    // ── bindings ─────────────────────────────────────────────────────

    /// Full replacement (PUT) of one agent's channel-bot bindings. The caller
    /// (route layer) must have validated the plugin ids against the channel
    /// repository; a plugin bound elsewhere is stolen (同 bot 重绑替换).
    pub async fn replace_bindings(
        &self,
        cs_agent_id: &str,
        channel_plugin_ids: Vec<String>,
    ) -> Result<Vec<CsChannelBindingRow>, AppError> {
        let mut deduped: Vec<String> = Vec::with_capacity(channel_plugin_ids.len());
        for id in channel_plugin_ids {
            if !deduped.contains(&id) {
                deduped.push(id);
            }
        }
        Ok(self
            .repo
            .replace_agent_bindings(cs_agent_id, &deduped, now_ms())
            .await?)
    }

    pub async fn list_bindings(
        &self,
        cs_agent_id: &str,
    ) -> Result<Vec<CsChannelBindingRow>, AppError> {
        Ok(self.repo.list_agent_bindings(cs_agent_id).await?)
    }

    /// The agent bound to `channel_plugin_id`, if any (channel seam query).
    pub async fn binding_for_plugin(
        &self,
        channel_plugin_id: &str,
    ) -> Result<Option<String>, AppError> {
        Ok(self
            .repo
            .binding_for_plugin(channel_plugin_id)
            .await?
            .map(|row| row.cs_agent_id))
    }

    // ── notes ────────────────────────────────────────────────────────

    pub async fn create_note(&self, input: CreateCsNoteInput) -> Result<CsNoteRow, AppError> {
        if input.content.trim().is_empty() {
            return Err(AppError::BadRequest("笔记内容不能为空".into()));
        }
        if let Some(agent_id) = &input.cs_agent_id {
            // A private note must belong to a live agent.
            self.get_agent(agent_id).await?;
        }
        let now = now_ms();
        let row = CsNoteRow {
            cs_note_id: CsNoteId::new().into_string(),
            cs_agent_id: input.cs_agent_id,
            kind: input.kind,
            content: input.content,
            enabled: input.enabled,
            created_at: now,
            updated_at: now,
        };
        Ok(self.repo.create_note(&row).await?)
    }

    /// Notes visible to `cs_agent_id` (shared + private), or every note when
    /// `None`.
    pub async fn list_notes(&self, cs_agent_id: Option<&str>) -> Result<Vec<CsNoteRow>, AppError> {
        Ok(self.repo.list_notes(cs_agent_id).await?)
    }

    pub async fn update_note(
        &self,
        cs_note_id: &str,
        kind: Option<&str>,
        content: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<CsNoteRow, AppError> {
        if let Some(content) = content
            && content.trim().is_empty()
        {
            return Err(AppError::BadRequest("笔记内容不能为空".into()));
        }
        Ok(self
            .repo
            .update_note(cs_note_id, kind, content, enabled, now_ms())
            .await?)
    }

    pub async fn delete_note(&self, cs_note_id: &str) -> Result<(), AppError> {
        Ok(self.repo.delete_note(cs_note_id).await?)
    }
}

fn validate_max_concurrent(value: i64) -> Result<(), AppError> {
    if !MAX_CONCURRENT_RANGE.contains(&value) {
        return Err(AppError::BadRequest(format!(
            "max_concurrent must be within {}..={}",
            MAX_CONCURRENT_RANGE.start(),
            MAX_CONCURRENT_RANGE.end()
        )));
    }
    Ok(())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::ChannelPluginId;
    use nomifun_db::SqliteCustomerServiceRepository;

    async fn service() -> (nomifun_db::Database, CustomerServiceService) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteCustomerServiceRepository::new(db.pool().clone()));
        (db, CustomerServiceService::new(repo))
    }

    fn agent_input(name: &str) -> CreateCsAgentInput {
        CreateCsAgentInput {
            name: name.into(),
            greeting: "您好".into(),
            persona: "耐心".into(),
            service_policy: "只答业务".into(),
            model: Some("model-a".into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_update_disable_agent() {
        let (_db, svc) = service().await;
        let agent = svc.create_agent(agent_input("小客服")).await.unwrap();
        assert_eq!(agent.max_concurrent, DEFAULT_MAX_CONCURRENT);
        assert!(agent.enabled);

        let updated = svc
            .update_agent(
                &agent.cs_agent_id,
                UpdateCsAgentInput {
                    name: Some("  改名  ".into()),
                    enabled: Some(false),
                    max_concurrent: Some(2),
                    knowledge_base_ids: Some(vec!["0190f5fe-7c00-7a00-8000-0000000000aa".into()]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "改名");
        assert!(!updated.enabled);
        assert_eq!(updated.max_concurrent, 2);
        assert_eq!(updated.knowledge_base_ids_vec().len(), 1);
    }

    #[tokio::test]
    async fn create_agent_validation_errors() {
        let (_db, svc) = service().await;
        assert!(matches!(
            svc.create_agent(agent_input("   ")).await.unwrap_err(),
            AppError::BadRequest(_)
        ));
        let mut input = agent_input("ok");
        input.max_concurrent = Some(0);
        assert!(matches!(svc.create_agent(input).await.unwrap_err(), AppError::BadRequest(_)));
        let mut input = agent_input("ok");
        input.max_concurrent = Some(65);
        assert!(matches!(svc.create_agent(input).await.unwrap_err(), AppError::BadRequest(_)));
        assert!(matches!(
            svc.update_agent(
                "0190f5fe-7c00-7a00-8000-000000000001",
                UpdateCsAgentInput { max_concurrent: Some(100), ..Default::default() }
            )
            .await
            .unwrap_err(),
            AppError::BadRequest(_)
        ));
    }

    #[tokio::test]
    async fn update_input_double_option_semantics() {
        // absent → keep; null → clear; value → set.
        let json = r#"{"model": null}"#;
        let input: UpdateCsAgentInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.model, Some(None));
        assert_eq!(input.provider_id, None);

        let json = r#"{"provider_id": "0190f5fe-7c00-7a00-8000-000000000001"}"#;
        let input: UpdateCsAgentInput = serde_json::from_str(json).unwrap();
        assert_eq!(
            input.provider_id,
            Some(Some("0190f5fe-7c00-7a00-8000-000000000001".into()))
        );

        let (_db, svc) = service().await;
        let agent = svc.create_agent(agent_input("m")).await.unwrap();
        assert_eq!(agent.model.as_deref(), Some("model-a"));
        let updated = svc
            .update_agent(
                &agent.cs_agent_id,
                serde_json::from_str(r#"{"model": null}"#).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(updated.model, None, "explicit null clears the model");
        let kept = svc
            .update_agent(&agent.cs_agent_id, serde_json::from_str(r#"{}"#).unwrap())
            .await
            .unwrap();
        assert_eq!(kept.model, None, "absent field keeps the stored value");
    }

    #[tokio::test]
    async fn binding_replacement_is_unique_per_bot() {
        let (_db, svc) = service().await;
        let agent_a = svc.create_agent(agent_input("A")).await.unwrap();
        let agent_b = svc.create_agent(agent_input("B")).await.unwrap();
        let plugin = ChannelPluginId::new().into_string();

        svc.replace_bindings(&agent_a.cs_agent_id, vec![plugin.clone(), plugin.clone()])
            .await
            .unwrap();
        let bindings = svc.list_bindings(&agent_a.cs_agent_id).await.unwrap();
        assert_eq!(bindings.len(), 1, "duplicate ids in one PUT are deduped");

        // Rebinding the same bot to B replaces A's binding.
        svc.replace_bindings(&agent_b.cs_agent_id, vec![plugin.clone()]).await.unwrap();
        assert_eq!(
            svc.binding_for_plugin(&plugin).await.unwrap().as_deref(),
            Some(agent_b.cs_agent_id.as_str())
        );
        assert!(svc.list_bindings(&agent_a.cs_agent_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn notes_scope_and_validation() {
        let (_db, svc) = service().await;
        let agent = svc.create_agent(agent_input("A")).await.unwrap();

        assert!(matches!(
            svc.create_note(CreateCsNoteInput {
                cs_agent_id: None,
                kind: "faq".into(),
                content: "  ".into(),
                enabled: true,
            })
            .await
            .unwrap_err(),
            AppError::BadRequest(_)
        ));
        // A private note must name a live agent.
        assert!(matches!(
            svc.create_note(CreateCsNoteInput {
                cs_agent_id: Some("0190f5fe-7c00-7a00-8000-0000000000ff".into()),
                kind: "faq".into(),
                content: "x".into(),
                enabled: true,
            })
            .await
            .unwrap_err(),
            AppError::NotFound(_)
        ));

        let shared = svc
            .create_note(CreateCsNoteInput {
                cs_agent_id: None,
                kind: "faq".into(),
                content: "退货政策".into(),
                enabled: true,
            })
            .await
            .unwrap();
        let private = svc
            .create_note(CreateCsNoteInput {
                cs_agent_id: Some(agent.cs_agent_id.clone()),
                kind: "script".into(),
                content: "话术".into(),
                enabled: true,
            })
            .await
            .unwrap();

        // Merged query: shared + own private.
        let visible = svc.list_notes(Some(&agent.cs_agent_id)).await.unwrap();
        assert_eq!(visible.len(), 2);

        let updated = svc
            .update_note(&private.cs_note_id, None, Some("新话术"), Some(false))
            .await
            .unwrap();
        assert_eq!(updated.content, "新话术");
        assert!(!updated.enabled);

        svc.delete_note(&shared.cs_note_id).await.unwrap();
        assert!(matches!(
            svc.delete_note(&shared.cs_note_id).await.unwrap_err(),
            AppError::NotFound(_)
        ));
    }
}
