//! Black-box integration tests for `CronService`.
//!
//! Uses real SQLite (in-memory), mock broadcaster, and stubs for
//! task manager / conversation service (since integration with AI agents
//! is out of scope for this service-layer test).
//!
//! Covers test-plan items: CJ-1..CJ-12, SK-1..SK-7, SC-1..SC-8,
//! OC-1, SR-1, ICronService trait integration.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use nomifun_ai_agent::AgentRegistry;
use nomifun_ai_agent::agent_task::AgentInstance;
use nomifun_ai_agent::types::BuildTaskOptions;
use nomifun_api_types::{
    CreateCronJobRequest, CronScheduleDto, ListCronJobsQuery, SaveCronSkillRequest,
    UpdateCronJobRequest, WebSocketMessage,
};
use nomifun_common::{PaginatedResult, TimestampMs, now_ms};
use nomifun_conversation::ConversationService;
use nomifun_conversation::response_middleware::{CronCreateParams, CronUpdateParams};
use nomifun_db::{
    ConversationFilters, ConversationRowUpdate, IAcpSessionRepository, IAgentMetadataRepository,
    IConversationRepository, ICronRepository, MessageRowUpdate, MessageSearchRow, SortOrder,
    SqliteAcpSessionRepository, SqliteAgentMetadataRepository, SqliteConversationRepository,
    SqliteCronRepository, init_database_memory, models::MessageRow,
};
use nomifun_realtime::EventBroadcaster;

use nomifun_cron::busy_guard::CronBusyGuard;
use nomifun_cron::events::CronEventEmitter;
use nomifun_cron::executor::JobExecutor;
use nomifun_cron::scheduler::CronScheduler;
use nomifun_cron::service::CronService;
use nomifun_cron::types::JobStatus;

// ── Test infrastructure ────────────────────────────────────────────

struct MockBroadcaster {
    events: Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
}

impl MockBroadcaster {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn take_events(&self) -> Vec<WebSocketMessage<serde_json::Value>> {
        let mut guard = self.events.lock().unwrap();
        std::mem::take(&mut *guard)
    }
}

impl EventBroadcaster for MockBroadcaster {
    fn broadcast(&self, event: WebSocketMessage<serde_json::Value>) {
        self.events.lock().unwrap().push(event);
    }
}

struct StubTaskManager;

#[async_trait::async_trait]
impl nomifun_ai_agent::task_manager::IWorkerTaskManager for StubTaskManager {
    fn get_task(&self, _: &str) -> Option<AgentInstance> {
        None
    }
    async fn get_or_build_task(
        &self,
        _: &str,
        _: BuildTaskOptions,
    ) -> Result<AgentInstance, nomifun_common::AppError> {
        Err(nomifun_common::AppError::Internal("stub".into()))
    }
    fn kill(
        &self,
        _: &str,
        _: Option<nomifun_common::AgentKillReason>,
    ) -> Result<(), nomifun_common::AppError> {
        Ok(())
    }
    fn kill_and_wait(
        &self,
        _: &str,
        _: Option<nomifun_common::AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(std::future::ready(()))
    }
    fn clear(&self) {}
    fn active_count(&self) -> usize {
        0
    }
    fn collect_idle(&self, _: TimestampMs) -> Vec<String> {
        vec![]
    }
}

struct StubConvRepo {
    messages: Mutex<Vec<MessageRow>>,
    artifacts: Mutex<Vec<nomifun_db::ConversationArtifactRow>>,
    rows: Mutex<HashMap<i64, nomifun_db::models::ConversationRow>>,
    /// Mimics SQLite's AUTOINCREMENT for `conversation_artifacts.id`.
    next_artifact_id: AtomicI64,
}

