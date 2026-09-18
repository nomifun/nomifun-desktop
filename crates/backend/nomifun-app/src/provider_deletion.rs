//! App-layer aggregation of every subsystem's provider-in-use scan.
//!
//! `nomifun-app` is the only layer that sees the companion, customer-service,
//! workshop and Agent Execution subsystems at once, so the cross-subsystem
//! [`ProviderDeletionCoordinator`](nomifun_system::provider_deletion::ProviderDeletionCoordinator)
//! is implemented here and injected into `ProviderService` (see
//! `router::state::build_system_state`). Deletion then refuses an in-use provider
//! (409 `PROVIDER_IN_USE`). The provider repository owns the deletion-time
//! logical-reference guards and soft-reference cleanup in one transaction; this
//! coordinator exists only for friendly, labeled product errors.

use std::sync::Arc;

use nomifun_common::{
    AppError, ProviderId, ProviderLifecycleBarrier, ProviderUsage, ProviderUsageFeature,
};
use nomifun_db::{
    IAgentExecutionRepository, IAgentExecutionTemplateRepository, SqlitePool,
};
use nomifun_system::provider_deletion::ProviderDeletionCoordinator;

/// Aggregates every subsystem's provider-in-use scan behind the single
/// `ProviderDeletionCoordinator` hook `ProviderService::delete` calls.
pub struct AppProviderDeletionCoordinator {
    pub provider_lifecycle: Arc<ProviderLifecycleBarrier>,
    pub companion: Arc<nomifun_companion::CompanionService>,
    pub customer_service: Arc<nomifun_customer_service::CustomerServiceService>,
    pub workshop: Arc<nomifun_workshop::WorkshopService>,
    pub execution_repo: Arc<dyn IAgentExecutionRepository>,
    pub execution_template_repo: Arc<dyn IAgentExecutionTemplateRepository>,
    pub pool: SqlitePool,
}

#[async_trait::async_trait]
impl ProviderDeletionCoordinator for AppProviderDeletionCoordinator {
    fn provider_lifecycle_barrier(&self) -> Option<Arc<ProviderLifecycleBarrier>> {
        Some(self.provider_lifecycle.clone())
    }

    async fn cleanup_soft_references(&self, provider_id: &str) -> Result<(), AppError> {
        self.workshop
            .clear_provider_references_under_lifecycle_write_guard(provider_id)
            .await
    }