impl StubConvRepo {
    fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            artifacts: Mutex::new(Vec::new()),
            rows: Mutex::new(HashMap::new()),
            next_artifact_id: AtomicI64::new(1),
        }
    }

    fn take_messages(&self) -> Vec<MessageRow> {
        let mut guard = self.messages.lock().unwrap();
        std::mem::take(&mut *guard)
    }

    fn upsert_artifact_row(&self, mut artifact: nomifun_db::ConversationArtifactRow) {
        let mut guard = self.artifacts.lock().unwrap();
        if artifact.id == 0 {
            artifact.id = self.next_artifact_id.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(existing) = guard.iter_mut().find(|row| row.id == artifact.id) {
            *existing = artifact;
        } else {
            guard.push(artifact);
        }
    }

    fn artifacts(&self) -> Vec<nomifun_db::ConversationArtifactRow> {
        self.artifacts.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl IConversationRepository for StubConvRepo {
    async fn get(
        &self,
        id: i64,
    ) -> Result<Option<nomifun_db::models::ConversationRow>, nomifun_db::DbError> {
        let mut rows = self.rows.lock().unwrap();

        if let Some(existing) = rows.get(&id) {
            return Ok(Some(existing.clone()));
        }
        // Id 9 == "missing-conv-1": reported absent so orphan cleanup fires.
        if id == 9 {
            return Ok(None);
        }

        let row = if id == 10 {
            // conv_mode
            nomifun_db::models::ConversationRow {
                id,
                user_id: "u1".into(),
                name: "Gemini Chat".into(),
                r#type: "acp".into(),
                model: Some(
                    serde_json::json!({
                        "provider_id": "gemini",
                        "model": "gemini-2.5-pro",
                        "use_model": "gemini-2.5-pro"
                    })
                    .to_string(),
                ),
                status: Some("active".into()),
                source: None,
                channel_chat_id: None,
                extra: serde_json::json!({
                    "backend": "gemini",
                    "agent_name": "Gemini",
                    "workspace": "/tmp/gemini-workspace",
                    "session_mode": "yolo",
                    "current_model_id": "gemini-2.5-pro"
                })
                .to_string(),
                pinned: false,
                pinned_at: None,
                cron_job_id: None,
                created_at: 1000,
                updated_at: 1000,
            }
        } else if id == 11 {
            // conv_mode_default
            nomifun_db::models::ConversationRow {
                id,
                user_id: "u1".into(),
                name: "Gemini Default Chat".into(),
                r#type: "acp".into(),
                model: Some(
                    serde_json::json!({
                        "provider_id": "gemini",
                        "model": "gemini-2.5-pro",
                        "use_model": "gemini-2.5-pro"
                    })
                    .to_string(),
                ),
                status: Some("active".into()),
                source: None,
                channel_chat_id: None,
                extra: serde_json::json!({
                    "backend": "gemini",
                    "agent_name": "Gemini",
                    "workspace": "/tmp/gemini-default-workspace",
                    "session_mode": "default",
                    "current_model_id": "gemini-2.5-pro"
                })
                .to_string(),
                pinned: false,
                pinned_at: None,
                cron_job_id: None,
                created_at: 1000,
                updated_at: 1000,
            }
        } else if id == 12 {
            // conv_mode_codex
            nomifun_db::models::ConversationRow {
                id,
                user_id: "u1".into(),
                name: "Codex Chat".into(),
                r#type: "acp".into(),
                model: Some(
                    serde_json::json!({
                        "provider_id": "codex",
                        "model": "gpt-5-codex",
                        "use_model": "gpt-5-codex"
                    })
                    .to_string(),
                ),
                status: Some("active".into()),
                source: None,
                channel_chat_id: None,
                extra: serde_json::json!({
                    "backend": "codex",
                    "agent_name": "Codex",
                    "workspace": "/tmp/codex-workspace",
                    "session_mode": "default",
                    "current_model_id": "gpt-5-codex"
                })
                .to_string(),
                pinned: false,
                pinned_at: None,
                cron_job_id: None,
                created_at: 1000,
                updated_at: 1000,
            }
        } else if id == 13 {
            // conv_mode_claude
            nomifun_db::models::ConversationRow {
                id,
                user_id: "u1".into(),
                name: "Claude Chat".into(),
                r#type: "acp".into(),
                model: Some(
                    serde_json::json!({
                        "provider_id": "claude",
                        "model": "claude-sonnet-4-20250514",
                        "use_model": "claude-sonnet-4-20250514"
                    })
                    .to_string(),
                ),
                status: Some("active".into()),
                source: None,
                channel_chat_id: None,
                extra: serde_json::json!({
                    "backend": "claude",
                    "agent_name": "Claude",
                    "workspace": "/tmp/claude-workspace",
                    "session_mode": "default",
                    "current_model_id": "claude-sonnet-4-20250514"
                })
                .to_string(),
                pinned: false,
                pinned_at: None,
                cron_job_id: None,
                created_at: 1000,
                updated_at: 1000,
            }
        } else if id == 14 {
            // conv_mode_nomi
            nomifun_db::models::ConversationRow {
                id,
                user_id: "u1".into(),
                name: "Nomi Chat".into(),
                r#type: "nomi".into(),
                model: Some(
                    serde_json::json!({
                        "provider_id": "anthropic",
                        "model": "claude-sonnet-4-20250514",
                        "use_model": "claude-sonnet-4-20250514"
                    })
                    .to_string(),
                ),
                status: Some("active".into()),
                source: None,
                channel_chat_id: None,
                extra: serde_json::json!({
                    "backend": "anthropic",
                    "agent_name": "Nomi",
                    "workspace": "/tmp/nomi-workspace",
                    "session_mode": "default",
                    "current_model_id": "claude-sonnet-4-20250514"
                })
                .to_string(),
                pinned: false,
                pinned_at: None,
                cron_job_id: None,
                created_at: 1000,
                updated_at: 1000,
            }
        } else {
            nomifun_db::models::ConversationRow {
                id,
                user_id: "u1".into(),
                name: "stub".into(),
                r#type: "default".into(),
                model: None,
                status: Some("active".into()),
                source: None,
                channel_chat_id: None,
                extra: "{}".into(),
                pinned: false,
                pinned_at: None,
                cron_job_id: None,
                created_at: 1000,
                updated_at: 1000,
            }
        };

        rows.insert(id, row.clone());
        Ok(Some(row))
    }
    async fn create(
        &self,
        row: &nomifun_db::models::ConversationRow,
    ) -> Result<i64, nomifun_db::DbError> {
        self.rows.lock().unwrap().insert(row.id, row.clone());
        Ok(row.id)
    }
    async fn update(
        &self,
        id: i64,
        updates: &ConversationRowUpdate,
    ) -> Result<(), nomifun_db::DbError> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows
            .entry(id)
            .or_insert_with(|| nomifun_db::models::ConversationRow {
                id,
                user_id: "u1".into(),
                name: "stub".into(),
                r#type: "default".into(),
                model: None,
                status: Some("active".into()),
                source: None,
                channel_chat_id: None,
                extra: "{}".into(),
                pinned: false,
                pinned_at: None,
                cron_job_id: None,
                created_at: 1000,
                updated_at: 1000,
            });
        if let Some(extra) = &updates.extra {
            row.extra = extra.clone();
        }
        if let Some(cron_job_id) = &updates.cron_job_id {
            row.cron_job_id = cron_job_id.clone();
        }
        if let Some(updated_at) = updates.updated_at {
            row.updated_at = updated_at;
        }
        Ok(())
    }
    async fn delete(&self, _id: i64) -> Result<(), nomifun_db::DbError> {
        Ok(())
    }
    async fn list_paginated(
        &self,
        _user_id: &str,
        _filters: &ConversationFilters,
    ) -> Result<PaginatedResult<nomifun_db::models::ConversationRow>, nomifun_db::DbError> {
        Ok(PaginatedResult {
            items: vec![],
            total: 0,
            has_more: false,
        })
    }
    async fn find_by_source_and_chat(
        &self,
        _user_id: &str,
        _source: &str,
        _chat_id: &str,
        _agent_type: &str,
    ) -> Result<Option<nomifun_db::models::ConversationRow>, nomifun_db::DbError> {
        Ok(None)
    }
    async fn list_by_cron_job(
        &self,
        _user_id: &str,
        cron_job_id: &str,
    ) -> Result<Vec<nomifun_db::models::ConversationRow>, nomifun_db::DbError> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .values()
            .filter(|row| row.cron_job_id.as_deref() == Some(cron_job_id))
            .cloned()
            .collect())
    }
    async fn list_associated(
        &self,
        _user_id: &str,
        _conversation_id: i64,
    ) -> Result<Vec<nomifun_db::models::ConversationRow>, nomifun_db::DbError> {
        Ok(vec![])
    }
    async fn get_messages(
        &self,
        _conv_id: i64,
        _page: u32,
        _page_size: u32,
        _order: SortOrder,
    ) -> Result<PaginatedResult<nomifun_db::models::MessageRow>, nomifun_db::DbError> {
        Ok(PaginatedResult {
            items: vec![],
            total: 0,
            has_more: false,
        })
    }
    async fn insert_message(
        &self,
        message: &nomifun_db::models::MessageRow,
    ) -> Result<(), nomifun_db::DbError> {
        self.messages.lock().unwrap().push(message.clone());
        Ok(())
    }
    async fn update_message(
        &self,
        _id: &str,
        _updates: &MessageRowUpdate,
    ) -> Result<(), nomifun_db::DbError> {
        Ok(())
    }
    async fn delete_messages_by_conversation(
        &self,
        _conv_id: i64,
    ) -> Result<(), nomifun_db::DbError> {
        Ok(())
    }
    async fn get_message_by_msg_id(
        &self,
        _conv_id: i64,
        _msg_id: &str,
        _msg_type: &str,
    ) -> Result<Option<nomifun_db::models::MessageRow>, nomifun_db::DbError> {
        Ok(None)
    }
    async fn search_messages(
        &self,
        _user_id: &str,
        _keyword: &str,
        _page: u32,
        _page_size: u32,
    ) -> Result<PaginatedResult<MessageSearchRow>, nomifun_db::DbError> {
        Ok(PaginatedResult {
            items: vec![],
            total: 0,
            has_more: false,
        })
    }
    async fn list_artifacts(
        &self,
        conversation_id: i64,
    ) -> Result<Vec<nomifun_db::ConversationArtifactRow>, nomifun_db::DbError> {
        Ok(self
            .artifacts
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.conversation_id == conversation_id)
            .cloned()
            .collect())
    }
    async fn get_artifact(
        &self,
        conversation_id: i64,
        artifact_id: i64,
    ) -> Result<Option<nomifun_db::ConversationArtifactRow>, nomifun_db::DbError> {
        Ok(self
            .artifacts
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.conversation_id == conversation_id && row.id == artifact_id)
            .cloned())
    }
    async fn upsert_artifact(
        &self,
        artifact: &nomifun_db::ConversationArtifactRow,
    ) -> Result<nomifun_db::ConversationArtifactRow, nomifun_db::DbError> {
        let mut guard = self.artifacts.lock().unwrap();
        // `skill_suggest` is upserted against the partial-unique
        // `(conversation_id, cron_job_id)`; `cron_trigger` is a fresh insert.
        if artifact.kind == "skill_suggest"
            && let Some(existing) = guard.iter_mut().find(|row| {
                row.kind == "skill_suggest"
                    && row.conversation_id == artifact.conversation_id
                    && row.cron_job_id == artifact.cron_job_id
            })
        {
            let id = existing.id;
            *existing = artifact.clone();
            existing.id = id;
            return Ok(existing.clone());
        }
        let mut stored = artifact.clone();
        stored.id = self.next_artifact_id.fetch_add(1, Ordering::Relaxed);
        guard.push(stored.clone());
        Ok(stored)
    }
    async fn update_artifact_status(
        &self,
        conversation_id: i64,
        artifact_id: i64,
        status: &str,
        updated_at: TimestampMs,
    ) -> Result<Option<nomifun_db::ConversationArtifactRow>, nomifun_db::DbError> {
        let mut guard = self.artifacts.lock().unwrap();
        let Some(existing) = guard
            .iter_mut()
            .find(|row| row.conversation_id == conversation_id && row.id == artifact_id)
        else {
            return Ok(None);
        };
        existing.status = status.to_string();
        existing.updated_at = updated_at;
        Ok(Some(existing.clone()))
    }
    async fn mark_skill_suggest_artifacts_saved(
        &self,
        cron_job_id: &str,
        updated_at: TimestampMs,
    ) -> Result<Vec<nomifun_db::ConversationArtifactRow>, nomifun_db::DbError> {
        let mut guard = self.artifacts.lock().unwrap();
        let mut updated = Vec::new();
        for artifact in guard.iter_mut() {
            if artifact.kind == "skill_suggest"
                && artifact.cron_job_id.as_deref() == Some(cron_job_id)
            {
                artifact.status = "saved".into();
                artifact.updated_at = updated_at;
                updated.push(artifact.clone());
            }
        }
        Ok(updated)
    }
}

async fn setup() -> (CronService, Arc<dyn ICronRepository>, Arc<MockBroadcaster>) {
    let (svc, repo, bc, _) = setup_with_conv_repo().await;
    (svc, repo, bc)
}

async fn setup_with_conv_repo() -> (
    CronService,
    Arc<dyn ICronRepository>,
    Arc<MockBroadcaster>,
    Arc<StubConvRepo>,
) {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    let cron_repo: Arc<dyn ICronRepository> = Arc::new(SqliteCronRepository::new(pool.clone()));

    // Seed the conversations that cron tests bind jobs to into the REAL DB so
    // the new `cron_jobs.conversation_id → conversations(id)` FK (foreign_keys=ON)
    // is satisfied at insert time. Conversation *state* (existence for orphan
    // cleanup, bind targets) is still driven by the in-memory `StubConvRepo`
    // the service actually queries; this real-DB seed only satisfies the FK.
    //
    // Conversation ids are now integer PKs allocated by AUTOINCREMENT. Inserting
    // 14 rows into a fresh in-memory DB allocates ids 1..=14 in order; the tests
    // reference these by their fixed integer (see the id map below). The id 9
    // ("missing-conv-1") and id 8 ("conv-that-no-longer-exists") rows exist for
    // the FK, while `StubConvRepo::get` independently reports id 9 absent so
    // orphan-cleanup logic still fires.
    //
    // Id map: 1=conv_1, 2=conv_target, 3=conv_other, 4=conv_existing_bind,
    // 5=conv_cascade, 6=conv_trait_del, 7=conv-existing,
    // 8=conv-that-no-longer-exists, 9=missing-conv-1, 10=conv_mode,
    // 11=conv_mode_default, 12=conv_mode_codex, 13=conv_mode_claude,
    // 14=conv_mode_nomi.
    {
        let real_conv_repo = SqliteConversationRepository::new(pool.clone());
        for _ in 1..=14 {
            real_conv_repo
                .create(&nomifun_db::models::ConversationRow {
                    // `create` allocates the PK (AUTOINCREMENT) and ignores this.
                    id: 0,
                    user_id: "system_default_user".into(),
                    name: "Seed Conversation".into(),
                    r#type: "acp".into(),
                    extra: "{}".into(),
                    model: None,
                    status: Some("finished".into()),
                    source: Some("nomifun".into()),
                    channel_chat_id: None,
                    pinned: false,
                    pinned_at: None,
                    cron_job_id: None,
                    created_at: now_ms(),
                    updated_at: now_ms(),
                })
                .await
                .unwrap();
        }
    }

    let agent_metadata_repo: Arc<dyn IAgentMetadataRepository> =
        Arc::new(SqliteAgentMetadataRepository::new(pool.clone()));
    let acp_session_repo: Arc<dyn IAcpSessionRepository> =
        Arc::new(SqliteAcpSessionRepository::new(pool));
    let bc = Arc::new(MockBroadcaster::new());
    let data_dir = std::env::temp_dir().join(format!("nomifun-cron-test-{}", now_ms()));
    std::fs::create_dir_all(&data_dir).unwrap();

    struct StubSkillResolver;
    #[async_trait::async_trait]
    impl nomifun_conversation::skill_resolver::SkillResolver for StubSkillResolver {
        async fn auto_inject_names(&self) -> Vec<String> {
            Vec::new()
        }

        async fn resolve_skills(
            &self,
            _names: &[String],
        ) -> Vec<nomifun_conversation::skill_resolver::ResolvedAgentSkill> {
            Vec::new()
        }

        async fn link_workspace_skills(
            &self,
            _workspace: &std::path::Path,
            _rel_dirs: &[&str],
            _skills: &[nomifun_conversation::skill_resolver::ResolvedAgentSkill],
        ) -> usize {
            0
        }
    }

    let stub_conv_repo = Arc::new(StubConvRepo::new());
    let stub_conv_repo_trait: Arc<dyn IConversationRepository> = stub_conv_repo.clone();
    let task_manager: Arc<dyn nomifun_ai_agent::task_manager::IWorkerTaskManager> =
        Arc::new(StubTaskManager);
    let conv_service = Arc::new(ConversationService::new(
        std::env::temp_dir(),
        bc.clone() as Arc<dyn EventBroadcaster>,
        Arc::new(StubSkillResolver),
        Arc::clone(&task_manager),
        Arc::clone(&stub_conv_repo_trait),
        Arc::clone(&agent_metadata_repo),
        acp_session_repo,
    ));
    let agent_registry = AgentRegistry::new(agent_metadata_repo);
    agent_registry.hydrate().await.unwrap();
    let busy_guard = Arc::new(CronBusyGuard::new());
    let executor = Arc::new(JobExecutor::new(
        task_manager,
        stub_conv_repo_trait,
        conv_service,
        busy_guard,
        data_dir.clone(),
        data_dir.clone(),
        bc.clone() as Arc<dyn EventBroadcaster>,
        agent_registry,
    ));

    let scheduler = Arc::new(CronScheduler::new(Arc::new(|_| {})));

    let emitter = CronEventEmitter::new(bc.clone() as Arc<dyn EventBroadcaster>);
    let svc = CronService::new(cron_repo.clone(), scheduler, executor, emitter, data_dir);

    std::mem::forget(db);
    (svc, cron_repo, bc, stub_conv_repo)
}