    async fn prepare_soft_model_cleanup(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<nomifun_db::ProviderModelCleanupPlan, AppError> {
        self.workshop
            .plan_provider_model_cleanup_under_lifecycle_write_guard(provider_id, model)
            .await
    }

    async fn usages(&self, provider_id: &str) -> Result<Vec<ProviderUsage>, AppError> {
        ProviderId::parse(provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?;

        let mut out = Vec::new();
        out.extend(self.companion.providers_in_use(provider_id).await);
        out.extend(self.customer_service.providers_in_use(provider_id).await);

        // Current Agent revisions are hard provider bindings. Session model
        // authority is frozen through these exact revision payloads; the
        // retired Conversation table is never consulted.
        let agents: Vec<(String, String)> = nomifun_db::sqlx::query_as(
            "SELECT DISTINCT preset.preset_id, \
                    COALESCE(json_extract(preset.display_json, '$.name'), preset.preset_id) \
             FROM agent_presets preset \
             JOIN agent_preset_revisions revision \
               ON revision.preset_id = preset.preset_id \
              AND revision.revision_no = preset.current_stable_revision \
             JOIN json_tree(revision.payload_json) route \
               ON route.key = 'provider_id' AND route.value = ? \
             WHERE preset.retired_at_ms IS NULL",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| AppError::Internal(format!("scan Agent model routes: {error}")))?;
        for (id, name) in agents {
            out.push(ProviderUsage {
                feature: ProviderUsageFeature::Agent,
                label: name,
                target_id: Some(id),
            });
        }

        // Current Agent Execution participants remain live provider bindings
        // even after completed/failed settlement: retry and adopt can reopen
        // those aggregates. Only cancelled or tombstoned executions are
        // permanently inert and therefore stop blocking provider deletion.
        let executions = self
            .execution_repo
            .list_reopenable_provider_usages(provider_id)
            .await
            .map_err(|e| AppError::Internal(format!("scan Agent Executions: {e}")))?;
        for (id, goal) in executions {
            out.push(ProviderUsage {
                feature: ProviderUsageFeature::AgentExecution,
                label: goal,
                target_id: Some(id),
            });
        }
        let templates = self
            .execution_template_repo
            .list_templates_using_provider(provider_id)
            .await
            .map_err(|e| AppError::Internal(format!("scan Agent Execution templates: {e}")))?;
        for (id, name) in templates {
            out.push(ProviderUsage {
                feature: ProviderUsageFeature::AgentExecution,
                label: name,
                target_id: Some(id),
            });
        }
        Ok(out)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::{
        AdaptationPolicy, AgentExecutionEventKind, AgentExecutionStatus, DecisionPolicy,
        DelegationPolicy, ProviderUsageFeature,
    };
    use nomifun_db::{
        IAgentExecutionRepository, IAgentExecutionTemplateRepository,
        IClientPreferenceRepository, IProviderRepository, SqliteAgentExecutionRepository,
        SqliteAgentExecutionTemplateRepository, SqliteClientPreferenceRepository,
        SqliteProviderRepository, init_database_memory,
    };
    use std::sync::Arc;

    const NOMI_AGENT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000114";

    /// Minimal completer so `CompanionService::start` needs no live provider — the
    /// deletion-guard tests never trigger a distillation call.
    struct NoopCompleter;

    #[async_trait::async_trait]
    impl nomifun_companion::learner::CompanionCompleter for NoopCompleter {
        async fn complete(
            &self,
            _provider_id: &str,
            _model: &str,
            _system: &str,
            _user: &str,
            _max_tokens: u32,
        ) -> Result<String, nomifun_common::AppError> {
            Ok("{}".into())
        }
    }

    /// Build a real coordinator over an in-memory DB + tempdir-backed companion /
    /// customer-service services — mirrors the app's `build_system_state` construction
    /// (minus the live provider completer). Returns the `Database` so its in-memory
    /// pool outlives the coordinator.
    async fn coordinator(
        dir: &std::path::Path,
    ) -> (AppProviderDeletionCoordinator, Arc<nomifun_db::Database>) {
        let db = Arc::new(init_database_memory().await.unwrap());
        let installation_owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let credentials_encrypted = nomifun_common::encrypt_string(
            r#"{"api_keys":["test-only"]}"#,
            &[0x42; 32],
        )
        .unwrap();
        for provider_id in [
            "0190f5fe-7c00-7a00-8000-000000000021",
            "0190f5fe-7c00-7a00-8000-000000000022",
            "0190f5fe-7c00-7a00-8000-000000000023",
            "0190f5fe-7c00-7a00-8000-000000000024",
            "0190f5fe-7c00-7a00-8000-000000000025",
            "0190f5fe-7c00-7a00-8000-000000000026",
        ] {
            nomifun_db::sqlx::query(
                "INSERT INTO providers (\
                    provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
                    created_at, updated_at\
                 ) VALUES (?, 'openai', ?, 'https://example.invalid', 'bearer', ?, \
                            1, 1, 1)",
            )
            .bind(provider_id)
            .bind(provider_id)
            .bind(&credentials_encrypted)
            .execute(db.pool())
            .await
            .unwrap();
        }
        let companion = nomifun_companion::CompanionService::start(
            dir,
            Arc::new(nomifun_realtime::BroadcastEventBus::new(16)),
            &installation_owner,
            Arc::new(NoopCompleter),
            Arc::new(nomifun_skill_library::skill_service::resolve_skill_paths(dir, dir)),
        )
        .await
        .unwrap();
        let customer_service = Arc::new(nomifun_customer_service::CustomerServiceService::new(
            Arc::new(nomifun_db::SqliteCustomerServiceRepository::new(db.pool().clone())),
        ));
        let execution_repo: Arc<dyn IAgentExecutionRepository> =
            Arc::new(SqliteAgentExecutionRepository::new(db.pool().clone()));
        let execution_template_repo: Arc<dyn IAgentExecutionTemplateRepository> =
            Arc::new(SqliteAgentExecutionTemplateRepository::new(db.pool().clone()));
        let provider_lifecycle = Arc::new(ProviderLifecycleBarrier::new());
        (
            AppProviderDeletionCoordinator {
                provider_lifecycle: provider_lifecycle.clone(),
                companion,
                customer_service,
                workshop: nomifun_workshop::WorkshopService::start_with_provider_lifecycle(
                    dir,
                    Arc::new(nomifun_db::SqliteWorkshopRepository::new(db.pool().clone())),
                    provider_lifecycle,
                ),
                execution_repo,
                execution_template_repo,
                pool: db.pool().clone(),
            },
            db,
        )
    }

    #[tokio::test]
    async fn usages_rejects_empty_provider_id() {
        let dir = tempfile::tempdir().unwrap();
        let (coord, _db) = coordinator(dir.path()).await;
        assert!(matches!(coord.usages("").await, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn aggregates_reopenable_agent_execution_usage() {
        let dir = tempfile::tempdir().unwrap();
        let (coord, db) = coordinator(dir.path()).await;
        let installation_owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let execution = coord
            .execution_repo
            .create_execution_with_participants(
                &installation_owner,
                &nomifun_db::CreateAgentExecutionParams {
                    goal: "正在使用受保护模型".into(),
                    status: AgentExecutionStatus::Paused,
                    adaptation_policy: AdaptationPolicy::Fixed,
                    decision_policy: DecisionPolicy::Automatic,
                    delegation_policy: DelegationPolicy::Automatic,
                    max_parallel: 1,
                    work_dir: None,
                    lead_conversation_id: None,
                    initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
                },
                &[nomifun_db::NewAgentExecutionParticipant {
                    participant_id: nomifun_common::generate_id(),
                    source_agent_id: NOMI_AGENT_ID.into(),
                    preset_id: None,
                    preset_revision: None,
                    agent_snapshot: None,
                    provider_id: Some("0190f5fe-7c00-7a00-8000-000000000022".into()),
                    model: Some("model_in_use".into()),
                    role: None,
                    capability: None,
                    constraints: None,
                    description: None,
                    system_prompt: None,
                    enabled_skills: "[]".into(),
                    disabled_builtin_skills: "[]".into(),
                    sort_order: 0,
                }],
                &nomifun_db::NewAgentExecutionEvent {
                    event_type: AgentExecutionEventKind::Created,
                    step_id: None,
                    attempt_id: None,
                    actor: nomifun_common::AgentExecutionActor::user(&installation_owner),
                    payload: "{}".into(),
                },
            )
            .await
            .unwrap();

        let usages = coord.usages("0190f5fe-7c00-7a00-8000-000000000022").await.unwrap();
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].feature, ProviderUsageFeature::AgentExecution);
        assert_eq!(usages[0].label, "正在使用受保护模型");
        assert_eq!(
            usages[0].target_id.as_deref(),
            Some(execution.execution_id.as_str())
        );

        sqlx::query("UPDATE agent_executions SET status = 'running' WHERE execution_id = ?")
            .bind(&execution.execution_id)
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE agent_executions SET status = 'completed' WHERE execution_id = ?")
            .bind(&execution.execution_id)
            .execute(db.pool())
            .await
            .unwrap();
        assert_eq!(
            coord.usages("0190f5fe-7c00-7a00-8000-000000000022").await.unwrap().len(),
            1,
            "a completed execution can be reopened by retry/adopt and must retain its provider binding"
        );

        sqlx::query("UPDATE agent_executions SET status = 'running' WHERE execution_id = ?")
            .bind(&execution.execution_id)
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE agent_executions SET status = 'cancelled' WHERE execution_id = ?")
            .bind(&execution.execution_id)
            .execute(db.pool())
            .await
            .unwrap();
        assert!(
            coord.usages("0190f5fe-7c00-7a00-8000-000000000022").await.unwrap().is_empty(),
            "a cancelled execution can never reopen and must not retain a live provider binding"
        );
    }

    #[tokio::test]
    async fn aggregates_saved_agent_execution_template_usage() {
        let dir = tempfile::tempdir().unwrap();
        let (coord, db) = coordinator(dir.path()).await;
        let installation_owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let template = coord
            .execution_template_repo
            .create_template(
                &installation_owner,
                &nomifun_db::CreateAgentExecutionTemplateParams {
                    name: "长期协作方案".to_owned(),
                    description: None,
                    max_parallel: Some(2),
                    work_dir: None,
                    context: None,
                    participants: vec![nomifun_db::NewAgentExecutionTemplateParticipant {
                        template_participant_id: nomifun_common::generate_id(),
                        source_agent_id: NOMI_AGENT_ID.to_owned(),
                        preset_id: None,
                        preset_revision: None,
                        agent_snapshot: None,
                        provider_id: Some("0190f5fe-7c00-7a00-8000-000000000025".to_owned()),
                        model: Some("model_template".to_owned()),
                        role: None,
                        capability: None,
                        constraints: None,
                        description: None,
                        system_prompt: None,
                        enabled_skills: "[]".to_owned(),
                        disabled_builtin_skills: "[]".to_owned(),
                        sort_order: 0,
                    }],
                },
            )
            .await
            .unwrap();

        let usages = coord.usages("0190f5fe-7c00-7a00-8000-000000000025").await.unwrap();
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].feature, ProviderUsageFeature::AgentExecution);
        assert_eq!(usages[0].label, "长期协作方案");
        assert_eq!(
            usages[0].target_id.as_deref(),
            Some(template.template.execution_template_id.as_str())
        );
        assert!(
            coord
                .usages("0190f5fe-7c00-7a00-8000-000000000024")
                .await
                .unwrap()
                .is_empty(),
            "the frozen preset snapshot is audit data; the concrete participant row is the only live provider binding"
        );
    }

    #[tokio::test]
    async fn provider_delete_atomically_strips_failover_queue_entry() {
        use nomifun_conversation::model_failover::{
            get_global_failover_config, set_global_failover_config,
        };
        let dir = tempfile::tempdir().unwrap();
        // 协调器只为把删除守卫接上来;这条用例断言的是删除本身的事务效果。
        let (_coord, db) = coordinator(dir.path()).await;
        let client_prefs: Arc<dyn IClientPreferenceRepository> =
            Arc::new(SqliteClientPreferenceRepository::new(db.pool().clone()));
        let mut cfg = get_global_failover_config(&client_prefs).await;
        cfg.queue = vec![
            nomifun_common::ProviderWithModel {
                provider_id: "0190f5fe-7c00-7a00-8000-000000000026".into(),
                model: "m".into(),
                use_model: None,
            },
            nomifun_common::ProviderWithModel {
                provider_id: "0190f5fe-7c00-7a00-8000-000000000023".into(),
                model: "m2".into(),
                use_model: None,
            },
        ];
        set_global_failover_config(&client_prefs, &cfg)
            .await
            .unwrap();

        SqliteProviderRepository::new(db.pool().clone())
            .delete("0190f5fe-7c00-7a00-8000-000000000026")
            .await
            .unwrap();
        let after = get_global_failover_config(&client_prefs).await;
        assert_eq!(after.queue.len(), 1);
        assert_eq!(after.queue[0].provider_id, "0190f5fe-7c00-7a00-8000-000000000023");
    }

    #[tokio::test]
    async fn active_agent_revision_is_a_hard_provider_binding() {
        let dir = tempfile::tempdir().unwrap();
        let (coord, db) = coordinator(dir.path()).await;
        let preset_id = nomifun_common::generate_id();
        nomifun_db::sqlx::query(
            "INSERT INTO agent_presets \
             (preset_id, owner_ref_json, source_json, display_json, current_stable_revision, created_at) \
             VALUES (?, '{}', '{}', '{\"name\":\"受保护 Agent\"}', 1, 1)",
        )
        .bind(&preset_id)
        .execute(db.pool())
        .await
        .unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO agent_preset_revisions \
             (revision_id, preset_id, revision_no, schema_version, payload_json, revision_digest, created_by, created_at) \
             VALUES (?, ?, 1, '1.0.0', ?, ?, 'owner', 1)",
        )
        .bind(nomifun_common::generate_id())
        .bind(&preset_id)
        .bind(serde_json::json!({
            "chat_route_records": {
                "agent_chat": { "primary": {
                    "provider_id": "0190f5fe-7c00-7a00-8000-000000000021",
                    "model": "catalog-name"
                }}
            }
        }).to_string())
        .bind("a".repeat(64))
        .execute(db.pool())
        .await
        .unwrap();

        let usages = coord.usages("0190f5fe-7c00-7a00-8000-000000000021").await.unwrap();
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].feature, ProviderUsageFeature::Agent);
        assert_eq!(usages[0].label, "受保护 Agent");
        assert_eq!(usages[0].target_id, Some(preset_id));
    }

    #[tokio::test]
    async fn provider_service_delete_cleans_workshop_soft_references() {
        let dir = tempfile::tempdir().unwrap();
        let (coord, db) = coordinator(dir.path()).await;
        let workshop = coord.workshop.clone();
        let deleted_provider_id = "0190f5fe-7c00-7a00-8000-000000000026";
        let surviving_provider_id = "0190f5fe-7c00-7a00-8000-000000000023";
        for (provider_id, model) in [
            (deleted_provider_id, "delete-me"),
            (surviving_provider_id, "keep-me"),
        ] {
            nomifun_db::sqlx::query(
                "INSERT INTO provider_models \
                    (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
                 VALUES (?, ?, 1, 0, NULL, 1, 1)",
            )
            .bind(provider_id)
            .bind(model)
            .execute(db.pool())
            .await
            .unwrap();
        }

        let project = workshop
            .create_creative_project(Some("provider cleanup integration".into()))
            .await
            .unwrap();
        let mut document = nomifun_workshop::CreativeProjectDocument::empty(project.project_id.clone());
        for (id, provider_id, model, prompt) in [
            ("delete-config", deleted_provider_id, "delete-me", "preserve"),
            ("keep-config", surviving_provider_id, "keep-me", ""),
        ] {
            document.nodes.push(
                serde_json::from_value(serde_json::json!({
                    "id": id,
                    "type": "config",
                    "position": { "x": 0, "y": 0 },
                    "size": { "width": 320, "height": 240 },
                    "groupId": null,
                    "zIndex": 1,
                    "locked": false,
                    "data": {
                        "task": "image_generation",
                        "capability": "t2i",
                        "providerId": provider_id,
                        "model": model,
                        "prompt": prompt,
                        "negativePrompt": "",
                        "parameters": {},
                        "inputAssetIds": [],
                        "taskId": null,
                        "resultAssetIds": [],
                        "status": "idle",
                        "errorMessage": null
                    }
                }))
                .unwrap(),
            );
        }
        workshop
            .save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();

        let provider_repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
        let provider_service = nomifun_system::ProviderService::new(
            provider_repo.clone(),
            Arc::new(nomifun_db::SqliteProviderModelRepository::new(db.pool().clone())),
            Arc::new(nomifun_db::SqliteProviderModelCapabilityRepository::new(
                db.pool().clone(),
            )),
            Arc::new(nomifun_db::SqliteProviderConnectionRepository::new(
                db.pool().clone(),
            )),
            [0u8; 32],
        )
        .with_deletion_coordinator(Arc::new(coord));
        provider_service.delete(deleted_provider_id).await.unwrap();

        assert!(provider_repo.find_by_id(deleted_provider_id).await.unwrap().is_none());
        let cleaned = workshop.get_creative_project(&project.project_id).await.unwrap();
        assert_eq!(cleaned.project.revision, "3");
        let nomifun_workshop::creative_studio::CreativeNodeData::Config(deleted) =
            &cleaned.document.nodes[0].data
        else {
            panic!("expected deleted config node")
        };
        assert_eq!(deleted.provider_id, None);
        assert_eq!(deleted.model, None);
        assert_eq!(deleted.prompt, "preserve");
        let nomifun_workshop::creative_studio::CreativeNodeData::Config(surviving) =
            &cleaned.document.nodes[1].data
        else {
            panic!("expected surviving config node")
        };
        assert_eq!(surviving.provider_id.as_deref(), Some(surviving_provider_id));
        assert_eq!(surviving.model.as_deref(), Some("keep-me"));
    }

    #[tokio::test]
    async fn provider_model_delete_atomically_cleans_only_the_exact_workshop_pair() {
        let dir = tempfile::tempdir().unwrap();
        let (coord, db) = coordinator(dir.path()).await;
        let workshop = coord.workshop.clone();
        let provider_id = "0190f5fe-7c00-7a00-8000-000000000021";
        for model in ["delete-me", "keep-same-provider"] {
            nomifun_db::sqlx::query(
                "INSERT INTO provider_models \
                    (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
                 VALUES (?, ?, 1, 0, NULL, 1, 1)",
            )
            .bind(provider_id)
            .bind(model)
            .execute(db.pool())
            .await
            .unwrap();
            nomifun_db::sqlx::query(
                "INSERT INTO provider_model_capabilities \
                    (provider_id, model, task, traits, protocol, connection_role, \
                     allow_cross_origin_credentials, provider_params, created_at, updated_at) \
                 VALUES (?, ?, 'image_generation', '[]', 'openai.images', 'default', \
                         0, '{}', 1, 1)",
            )
            .bind(provider_id)
            .bind(model)
            .execute(db.pool())
            .await
            .unwrap();
        }

        let project = workshop
            .create_creative_project(Some("exact model cleanup integration".into()))
            .await
            .unwrap();
        let mut document =
            nomifun_workshop::CreativeProjectDocument::empty(project.project_id.clone());
        for (id, model) in [
            ("delete-config", "delete-me"),
            ("keep-config", "keep-same-provider"),
        ] {
            document.nodes.push(
                serde_json::from_value(serde_json::json!({
                    "id": id,
                    "type": "config",
                    "position": { "x": 0, "y": 0 },
                    "size": { "width": 320, "height": 240 },
                    "groupId": null,
                    "zIndex": 1,
                    "locked": false,
                    "data": {
                        "task": "image_generation",
                        "capability": "t2i",
                        "providerId": provider_id,
                        "model": model,
                        "prompt": "preserve",
                        "negativePrompt": "",
                        "parameters": {},
                        "inputAssetIds": [],
                        "taskId": null,
                        "resultAssetIds": [],
                        "status": "idle",
                        "errorMessage": null
                    }
                }))
                .unwrap(),
            );
        }
        workshop
            .save_creative_project(&project.project_id, "1", &document)
            .await
            .unwrap();

        let model_service = nomifun_system::ProviderModelService::new(
            Arc::new(nomifun_db::SqliteProviderModelRepository::new(
                db.pool().clone(),
            )),
            Arc::new(nomifun_db::SqliteProviderModelCapabilityRepository::new(
                db.pool().clone(),
            )),
            Arc::new(SqliteProviderRepository::new(db.pool().clone())),
            Arc::new(nomifun_db::SqliteProviderConnectionRepository::new(
                db.pool().clone(),
            )),
            Arc::new(coord),
        );

        assert!(model_service.delete(provider_id, "delete-me").await.unwrap());
        let deleted_count: i64 = nomifun_db::sqlx::query_scalar(
            "SELECT COUNT(*) FROM provider_models WHERE provider_id = ? AND model = 'delete-me'",
        )
        .bind(provider_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        let sibling_count: i64 = nomifun_db::sqlx::query_scalar(
            "SELECT COUNT(*) FROM provider_models WHERE provider_id = ? AND model = 'keep-same-provider'",
        )
        .bind(provider_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(deleted_count, 0);
        assert_eq!(sibling_count, 1);

        let cleaned = workshop.get_creative_project(&project.project_id).await.unwrap();
        assert_eq!(cleaned.project.revision, "3");
        let nomifun_workshop::creative_studio::CreativeNodeData::Config(deleted) =
            &cleaned.document.nodes[0].data
        else {
            panic!("expected deleted config")
        };
        assert_eq!(deleted.provider_id, None);
        assert_eq!(deleted.model, None);
        assert_eq!(deleted.prompt, "preserve");
        let nomifun_workshop::creative_studio::CreativeNodeData::Config(sibling) =
            &cleaned.document.nodes[1].data
        else {
            panic!("expected sibling config")
        };
        assert_eq!(sibling.provider_id.as_deref(), Some(provider_id));
        assert_eq!(sibling.model.as_deref(), Some("keep-same-provider"));
    }
}