fn make_create_req(name: &str, schedule: CronScheduleDto) -> CreateCronJobRequest {
    CreateCronJobRequest {
        name: name.into(),
        description: Some("test description".into()),
        schedule,
        prompt: None,
        message: Some("test message".into()),
        conversation_id: 1,
        conversation_title: Some("Test Conv".into()),
        agent_type: "acp".into(),
        created_by: "user".into(),
        execution_mode: None,
        agent_config: None,
        target_kind: "agent".into(),
    }
}

fn every_60s() -> CronScheduleDto {
    CronScheduleDto::Every {
        every_ms: 60000,
        description: Some("every minute".into()),
    }
}

fn at_future(offset_ms: i64) -> CronScheduleDto {
    CronScheduleDto::At {
        at_ms: now_ms() + offset_ms,
        description: Some("once".into()),
    }
}

fn cron_every_5min() -> CronScheduleDto {
    CronScheduleDto::Cron {
        expr: "0 */5 * * * *".into(),
        tz: None,
        description: Some("every 5 min".into()),
    }
}

// ── CJ-1: Create cron job ──────────────────────────────────────────

#[tokio::test]
async fn cj1_create_cron_job() {
    let (svc, _, bc) = setup().await;
    let req = make_create_req("Daily Report", every_60s());

    let job = svc.add_job(req).await.unwrap();

    assert!(job.id.starts_with("cron_"));
    assert_eq!(job.name, "Daily Report");
    assert!(job.enabled);
    assert!(job.next_run_at.is_some());
    assert_eq!(job.run_count, 0);

    let events = bc.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "cron.job-created");
}

#[tokio::test]
async fn cj1b_reject_terminal_target_cron_job() {
    let (svc, _, _) = setup().await;
    let mut req = make_create_req("Terminal Target", every_60s());
    req.conversation_id = 0;
    req.agent_type = "terminal".into();
    req.target_kind = "terminal".into();

    let err = svc
        .add_job(req)
        .await
        .expect_err("terminal targets are no longer supported by scheduled tasks");

    assert!(matches!(
        err,
        nomifun_cron::error::CronError::InvalidTargetKind(_)
    ));
}

// ── CJ-2: Create three schedule types ──────────────────────────────

#[tokio::test]
async fn cj2_create_three_schedule_types() {
    let (svc, _, _) = setup().await;
    let now = now_ms();

    let at_job = svc
        .add_job(make_create_req("At Job", at_future(3600000)))
        .await
        .unwrap();
    assert!(at_job.next_run_at.unwrap() > now);

    let every_job = svc
        .add_job(make_create_req("Every Job", every_60s()))
        .await
        .unwrap();
    let next = every_job.next_run_at.unwrap();
    assert!((next - now - 60000).abs() < 2000);

    let cron_job = svc
        .add_job(make_create_req("Cron Job", cron_every_5min()))
        .await
        .unwrap();
    assert!(cron_job.next_run_at.unwrap() > now);
}

// ── CJ-4: Get single job ──────────────────────────────────────────

#[tokio::test]
async fn cj4_get_single_job() {
    let (svc, _, _) = setup().await;
    let created = svc
        .add_job(make_create_req("Get Test", every_60s()))
        .await
        .unwrap();

    let fetched = svc.get_job(&created.id).await.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "Get Test");
}

// ── CJ-5: Get nonexistent job ─────────────────────────────────────

#[tokio::test]
async fn cj5_get_nonexistent_job() {
    let (svc, _, _) = setup().await;
    let err = svc.get_job("cron_nonexistent").await.unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::JobNotFound(_)
    ));
}

// ── CJ-6: List all jobs ───────────────────────────────────────────

#[tokio::test]
async fn cj6_list_all_jobs() {
    let (svc, _, _) = setup().await;
    for i in 0..3 {
        svc.add_job(make_create_req(&format!("Job {i}"), every_60s()))
            .await
            .unwrap();
    }

    let jobs = svc.list_jobs(&ListCronJobsQuery::default()).await.unwrap();
    assert!(jobs.len() >= 3);
}

// ── CJ-7: List by conversation ────────────────────────────────────

#[tokio::test]
async fn cj7_list_by_conversation() {
    let (svc, _, _) = setup().await;

    let mut req1 = make_create_req("Job A", every_60s());
    req1.conversation_id = 2;
    svc.add_job(req1).await.unwrap();

    let mut req2 = make_create_req("Job B", every_60s());
    req2.conversation_id = 2;
    svc.add_job(req2).await.unwrap();

    let mut req3 = make_create_req("Job C", every_60s());
    req3.conversation_id = 3;
    svc.add_job(req3).await.unwrap();

    let query = ListCronJobsQuery {
        conversation_id: Some(2),
    };
    let jobs = svc.list_jobs(&query).await.unwrap();
    assert_eq!(jobs.len(), 2);
}

#[tokio::test]
async fn cj7b_add_job_binds_existing_conversation_to_job() {
    let (svc, _, _, conv_repo) = setup_with_conv_repo().await;

    let mut req = make_create_req("Bound Existing Conversation", every_60s());
    req.conversation_id = 4;

    let job = svc.add_job(req).await.unwrap();

    let bound = conv_repo.get(4).await.unwrap().unwrap();
    assert_eq!(bound.cron_job_id.as_deref(), Some(job.id.as_str()));

    let linked = conv_repo.list_by_cron_job("user_1", &job.id).await.unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].id, 4);
}

// ── CJ-8: Update job ──────────────────────────────────────────────

#[tokio::test]
async fn cj8_update_job() {
    let (svc, _, bc) = setup().await;
    let created = svc
        .add_job(make_create_req("Original", every_60s()))
        .await
        .unwrap();
    bc.take_events();

    let req = UpdateCronJobRequest {
        name: Some("Updated Name".into()),
        description: Some("Updated description".into()),
        enabled: Some(false),
        schedule: None,
        message: None,
        execution_mode: None,
        agent_config: None,
        conversation_title: None,
        max_retries: None,
        target_kind: None,
    };

    let updated = svc.update_job(&created.id, req).await.unwrap();
    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.description.as_deref(), Some("Updated description"));
    assert!(!updated.enabled);
    assert!(updated.updated_at >= created.created_at);

    let events = bc.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "cron.job-updated");
}

// ── CJ-9: Update schedule type ────────────────────────────────────

#[tokio::test]
async fn cj9_update_schedule_type() {
    let (svc, _, _) = setup().await;
    let created = svc
        .add_job(make_create_req("Schedule Change", every_60s()))
        .await
        .unwrap();

    let req = UpdateCronJobRequest {
        name: None,
        description: None,
        enabled: None,
        schedule: Some(cron_every_5min()),
        message: None,
        execution_mode: None,
        agent_config: None,
        conversation_title: None,
        max_retries: None,
        target_kind: None,
    };

    let updated = svc.update_job(&created.id, req).await.unwrap();
    assert!(matches!(
        updated.schedule,
        nomifun_cron::types::CronSchedule::Cron { .. }
    ));
    assert!(updated.next_run_at.is_some());
}

// ── CJ-10: Update nonexistent job ─────────────────────────────────

#[tokio::test]
async fn cj10_update_nonexistent() {
    let (svc, _, _) = setup().await;
    let req = UpdateCronJobRequest {
        name: Some("x".into()),
        description: None,
        enabled: None,
        schedule: None,
        message: None,
        execution_mode: None,
        agent_config: None,
        conversation_title: None,
        max_retries: None,
        target_kind: None,
    };
    let err = svc.update_job("cron_nonexistent", req).await.unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::JobNotFound(_)
    ));
}

// ── CJ-11: Delete job ─────────────────────────────────────────────

#[tokio::test]
async fn cj11_delete_job() {
    let (svc, _, bc) = setup().await;
    let created = svc
        .add_job(make_create_req("To Delete", every_60s()))
        .await
        .unwrap();
    bc.take_events();

    svc.remove_job(&created.id).await.unwrap();

    let err = svc.get_job(&created.id).await.unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::JobNotFound(_)
    ));

    let events = bc.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "cron.job-removed");
}

// ── CJ-12: Delete nonexistent ─────────────────────────────────────

#[tokio::test]
async fn cj12_delete_nonexistent() {
    let (svc, _, _) = setup().await;
    let err = svc.remove_job("cron_nonexistent").await.unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::Database(nomifun_db::DbError::NotFound(_))
    ));
}

// ── SK-1: Save skill ──────────────────────────────────────────────

#[tokio::test]
async fn sk1_save_skill() {
    let (svc, _, _) = setup().await;
    let job = svc
        .add_job(make_create_req("Skill Job", every_60s()))
        .await
        .unwrap();

    let req = SaveCronSkillRequest {
        content: "---\nname: test\ndescription: test skill\n---\nDo something".into(),
    };
    svc.save_skill(&job.id, req).await.unwrap();
}

#[tokio::test]
async fn sk1_1_save_skill_marks_related_skill_suggest_artifacts_saved() {
    let (svc, _, bc, conv_repo) = setup_with_conv_repo().await;
    let job = svc
        .add_job(make_create_req("Skill Artifact Job", every_60s()))
        .await
        .unwrap();

    conv_repo.upsert_artifact_row(nomifun_db::ConversationArtifactRow {
        id: 0,
        conversation_id: 1,
        cron_job_id: Some(job.id.clone()),
        kind: "skill_suggest".into(),
        status: "active".into(),
        payload: serde_json::json!({
            "cron_job_id": job.id,
            "name": "daily-report",
            "description": "Daily report",
            "skillContent": "---\nname: daily-report\n---\nUse it."
        })
        .to_string(),
        created_at: 1000,
        updated_at: 1000,
    });

    svc.save_skill(
        &job.id,
        SaveCronSkillRequest {
            content: "---\nname: daily-report\ndescription: Daily report\n---\nUse it.".into(),
        },
    )
    .await
    .unwrap();

    let artifacts = conv_repo.artifacts();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].status, "saved");

    let events = bc.take_events();
    let saved_event = events
        .iter()
        .find(|event| {
            event.name == "conversation.artifact"
                && event.data["id"] == artifacts[0].id
                && event.data["status"] == "saved"
        })
        .expect("save_skill should broadcast saved artifact upsert");
    assert_eq!(saved_event.data["conversation_id"], 1);
}

// ── SK-2: Has skill (true) ────────────────────────────────────────

#[tokio::test]
async fn sk2_has_skill_true() {
    let (svc, _, _) = setup().await;
    let job = svc
        .add_job(make_create_req("Skill Check", every_60s()))
        .await
        .unwrap();

    svc.save_skill(
        &job.id,
        SaveCronSkillRequest {
            content: "---\nname: x\n---\nContent".into(),
        },
    )
    .await
    .unwrap();

    let resp = svc.has_skill(&job.id).await.unwrap();
    assert!(resp.has_skill);
}

// ── SK-3: Has skill (false) ───────────────────────────────────────

#[tokio::test]
async fn sk3_has_skill_false() {
    let (svc, _, _) = setup().await;
    let job = svc
        .add_job(make_create_req("No Skill", every_60s()))
        .await
        .unwrap();

    let resp = svc.has_skill(&job.id).await.unwrap();
    assert!(!resp.has_skill);
}

// ── SK-4: Save empty skill ────────────────────────────────────────

#[tokio::test]
async fn sk4_save_empty_skill() {
    let (svc, _, _) = setup().await;
    let job = svc
        .add_job(make_create_req("Empty Skill", every_60s()))
        .await
        .unwrap();

    let err = svc
        .save_skill(&job.id, SaveCronSkillRequest { content: "".into() })
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::InvalidSkillContent(_)
    ));
}

// ── SK-5: Save placeholder skill ──────────────────────────────────

#[tokio::test]
async fn sk5_save_placeholder_skill() {
    let (svc, _, _) = setup().await;
    let job = svc
        .add_job(make_create_req("Placeholder Skill", every_60s()))
        .await
        .unwrap();

    let err = svc
        .save_skill(
            &job.id,
            SaveCronSkillRequest {
                content: "TODO: fill in later".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::InvalidSkillContent(_)
    ));
}

// ── SK-6: Save skill for nonexistent job ──────────────────────────

#[tokio::test]
async fn sk6_save_skill_nonexistent() {
    let (svc, _, _) = setup().await;
    let err = svc
        .save_skill(
            "cron_nonexistent",
            SaveCronSkillRequest {
                content: "---\nname: x\n---\nOk".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::JobNotFound(_)
    ));
}

// ── SK-7: Delete skill on job removal ─────────────────────────────

#[tokio::test]
async fn sk7_delete_cleans_skill() {
    let (svc, _, _) = setup().await;
    let job = svc
        .add_job(make_create_req("Skill Cleanup", every_60s()))
        .await
        .unwrap();
    svc.save_skill(
        &job.id,
        SaveCronSkillRequest {
            content: "---\nname: x\n---\nContent".into(),
        },
    )
    .await
    .unwrap();

    svc.remove_job(&job.id).await.unwrap();

    let err = svc.has_skill(&job.id).await.unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::JobNotFound(_)
    ));
}

// ── SC-3: Every type next_run ─────────────────────────────────────

#[tokio::test]
async fn sc3_every_type_next_run() {
    let (svc, _, _) = setup().await;
    let now = now_ms();
    let job = svc
        .add_job(make_create_req("Every 60s", every_60s()))
        .await
        .unwrap();

    let next = job.next_run_at.unwrap();
    let diff = (next - now - 60000).abs();
    assert!(diff < 2000, "expected next_run ≈ now+60000, diff={diff}");
}

// ── SC-5: Invalid cron expression ─────────────────────────────────

#[tokio::test]
async fn sc5_invalid_cron_expression() {
    let (svc, _, _) = setup().await;
    let req = make_create_req(
        "Invalid Cron",
        CronScheduleDto::Cron {
            expr: "invalid cron".into(),
            tz: None,
            description: None,
        },
    );
    let err = svc.add_job(req).await.unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::InvalidCronExpression(_)
    ));
}

// ── SC-6: Cron with timezone ──────────────────────────────────────

#[tokio::test]
async fn sc6_cron_with_timezone() {
    let (svc, _, _) = setup().await;
    let now = now_ms();
    let req = make_create_req(
        "Shanghai Job",
        CronScheduleDto::Cron {
            expr: "0 0 9 * * *".into(),
            tz: Some("Asia/Shanghai".into()),
            description: None,
        },
    );
    let job = svc.add_job(req).await.unwrap();
    assert!(job.next_run_at.unwrap() > now);
}

// ── SC-7: Every zero interval ─────────────────────────────────────

#[tokio::test]
async fn sc7_every_zero_interval() {
    let (svc, _, _) = setup().await;
    let req = make_create_req(
        "Zero Interval",
        CronScheduleDto::Every {
            every_ms: 0,
            description: None,
        },
    );
    let err = svc.add_job(req).await.unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::InvalidSchedule(_)
    ));
}

// ── SC-8: Every negative interval ─────────────────────────────────

#[tokio::test]
async fn sc8_every_negative_interval() {
    let (svc, _, _) = setup().await;
    let req = make_create_req(
        "Negative Interval",
        CronScheduleDto::Every {
            every_ms: -1000,
            description: None,
        },
    );
    let err = svc.add_job(req).await.unwrap_err();
    assert!(matches!(
        err,
        nomifun_cron::error::CronError::InvalidSchedule(_)
    ));
}

// ── OC-1: Init preserves lazy-bind "existing" jobs with empty conversation_id ─────

#[tokio::test]
async fn oc1_init_preserves_lazy_existing_jobs() {
    // "existing + empty conversation_id" is a legitimate lazy-binding job:
    // the frontend creates a cron from the standalone cron page before any
    // conversation exists, and the first execution materializes it. Those
    // jobs must survive init, not be cleaned up as orphans.
    let (svc, _repo, _) = setup().await;

    let mut req = make_create_req("Lazy Existing", every_60s());
    req.conversation_id = 0;
    req.execution_mode = Some("existing".into());
    let lazy = svc.add_job(req).await.unwrap();

    let normal_req = make_create_req("Normal", every_60s());
    let normal = svc.add_job(normal_req).await.unwrap();

    svc.init().await;

    let found_lazy = svc.get_job(&lazy.id).await;
    assert!(
        found_lazy.is_ok(),
        "lazy-bind existing job should survive init"
    );

    let found = svc.get_job(&normal.id).await;
    assert!(found.is_ok());
}

// NewConversation jobs don't depend on any existing conversation — they
// create one on every run. They must never be cleaned up as orphans.
#[tokio::test]
async fn oc1b_init_preserves_new_conversation_jobs() {
    let (svc, _repo, _) = setup().await;

    let mut empty_req = make_create_req("New-conv empty", every_60s());
    empty_req.conversation_id = 0;
    empty_req.execution_mode = Some("new_conversation".into());
    let empty = svc.add_job(empty_req).await.unwrap();

    let mut stale_req = make_create_req("New-conv with stale id", every_60s());
    stale_req.conversation_id = 8;
    stale_req.execution_mode = Some("new_conversation".into());
    let stale = svc.add_job(stale_req).await.unwrap();

    svc.init().await;

    assert!(
        svc.get_job(&empty.id).await.is_ok(),
        "empty new_conversation job must survive"
    );
    assert!(
        svc.get_job(&stale.id).await.is_ok(),
        "new_conversation job with stale id must survive"
    );
}

#[tokio::test]
async fn oc2_init_cleans_jobs_with_missing_conversation() {
    let (svc, _repo, _) = setup().await;

    let mut missing_req = make_create_req("Missing Conversation", every_60s());
    missing_req.conversation_id = 9;
    let missing = svc.add_job(missing_req).await.unwrap();

    let mut normal_req = make_create_req("Existing Conversation", every_60s());
    normal_req.conversation_id = 7;
    let normal = svc.add_job(normal_req).await.unwrap();

    svc.init().await;

    let err = svc.get_job(&missing.id).await;
    assert!(err.is_err());

    let found = svc.get_job(&normal.id).await;
    assert!(found.is_ok());
}

// ── Delete skill explicitly ───────────────────────────────────────

#[tokio::test]
async fn delete_skill_clears_content() {
    let (svc, _, _) = setup().await;
    let job = svc
        .add_job(make_create_req("Del Skill", every_60s()))
        .await
        .unwrap();

    svc.save_skill(
        &job.id,
        SaveCronSkillRequest {
            content: "---\nname: x\n---\nOk".into(),
        },
    )
    .await
    .unwrap();
    assert!(svc.has_skill(&job.id).await.unwrap().has_skill);

    svc.delete_skill(&job.id).await.unwrap();
    assert!(!svc.has_skill(&job.id).await.unwrap().has_skill);
}

// ── ICronService trait: create ─────────────────────────────────────

#[tokio::test]
async fn icron_service_create_job() {
    let (svc, _, _, conv_repo) = setup_with_conv_repo().await;

    use nomifun_conversation::response_middleware::ICronService;

    let params = CronCreateParams {
        name: "Agent Job".into(),
        schedule: "0 */10 * * * *".into(),
        schedule_description: "every 10 min".into(),
        message: "do agent work".into(),
    };

    let result = ICronService::create_job(&svc, "user_1", "1", &params).await;
    assert!(result.success);
    assert!(result.message.contains("Agent Job"));

    let bound = conv_repo.get(1).await.unwrap().unwrap();
    assert!(bound.cron_job_id.is_some());
}

#[tokio::test]
async fn icron_service_create_job_inherits_conversation_mode_and_backend() {
    let (svc, _, _) = setup().await;

    use nomifun_conversation::response_middleware::ICronService;

    let params = CronCreateParams {
        name: "Agent Job".into(),
        schedule: "0 */10 * * * *".into(),
        schedule_description: "every 10 min".into(),
        message: "do agent work".into(),
    };

    let result = ICronService::create_job(&svc, "user_1", "10", &params).await;
    assert!(result.success);

    let jobs = svc
        .list_jobs(&ListCronJobsQuery {
            conversation_id: Some(10),
        })
        .await
        .unwrap();
    assert_eq!(jobs.len(), 1);

    let job = &jobs[0];
    let config = job
        .agent_config
        .as_ref()
        .expect("agent config should be copied");
    assert_eq!(job.agent_type, "acp");
    assert_eq!(job.conversation_title.as_deref(), Some("Gemini Chat"));
    assert_eq!(config.backend, "gemini");
    assert_eq!(config.name, "Gemini");
    assert_eq!(config.mode.as_deref(), Some("yolo"));
    assert_eq!(config.model_id.as_deref(), Some("gemini-2.5-pro"));
    assert_eq!(config.workspace.as_deref(), Some("/tmp/gemini-workspace"));
}

#[tokio::test]
async fn icron_service_create_job_forces_full_auto_mode_for_generated_crons() {
    let (svc, _, _) = setup().await;

    use nomifun_conversation::response_middleware::ICronService;

    let params = CronCreateParams {
        name: "Generated Agent Job".into(),
        schedule: "0 */10 * * * *".into(),
        schedule_description: "every 10 min".into(),
        message: "do agent work".into(),
    };

    let gemini = ICronService::create_job(&svc, "user_1", "11", &params).await;
    assert!(gemini.success);

    let codex = ICronService::create_job(&svc, "user_1", "12", &params).await;
    assert!(codex.success);

    let claude = ICronService::create_job(&svc, "user_1", "13", &params).await;
    assert!(claude.success);

    let nomi = ICronService::create_job(&svc, "user_1", "14", &params).await;
    assert!(nomi.success);

    let gemini_jobs = svc
        .list_jobs(&ListCronJobsQuery {
            conversation_id: Some(11),
        })
        .await
        .unwrap();
    assert_eq!(
        gemini_jobs[0]
            .agent_config
            .as_ref()
            .and_then(|config| config.mode.as_deref()),
        Some("yolo")
    );

    let codex_jobs = svc
        .list_jobs(&ListCronJobsQuery {
            conversation_id: Some(12),
        })
        .await
        .unwrap();
    assert_eq!(
        codex_jobs[0]
            .agent_config
            .as_ref()
            .and_then(|config| config.mode.as_deref()),
        Some("full-access")
    );

    let claude_jobs = svc
        .list_jobs(&ListCronJobsQuery {
            conversation_id: Some(13),
        })
        .await
        .unwrap();
    assert_eq!(
        claude_jobs[0]
            .agent_config
            .as_ref()
            .and_then(|config| config.mode.as_deref()),
        Some("bypassPermissions")
    );

    let nomi_jobs = svc
        .list_jobs(&ListCronJobsQuery {
            conversation_id: Some(14),
        })
        .await
        .unwrap();
    assert_eq!(
        nomi_jobs[0]
            .agent_config
            .as_ref()
            .and_then(|config| config.mode.as_deref()),
        Some("yolo")
    );
}

// ── ICronService trait: list ───────────────────────────────────────

#[tokio::test]
async fn icron_service_list_jobs() {
    let (svc, _, _) = setup().await;

    use nomifun_conversation::response_middleware::ICronService;

    let result = ICronService::list_jobs(&svc, "user_1", "1").await;
    assert!(result.success);
    assert!(
        result
            .message
            .contains("No cron jobs found for conversation '1'")
    );

    let mut req = make_create_req("Listed Job", every_60s());
    req.conversation_id = 1;
    svc.add_job(req).await.unwrap();

    let result = ICronService::list_jobs(&svc, "user_1", "1").await;
    assert!(result.success);
    assert!(
        result
            .message
            .contains("1 cron job(s) for conversation '1'")
    );
    assert!(result.message.contains("Listed Job"));
}

// ── ICronService trait: update ─────────────────────────────────────

#[tokio::test]
async fn icron_service_update_job() {
    let (svc, _, _, conv_repo) = setup_with_conv_repo().await;

    use nomifun_conversation::response_middleware::ICronService;

    let job = svc
        .add_job(make_create_req("Update Via Trait", every_60s()))
        .await
        .unwrap();

    let params = CronUpdateParams {
        job_id: job.id.clone(),
        name: "Updated Via Trait".into(),
        schedule: "0 */10 * * * *".into(),
        schedule_description: "every 10 min".into(),
        message: "do updated work".into(),
    };

    let result = ICronService::update_job(&svc, "user_1", "1", &params).await;
    assert!(result.success);
    assert!(result.message.contains("Updated Via Trait"));

    let bound = conv_repo.get(1).await.unwrap().unwrap();
    assert_eq!(bound.cron_job_id.as_deref(), Some(job.id.as_str()));

    let linked = conv_repo.list_by_cron_job("user_1", &job.id).await.unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].id, 1);
}

// ── ICronService trait: delete ─────────────────────────────────────

#[tokio::test]
async fn icron_service_delete_job() {
    let (svc, _, _) = setup().await;

    use nomifun_conversation::response_middleware::ICronService;

    let job = svc
        .add_job(make_create_req("Delete Via Trait", every_60s()))
        .await
        .unwrap();

    let result = ICronService::delete_job(&svc, "user_1", &job.id).await;
    assert!(result.success);

    let result = ICronService::delete_job(&svc, "user_1", "cron_nonexistent").await;
    assert!(!result.success);
}

// ── Update with max_retries ───────────────────────────────────────

#[tokio::test]
async fn update_max_retries() {
    let (svc, _, _) = setup().await;
    let job = svc
        .add_job(make_create_req("Retries", every_60s()))
        .await
        .unwrap();
    assert_eq!(job.max_retries, 3);

    let req = UpdateCronJobRequest {
        name: None,
        description: None,
        enabled: None,
        schedule: None,
        message: None,
        execution_mode: None,
        agent_config: None,
        conversation_title: None,
        max_retries: Some(5),
        target_kind: None,
    };
    let updated = svc.update_job(&job.id, req).await.unwrap();
    assert_eq!(updated.max_retries, 5);
}

// ── SC-1: At type — future timestamp, nextRunAtMs == atMs ────────

#[tokio::test]
async fn sc1_at_type_future_timestamp() {
    let (svc, _, _) = setup().await;
    let target_ms = now_ms() + 3_600_000;
    let req = make_create_req(
        "At Future",
        CronScheduleDto::At {
            at_ms: target_ms,
            description: Some("once in 1h".into()),
        },
    );
    let job = svc.add_job(req).await.unwrap();
    assert_eq!(job.next_run_at, Some(target_ms));
}

// ── SC-2: At type — past timestamp, nextRunAtMs == atMs ──────────

#[tokio::test]
async fn sc2_at_type_past_timestamp() {
    let (svc, _, _) = setup().await;
    let target_ms = now_ms() - 3_600_000;
    let req = make_create_req(
        "At Past",
        CronScheduleDto::At {
            at_ms: target_ms,
            description: Some("once in the past".into()),
        },
    );
    let job = svc.add_job(req).await.unwrap();
    assert_eq!(job.next_run_at, Some(target_ms));
}

// ── SR-1: System resume detects missed jobs ──────────────────────

#[tokio::test]
async fn sr1_system_resume_missed_job() {
    let (svc, repo, bc, conv_repo) = setup_with_conv_repo().await;

    let req = make_create_req("Resume Job", every_60s());
    let job = svc.add_job(req).await.unwrap();
    bc.take_events();

    let past_ms = now_ms() - 10_000;
    let params = nomifun_db::UpdateCronJobParams {
        next_run_at: Some(Some(past_ms)),
        ..Default::default()
    };
    repo.update(&job.id, &params).await.unwrap();

    svc.handle_system_resume().await;

    let updated = svc.get_job(&job.id).await.unwrap();
    assert!(
        updated.last_run_at.is_none(),
        "missed job should not be auto-executed on resume"
    );
    assert_eq!(updated.last_status, Some(JobStatus::Missed));
    assert!(
        updated.next_run_at.is_some(),
        "job should be rescheduled after being marked missed"
    );
    assert!(
        updated.next_run_at.unwrap() > now_ms() - 2000,
        "next_run_at should be in the future"
    );

    let messages = conv_repo.take_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].r#type, "tips");
    assert!(messages[0].content.contains("Resume Job"));
    assert!(messages[0].content.contains("not run automatically"));

    let events = bc.take_events();
    assert!(
        events
            .iter()
            .any(|event| { event.name == "cron.job-executed" && event.data["status"] == "missed" }),
        "resume should emit a missed execution event"
    );
    assert!(
        events.iter().any(|event| {
            event.name == "message.stream"
                && event.data["type"] == "tips"
                && event.data["conversation_id"] == "1"
        }),
        "resume should emit a tips websocket message"
    );
}

// ── CD-1: Cascade delete cron jobs when conversation is deleted ──

#[tokio::test]
async fn cd1_cascade_delete_by_conversation() {
    let (svc, _repo, bc) = setup().await;

    let mut req_a = make_create_req("Cascade A", every_60s());
    req_a.conversation_id = 5;
    let job_a = svc.add_job(req_a).await.unwrap();

    let mut req_b = make_create_req("Cascade B", every_60s());
    req_b.conversation_id = 5;
    let job_b = svc.add_job(req_b).await.unwrap();

    let mut req_c = make_create_req("Unrelated", every_60s());
    req_c.conversation_id = 3;
    let _job_c = svc.add_job(req_c).await.unwrap();

    bc.take_events();

    svc.delete_jobs_by_conversation(5).await;

    assert!(svc.get_job(&job_a.id).await.is_err());
    assert!(svc.get_job(&job_b.id).await.is_err());

    let remaining = svc.list_jobs(&ListCronJobsQuery::default()).await.unwrap();
    assert_eq!(remaining.len(), 1, "only the unrelated job should remain");

    let events = bc.take_events();
    let removed_events: Vec<_> = events
        .iter()
        .filter(|e| e.name == "cron.job-removed")
        .collect();
    assert_eq!(removed_events.len(), 2, "should emit 2 removed events");
}

// ── CD-2: Cascade delete on empty conversation (no-op) ──────────

#[tokio::test]
async fn cd2_cascade_delete_no_matching_jobs() {
    let (svc, _repo, bc) = setup().await;

    svc.add_job(make_create_req("Existing", every_60s()))
        .await
        .unwrap();
    bc.take_events();

    svc.delete_jobs_by_conversation(999).await;

    let events = bc.take_events();
    assert!(
        events.is_empty(),
        "no events should be emitted when no jobs match"
    );

    let all = svc.list_jobs(&ListCronJobsQuery::default()).await.unwrap();
    assert_eq!(all.len(), 1, "existing job should remain untouched");
}

// ── CD-3: OnConversationDelete trait dispatches cascade ──────────

#[tokio::test]
async fn cd3_on_conversation_delete_trait() {
    use nomifun_common::OnConversationDelete;

    let (svc, _repo, bc) = setup().await;

    let mut req = make_create_req("Trait Cascade", every_60s());
    req.conversation_id = 6;
    let job = svc.add_job(req).await.unwrap();
    bc.take_events();

    svc.on_conversation_deleted(6).await;

    assert!(svc.get_job(&job.id).await.is_err());

    let events = bc.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "cron.job-removed");
}
