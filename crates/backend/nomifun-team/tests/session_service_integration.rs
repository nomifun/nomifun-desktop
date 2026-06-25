mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use nomifun_ai_agent::task_manager::AgentFactory;
use nomifun_ai_agent::types::BuildTaskOptions;
use nomifun_ai_agent::{IWorkerTaskManager, WorkerTaskManagerImpl};
use nomifun_api_types::{AddAgentRequest, CreateTeamRequest, TeamAgentInput, WebSocketMessage};
use nomifun_common::{AgentKillReason, AgentType, AppError, PaginatedResult, ProviderWithModel};
use nomifun_db::models::{
    AcpSessionRow, AgentMetadataRow, ConversationRow, MessageRow, UpdateAgentHandshakeParams, UpsertAgentMetadataParams,
};
use nomifun_db::{
    ConversationFilters, ConversationRowUpdate, CreateAcpSessionParams, DbError, IAcpSessionRepository,
    IAgentMetadataRepository, IConversationRepository, IProviderRepository, ITeamRepository, MessageRowUpdate,
    MessageSearchRow, PersistedSessionState, SaveRuntimeStateParams, SortOrder,
};
use nomifun_realtime::EventBroadcaster;

use common::MockTeamRepo;
use nomifun_conversation::ConversationService;
use nomifun_team::TeamSessionService;

// ---------------------------------------------------------------------------
// Mock ConversationRepository — minimal impl for TeamSessionService tests
// ---------------------------------------------------------------------------

struct MockConversationRepo {
    conversations: std::sync::Mutex<Vec<ConversationRow>>,
}

impl MockConversationRepo {
    fn new() -> Self {
        Self {
            conversations: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl IConversationRepository for MockConversationRepo {
    async fn get(&self, id: i64) -> Result<Option<ConversationRow>, DbError> {
        let convs = self.conversations.lock().unwrap();
        Ok(convs.iter().find(|c| c.id == id).cloned())
    }
    async fn create(&self, row: &ConversationRow) -> Result<i64, DbError> {
        // Mirror SQLite AUTOINCREMENT: the service passes id:0 as a placeholder
        // and relies on the repo to mint the real integer PK. Allocate a fresh
        // 1-based id so every conversation gets a unique nonzero id.
        let mut convs = self.conversations.lock().unwrap();
        let new_id = convs.len() as i64 + 1;
        let mut stored = row.clone();
        stored.id = new_id;
        convs.push(stored);
        Ok(new_id)
    }
    async fn update(&self, id: i64, updates: &ConversationRowUpdate) -> Result<(), DbError> {
        let mut convs = self.conversations.lock().unwrap();
        let conv = convs
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| DbError::NotFound(id.to_string()))?;
        if let Some(ref extra) = updates.extra {
            conv.extra = extra.clone();
        }
        if let Some(ref name) = updates.name {
            conv.name = name.clone();
        }
        if let Some(pinned) = updates.pinned {
            conv.pinned = pinned;
        }
        if let Some(ref model) = updates.model {
            conv.model = model.clone();
        }
        if let Some(updated_at) = updates.updated_at {
            conv.updated_at = updated_at;
        }
        Ok(())
    }
    async fn delete(&self, id: i64) -> Result<(), DbError> {
        self.conversations.lock().unwrap().retain(|c| c.id != id);
        Ok(())
    }
    async fn list_paginated(
        &self,
        _user_id: &str,
        _filters: &ConversationFilters,
    ) -> Result<PaginatedResult<ConversationRow>, DbError> {
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
    ) -> Result<Option<ConversationRow>, DbError> {
        Ok(None)
    }
    async fn list_by_cron_job(&self, _user_id: &str, _cron_job_id: &str) -> Result<Vec<ConversationRow>, DbError> {
        Ok(vec![])
    }
    async fn list_associated(&self, _user_id: &str, _conversation_id: i64) -> Result<Vec<ConversationRow>, DbError> {
        Ok(vec![])
    }
    async fn get_messages(
        &self,
        _conv_id: i64,
        _page: u32,
        _page_size: u32,
        _order: SortOrder,
    ) -> Result<PaginatedResult<MessageRow>, DbError> {
        Ok(PaginatedResult {
            items: vec![],
            total: 0,
            has_more: false,
        })
    }
    async fn insert_message(&self, _message: &MessageRow) -> Result<(), DbError> {
        Ok(())
    }
    async fn update_message(&self, _id: &str, _updates: &MessageRowUpdate) -> Result<(), DbError> {
        Ok(())
    }
    async fn delete_messages_by_conversation(&self, _conv_id: i64) -> Result<(), DbError> {
        Ok(())
    }
    async fn get_message_by_msg_id(
        &self,
        _conv_id: i64,
        _msg_id: &str,
        _msg_type: &str,
    ) -> Result<Option<MessageRow>, DbError> {
        Ok(None)
    }
    async fn search_messages(
        &self,
        _user_id: &str,
        _keyword: &str,
        _page: u32,
        _page_size: u32,
    ) -> Result<PaginatedResult<MessageSearchRow>, DbError> {
        Ok(PaginatedResult {
            items: vec![],
            total: 0,
            has_more: false,
        })
    }
}

// ---------------------------------------------------------------------------
// NullBroadcaster — no-op event broadcaster
// ---------------------------------------------------------------------------

struct NullBroadcaster;
impl EventBroadcaster for NullBroadcaster {
    fn broadcast(&self, _msg: WebSocketMessage<serde_json::Value>) {}
}

#[derive(Default)]
struct RecordingBroadcaster {
    events: std::sync::Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
}

impl RecordingBroadcaster {
    fn new() -> Self {
        Self::default()
    }

    fn events_by_name(&self, name: &str) -> Vec<WebSocketMessage<serde_json::Value>> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.name == name)
            .cloned()
            .collect()
    }
}

impl EventBroadcaster for RecordingBroadcaster {
    fn broadcast(&self, msg: WebSocketMessage<serde_json::Value>) {
        self.events.lock().unwrap().push(msg);
    }
}

// ---------------------------------------------------------------------------
// Team repo: the shared full in-memory `common::MockTeamRepo` now backs teams,
// team_agents, mailbox, tasks, and task_deps as first-class tables (post
// primary-key-redesign), so the previous bespoke `FullMockTeamRepo` wrapper is
// no longer needed — `MockTeamRepo` is used directly.
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct StubSkillResolver;
#[async_trait::async_trait]
impl nomifun_conversation::skill_resolver::SkillResolver for StubSkillResolver {
    async fn auto_inject_names(&self) -> Vec<String> {
        Vec::new()
    }
    async fn resolve_skills(&self, _names: &[String]) -> Vec<nomifun_conversation::skill_resolver::ResolvedAgentSkill> {
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

#[derive(Default)]
struct StubAgentMetadataRepo {
    rows_by_id: HashMap<String, AgentMetadataRow>,
    builtin_by_backend: HashMap<String, AgentMetadataRow>,
}

impl StubAgentMetadataRepo {
    fn empty() -> Self {
        Self::default()
    }

    fn with_rows(rows: Vec<AgentMetadataRow>) -> Self {
        let mut repo = Self::default();
        for row in rows {
            if row.agent_source == "builtin"
                && let Some(backend) = row.backend.as_deref()
            {
                repo.builtin_by_backend.insert(backend.to_owned(), row.clone());
            }
            repo.rows_by_id.insert(row.id.clone(), row);
        }
        repo
    }
}

#[async_trait::async_trait]
impl IAgentMetadataRepository for StubAgentMetadataRepo {
    async fn list_all(&self) -> Result<Vec<AgentMetadataRow>, DbError> {
        Ok(self.rows_by_id.values().cloned().collect())
    }
    async fn get(&self, id: &str) -> Result<Option<AgentMetadataRow>, DbError> {
        Ok(self.rows_by_id.get(id).cloned())
    }
    async fn find_by_source_and_name(
        &self,
        agent_source: &str,
        name: &str,
    ) -> Result<Option<AgentMetadataRow>, DbError> {
        Ok(self
            .rows_by_id
            .values()
            .find(|row| row.agent_source == agent_source && row.name == name)
            .cloned())
    }
    async fn find_builtin_by_backend(&self, backend: &str) -> Result<Option<AgentMetadataRow>, DbError> {
        Ok(self.builtin_by_backend.get(backend).cloned())
    }
    async fn upsert(&self, _params: &UpsertAgentMetadataParams<'_>) -> Result<AgentMetadataRow, DbError> {
        Err(DbError::Init("stub".into()))
    }
    async fn apply_handshake(
        &self,
        _id: &str,
        _params: &UpdateAgentHandshakeParams<'_>,
    ) -> Result<Option<AgentMetadataRow>, DbError> {
        Ok(None)
    }
    async fn set_enabled(&self, _id: &str, _enabled: bool) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn delete(&self, _id: &str) -> Result<bool, DbError> {
        Ok(false)
    }
}

struct StubAcpSessionRepo;

#[async_trait::async_trait]
impl IAcpSessionRepository for StubAcpSessionRepo {
    async fn get(&self, _conversation_id: i64) -> Result<Option<AcpSessionRow>, DbError> {
        Ok(None)
    }
    async fn create(&self, params: &CreateAcpSessionParams<'_>) -> Result<AcpSessionRow, DbError> {
        Ok(AcpSessionRow {
            conversation_id: params.conversation_id,
            agent_backend: params.agent_backend.to_owned(),
            agent_source: params.agent_source.to_owned(),
            agent_id: params.agent_id.to_owned(),
            session_id: None,
            session_status: "created".to_owned(),
            session_config: "{}".to_owned(),
            last_active_at: None,
            suspended_at: None,
        })
    }
    async fn update_session_id(&self, _conversation_id: i64, _session_id: &str) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn clear_session_id(&self, _conversation_id: i64) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn delete(&self, _conversation_id: i64) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn load_runtime_state(&self, _conversation_id: i64) -> Result<Option<PersistedSessionState>, DbError> {
        Ok(None)
    }
    async fn save_runtime_state(
        &self,
        _conversation_id: i64,
        _params: &SaveRuntimeStateParams<'_>,
    ) -> Result<bool, DbError> {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Counting task manager — wraps WorkerTaskManagerImpl so tests can assert
// kill / get_or_build_task call counts by conversation id.
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct TaskManagerCalls {
    kill: Vec<(String, Option<AgentKillReason>)>,
    build: Vec<String>,
}

struct CountingTaskManager {
    inner: WorkerTaskManagerImpl,
    calls: Mutex<TaskManagerCalls>,
}

impl CountingTaskManager {
    fn new(factory: AgentFactory) -> Self {
        Self {
            inner: WorkerTaskManagerImpl::new(factory),
            calls: Mutex::new(TaskManagerCalls::default()),
        }
    }

    fn reset(&self) {
        self.inner.clear();
        *self.calls.lock().unwrap() = TaskManagerCalls::default();
    }

    fn snapshot(&self) -> TaskManagerCalls {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl IWorkerTaskManager for CountingTaskManager {
    fn get_task(&self, conversation_id: &str) -> Option<nomifun_ai_agent::AgentInstance> {
        self.inner.get_task(conversation_id)
    }
    async fn get_or_build_task(
        &self,
        conversation_id: &str,
        options: BuildTaskOptions,
    ) -> Result<nomifun_ai_agent::AgentInstance, AppError> {
        self.calls.lock().unwrap().build.push(conversation_id.to_owned());
        self.inner.get_or_build_task(conversation_id, options).await
    }
    fn kill(&self, conversation_id: &str, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.calls
            .lock()
            .unwrap()
            .kill
            .push((conversation_id.to_owned(), reason));
        self.inner.kill(conversation_id, reason)
    }
    fn kill_and_wait(
        &self,
        conversation_id: &str,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let _ = self.kill(conversation_id, reason);
        Box::pin(std::future::ready(()))
    }
    fn clear(&self) {
        self.inner.clear()
    }
    fn active_count(&self) -> usize {
        self.inner.active_count()
    }
    fn collect_idle(&self, idle_threshold_ms: nomifun_common::TimestampMs) -> Vec<String> {
        self.inner.collect_idle(idle_threshold_ms)
    }
}

// Minimal stub agent returned by the test factory: ensure_session only
// asks the task manager to kill + rebuild; the returned handle never has
// `send_message` called on it.
mod mock_agent {
    use nomifun_ai_agent::agent_task::{IAgentTask, IMockAgent};
    use nomifun_ai_agent::protocol::events::AgentStreamEvent;
    use nomifun_ai_agent::types::SendMessageData;
    use nomifun_common::{AgentKillReason, AgentType, AppError, Confirmation, ConversationStatus, TimestampMs};
    use tokio::sync::broadcast;

    pub struct MockAgent {
        pub conversation_id: String,
        pub workspace: String,
        pub event_tx: broadcast::Sender<AgentStreamEvent>,
        pub confirmations: Vec<Confirmation>,
    }

    impl MockAgent {
        pub fn new(conversation_id: String, workspace: String) -> Self {
            Self::with_confirmations(conversation_id, workspace, Vec::new())
        }

        pub fn with_confirmations(
            conversation_id: String,
            workspace: String,
            confirmations: Vec<Confirmation>,
        ) -> Self {
            let (event_tx, _) = broadcast::channel(16);
            Self {
                conversation_id,
                workspace,
                event_tx,
                confirmations,
            }
        }
    }

    #[async_trait::async_trait]
    impl IAgentTask for MockAgent {
        fn agent_type(&self) -> AgentType {
            AgentType::Acp
        }
        fn conversation_id(&self) -> &str {
            &self.conversation_id
        }
        fn workspace(&self) -> &str {
            &self.workspace
        }
        fn status(&self) -> Option<ConversationStatus> {
            None
        }
        fn last_activity_at(&self) -> TimestampMs {
            0
        }
        fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
            self.event_tx.subscribe()
        }
        async fn send_message(&self, _data: SendMessageData) -> Result<(), nomifun_ai_agent::AgentSendError> {
            Ok(())
        }
        async fn cancel(&self) -> Result<(), AppError> {
            Ok(())
        }
        fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
            Ok(())
        }
    }

    impl IMockAgent for MockAgent {
        fn get_confirmations(&self) -> Vec<Confirmation> {
            self.confirmations.clone()
        }
    }
}

fn success_factory() -> AgentFactory {
    use futures_util::FutureExt;
    Arc::new(|opts: BuildTaskOptions| {
        async move {
            Ok(nomifun_ai_agent::AgentInstance::Mock(Arc::new(
                mock_agent::MockAgent::new(opts.conversation_id, opts.workspace),
            )))
        }
        .boxed()
    })
}

fn confirmations_factory(count: usize) -> AgentFactory {
    use futures_util::FutureExt;
    use nomifun_common::Confirmation;
    Arc::new(move |opts: BuildTaskOptions| {
        let confirmations = (0..count)
            .map(|idx| Confirmation {
                id: format!("tool-{idx}"),
                call_id: format!("tool-{idx}"),
                title: None,
                action: None,
                description: format!("Confirm tool {idx}"),
                command_type: None,
                options: vec![],
            })
            .collect::<Vec<_>>();
        async move {
            Ok(nomifun_ai_agent::AgentInstance::Mock(Arc::new(
                mock_agent::MockAgent::with_confirmations(opts.conversation_id, opts.workspace, confirmations),
            )))
        }
        .boxed()
    })
}

struct EmptyProviderRepo;

#[async_trait::async_trait]
impl IProviderRepository for EmptyProviderRepo {
    async fn list(&self) -> Result<Vec<nomifun_db::models::Provider>, DbError> {
        Ok(vec![])
    }
    async fn find_by_id(&self, _id: &str) -> Result<Option<nomifun_db::models::Provider>, DbError> {
        Ok(None)
    }
    async fn create(
        &self,
        _params: nomifun_db::CreateProviderParams<'_>,
    ) -> Result<nomifun_db::models::Provider, DbError> {
        Err(DbError::NotFound("not implemented".into()))
    }
    async fn update(
        &self,
        _id: &str,
        _params: nomifun_db::UpdateProviderParams<'_>,
    ) -> Result<nomifun_db::models::Provider, DbError> {
        Err(DbError::NotFound("not implemented".into()))
    }
    async fn delete(&self, _id: &str) -> Result<(), DbError> {
        Err(DbError::NotFound("not implemented".into()))
    }
}

fn setup_with_factory(factory: AgentFactory) -> (Arc<TeamSessionService>, Arc<CountingTaskManager>) {
    setup_with_factory_and_metadata(factory, Arc::new(StubAgentMetadataRepo::empty()))
}

fn setup_with_factory_and_metadata(
    factory: AgentFactory,
    agent_metadata_repo: Arc<dyn IAgentMetadataRepository>,
) -> (Arc<TeamSessionService>, Arc<CountingTaskManager>) {
    let team_repo: Arc<dyn ITeamRepository> = Arc::new(MockTeamRepo::new());
    let conv_repo: Arc<dyn IConversationRepository> = Arc::new(MockConversationRepo::new());
    let broadcaster: Arc<dyn EventBroadcaster> = Arc::new(NullBroadcaster);
    let acp_session_repo: Arc<dyn IAcpSessionRepository> = Arc::new(StubAcpSessionRepo);
    let task_manager = Arc::new(CountingTaskManager::new(factory));
    let task_manager_dyn: Arc<dyn IWorkerTaskManager> = task_manager.clone();
    let conv_service = ConversationService::new(
        std::env::temp_dir(),
        broadcaster.clone(),
        Arc::new(StubSkillResolver),
        task_manager_dyn.clone(),
        conv_repo,
        agent_metadata_repo.clone(),
        acp_session_repo,
    );
    let backend_binary_path = Arc::new(std::path::PathBuf::from("/tmp/nomicore-test"));
    let provider_repo: Arc<dyn IProviderRepository> = Arc::new(EmptyProviderRepo);
    let svc = TeamSessionService::new(
        team_repo,
        agent_metadata_repo,
        provider_repo,
        conv_service,
        broadcaster,
        task_manager_dyn,
        backend_binary_path,
        None,
    );
    (svc, task_manager)
}

fn setup() -> Arc<TeamSessionService> {
    setup_with_factory(success_factory()).0
}

fn setup_with_recording_broadcaster() -> (Arc<TeamSessionService>, Arc<RecordingBroadcaster>) {
    let team_repo: Arc<dyn ITeamRepository> = Arc::new(MockTeamRepo::new());
    let conv_repo: Arc<dyn IConversationRepository> = Arc::new(MockConversationRepo::new());
    let recorder = Arc::new(RecordingBroadcaster::new());
    let broadcaster: Arc<dyn EventBroadcaster> = recorder.clone();
    let agent_metadata_repo: Arc<dyn IAgentMetadataRepository> = Arc::new(StubAgentMetadataRepo::empty());
    let acp_session_repo: Arc<dyn IAcpSessionRepository> = Arc::new(StubAcpSessionRepo);
    let task_manager: Arc<dyn IWorkerTaskManager> = Arc::new(CountingTaskManager::new(success_factory()));
    let conv_service = ConversationService::new(
        std::env::temp_dir(),
        broadcaster.clone(),
        Arc::new(StubSkillResolver),
        task_manager.clone(),
        conv_repo,
        agent_metadata_repo.clone(),
        acp_session_repo,
    );
    let backend_binary_path = Arc::new(std::path::PathBuf::from("/tmp/nomicore-test"));
    let provider_repo: Arc<dyn IProviderRepository> = Arc::new(EmptyProviderRepo);
    let svc = TeamSessionService::new(
        team_repo,
        agent_metadata_repo,
        provider_repo,
        conv_service,
        broadcaster,
        task_manager,
        backend_binary_path,
        None,
    );
    (svc, recorder)
}

fn make_agent_metadata_row(id: &str, backend: &str, icon: &str) -> AgentMetadataRow {
    AgentMetadataRow {
        id: id.to_owned(),
        icon: Some(icon.to_owned()),
        name: backend.to_owned(),
        name_i18n: None,
        description: None,
        description_i18n: None,
        backend: Some(backend.to_owned()),
        agent_type: "acp".to_owned(),
        agent_source: "builtin".to_owned(),
        agent_source_info: None,
        enabled: true,
        command: None,
        args: None,
        env: None,
        native_skills_dirs: None,
        behavior_policy: None,
        yolo_id: None,
        agent_capabilities: None,
        auth_methods: None,
        config_options: None,
        available_modes: None,
        available_models: None,
        available_commands: None,
        sort_order: 0,
        created_at: 0,
        updated_at: 0,
    }
}

fn setup_with_metadata_rows(rows: Vec<AgentMetadataRow>) -> Arc<TeamSessionService> {
    let agent_metadata_repo: Arc<dyn IAgentMetadataRepository> = Arc::new(StubAgentMetadataRepo::with_rows(rows));
    setup_with_factory_and_metadata(success_factory(), agent_metadata_repo).0
}

fn two_agent_input() -> Vec<TeamAgentInput> {
    vec![
        TeamAgentInput {
            name: "Lead".into(),
            role: "lead".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            conversation_id: None,
        },
        TeamAgentInput {
            name: "Worker".into(),
            role: "teammate".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            conversation_id: None,
        },
    ]
}

fn reset_auto_started_session(svc: &Arc<TeamSessionService>, tm: &Arc<CountingTaskManager>, team_id: &str) {
    svc.stop_session(team_id);
    tm.reset();
}

// ===========================================================================
// Test: Team CRUD (TC-*, TL-*, TG-*, TD-*, TR-*)
// ===========================================================================

#[tokio::test]
async fn tc1_create_team_with_multiple_agents() {
    let svc = setup();
    let resp = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "Alpha".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(resp.name, "Alpha");
    assert_eq!(resp.agents.len(), 2);
    assert_eq!(resp.agents[0].role, "lead");
    assert_eq!(resp.agents[1].role, "teammate");
    assert!(resp.lead_agent_id.is_some());
    assert_eq!(resp.lead_agent_id, Some(resp.agents[0].slot_id.clone()));
}

#[tokio::test]
async fn tc_create_team_uses_custom_agent_id_icon_lookup() {
    let svc = setup_with_metadata_rows(vec![make_agent_metadata_row(
        "agent_builtin_claude",
        "claude",
        "/api/assets/logos/ai-major/claude.svg",
    )]);

    let resp = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "Alpha".into(),
                agents: vec![TeamAgentInput {
                    name: "Lead".into(),
                    role: "lead".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: Some("agent_builtin_claude".into()),
                    conversation_id: None,
                }],
                workspace: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        resp.agents[0].icon.as_deref(),
        Some("/api/assets/logos/ai-major/claude.svg")
    );
}

#[tokio::test]
async fn ta_add_agent_uses_model_fallback_for_acp_backend() {
    let svc = setup_with_metadata_rows(vec![make_agent_metadata_row(
        "agent_builtin_codex",
        "codex",
        "/api/assets/logos/tools/coding/codex.svg",
    )]);

    let team = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "Alpha".into(),
                agents: vec![TeamAgentInput {
                    name: "Lead".into(),
                    role: "lead".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                    conversation_id: None,
                }],
                workspace: None,
            },
        )
        .await
        .unwrap();

    let added = svc
        .add_agent(
            "user1",
            &team.id,
            AddAgentRequest {
                name: "Coder".into(),
                role: "teammate".into(),
                backend: "acp".into(),
                model: "codex".into(),
                custom_agent_id: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(added.icon.as_deref(), Some("/api/assets/logos/tools/coding/codex.svg"));
}

#[tokio::test]
async fn tc2_create_single_agent_team() {
    let svc = setup();
    let resp = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "Solo".into(),
                agents: vec![TeamAgentInput {
                    name: "Lead".into(),
                    role: "lead".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                    conversation_id: None,
                }],
                workspace: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(resp.agents.len(), 1);
    assert_eq!(resp.agents[0].role, "lead");
}

#[tokio::test]
async fn tc4_first_agent_is_lead() {
    let svc = setup();
    let resp = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: vec![
                    TeamAgentInput {
                        name: "A".into(),
                        role: "teammate".into(),
                        backend: "acp".into(),
                        model: "claude".into(),
                        custom_agent_id: None,
                        conversation_id: None,
                    },
                    TeamAgentInput {
                        name: "B".into(),
                        role: "teammate".into(),
                        backend: "acp".into(),
                        model: "claude".into(),
                        custom_agent_id: None,
                        conversation_id: None,
                    },
                ],
                workspace: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(resp.agents[0].role, "lead");
    assert_eq!(resp.lead_agent_id, Some(resp.agents[0].slot_id.clone()));
}

#[tokio::test]
async fn tc5_empty_agents_returns_error() {
    let svc = setup();
    let result = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "Empty".into(),
                agents: vec![],
                workspace: None,
            },
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn tc3_each_agent_has_conversation_id() {
    let svc = setup();
    let resp = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    for agent in &resp.agents {
        assert!(agent.conversation_id != 0);
    }
    assert_ne!(resp.agents[0].conversation_id, resp.agents[1].conversation_id);
}

// -- List teams ---------------------------------------------------------------

#[tokio::test]
async fn tl1_empty_list() {
    let svc = setup();
    let list = svc.list_teams().await.unwrap();
    assert!(list.is_empty());
}

#[tokio::test]
async fn tl2_list_multiple_teams() {
    let svc = setup();
    svc.create_team(
        "user1",
        CreateTeamRequest {
            name: "A".into(),
            agents: two_agent_input(),
            workspace: None,
        },
    )
    .await
    .unwrap();
    svc.create_team(
        "user1",
        CreateTeamRequest {
            name: "B".into(),
            agents: two_agent_input(),
            workspace: None,
        },
    )
    .await
    .unwrap();

    let list = svc.list_teams().await.unwrap();
    assert_eq!(list.len(), 2);
}

#[tokio::test]
async fn tl_list_teams_includes_pending_confirmation_counts_without_rebuilding_tasks() {
    let (svc, task_manager) = setup_with_factory(confirmations_factory(2));
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "With Confirmations".into(),
                agents: vec![TeamAgentInput {
                    name: "Lead".into(),
                    role: "lead".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                    conversation_id: None,
                }],
                workspace: None,
            },
        )
        .await
        .unwrap();
    // DTO conversation_id is i64; the task manager is string-keyed (Option A).
    let conversation_id = created.agents[0].conversation_id.to_string();
    task_manager
        .get_or_build_task(
            &conversation_id,
            BuildTaskOptions {
                agent_type: AgentType::Acp,
                workspace: "/tmp/ws".into(),
                model: ProviderWithModel {
                    provider_id: "test".into(),
                    model: "claude".into(),
                    use_model: None,
                },
                conversation_id: conversation_id.clone(),
                extra: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
    let before = task_manager.snapshot();

    let list = svc.list_teams().await.unwrap();
    let after = task_manager.snapshot();

    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, created.id);
    assert_eq!(list[0].agents[0].pending_confirmations, 2);
    assert_eq!(after.build, before.build);
}

// -- Get team -----------------------------------------------------------------

#[tokio::test]
async fn tg1_get_existing_team() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "Alpha".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    let got = svc.get_team(&created.id).await.unwrap();
    assert_eq!(got.id, created.id);
    assert_eq!(got.name, "Alpha");
    assert_eq!(got.agents.len(), 2);
}

#[tokio::test]
async fn tg2_get_nonexistent_returns_error() {
    let svc = setup();
    let result = svc.get_team("nonexistent").await;
    assert!(result.is_err());
}

// -- Delete team --------------------------------------------------------------

#[tokio::test]
async fn td1_delete_existing_team() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.remove_team("user1", &created.id).await.unwrap();
    let list = svc.list_teams().await.unwrap();
    assert!(list.is_empty());
}

#[tokio::test]
async fn td6_delete_nonexistent_returns_error() {
    let svc = setup();
    let result = svc.remove_team("user1", "nonexistent").await;
    assert!(result.is_err());
}

// -- Rename team --------------------------------------------------------------

#[tokio::test]
async fn tr1_rename_existing_team() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "Old".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.rename_team(&created.id, "New Name").await.unwrap();
    let got = svc.get_team(&created.id).await.unwrap();
    assert_eq!(got.name, "New Name");
}

#[tokio::test]
async fn tr4_rename_nonexistent_returns_error() {
    let svc = setup();
    let result = svc.rename_team("nonexistent", "X").await;
    assert!(result.is_err());
}

// ===========================================================================
// Test: Agent Management (AA-*, AR-*, AN-*)
// ===========================================================================

#[tokio::test]
async fn aa1_add_agent_to_team() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: vec![TeamAgentInput {
                    name: "Lead".into(),
                    role: "lead".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                    conversation_id: None,
                }],
                workspace: None,
            },
        )
        .await
        .unwrap();

    let agent = svc
        .add_agent(
            "user1",
            &created.id,
            AddAgentRequest {
                name: "Worker".into(),
                role: "teammate".into(),
                backend: "acp".into(),
                model: "claude".into(),
                custom_agent_id: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(agent.name, "Worker");
    assert_eq!(agent.role, "teammate");
    assert!(agent.conversation_id != 0);

    let got = svc.get_team(&created.id).await.unwrap();
    assert_eq!(got.agents.len(), 2);
}

#[tokio::test]
async fn aa4_add_agent_to_nonexistent_team() {
    let svc = setup();
    let result = svc
        .add_agent(
            "user1",
            "nonexistent",
            AddAgentRequest {
                name: "X".into(),
                role: "teammate".into(),
                backend: "acp".into(),
                model: "claude".into(),
                custom_agent_id: None,
            },
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn ar1_remove_agent_from_team() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    let worker_slot = created.agents[1].slot_id.clone();
    svc.remove_agent("user1", &created.id, &worker_slot).await.unwrap();

    let got = svc.get_team(&created.id).await.unwrap();
    assert_eq!(got.agents.len(), 1);
    assert!(got.agents.iter().all(|a| a.slot_id != worker_slot));
}

#[tokio::test]
async fn ar4_remove_nonexistent_agent() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    let result = svc.remove_agent("user1", &created.id, "nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn an1_rename_agent() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    let slot_id = created.agents[1].slot_id.clone();
    svc.rename_agent(&created.id, &slot_id, "Senior Worker").await.unwrap();

    let got = svc.get_team(&created.id).await.unwrap();
    let agent = got.agents.iter().find(|a| a.slot_id == slot_id).unwrap();
    assert_eq!(agent.name, "Senior Worker");
}

#[tokio::test]
async fn an3_rename_nonexistent_agent() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    let result = svc.rename_agent(&created.id, "nonexistent", "X").await;
    assert!(result.is_err());
}

// ===========================================================================
// Test: Session Management (ES-*, SS-*)
// ===========================================================================

#[tokio::test]
async fn es1_ensure_session_creates_session() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&created.id).await.unwrap();
}

#[tokio::test]
async fn es2_ensure_session_is_idempotent() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&created.id).await.unwrap();
    svc.ensure_session(&created.id).await.unwrap();
}

#[tokio::test]
async fn es3_ensure_session_nonexistent_team() {
    let svc = setup();
    let result = svc.ensure_session("nonexistent").await;
    assert!(result.is_err());
}

// -- W5-D31b-2: team.mcpStatus service-layer broadcasts ---------------------
//
// The happy-path assertion (session_injecting → session_ready) would require
// `create_team` to succeed, but on this branch base `create_team` panics at
// conversation creation because `StubAcpSessionRepo::create` returns Err
// (pre-existing baseline break — same root cause `es1_ensure_session_creates_session`
// fails with on `feat/team-wave4-5` HEAD). We therefore only assert the
// `load_failed` broadcast end-to-end here; the remaining phase transitions
// (SessionInjecting / SessionReady / ConfigWriteFailed / SessionError) are
// covered by inline assertions that do not depend on `create_team`.

#[tokio::test]
async fn d31b2_ensure_session_broadcasts_load_failed_for_missing_team() {
    let (svc, recorder) = setup_with_recording_broadcaster();
    let err = svc.ensure_session("nonexistent-team-xyz").await.unwrap_err();
    assert!(matches!(err, nomifun_team::TeamError::TeamNotFound(_)));

    let load_failed = recorder
        .events_by_name("team.mcpStatus")
        .into_iter()
        .find(|e| {
            e.data
                .get("phase")
                .and_then(|v| v.as_str())
                .map(|s| s == "load_failed")
                .unwrap_or(false)
        })
        .expect("load_failed broadcast expected");
    assert_eq!(
        load_failed.data.get("team_id").and_then(|v| v.as_str()),
        Some("nonexistent-team-xyz")
    );
    assert!(load_failed.data.get("error").is_some());
}

#[tokio::test]
async fn ss1_stop_session() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&created.id).await.unwrap();
    svc.stop_session(&created.id);
}

#[tokio::test]
async fn ss3_stop_session_without_active_is_noop() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.stop_session(&created.id);
}

// ===========================================================================
// Test: Message sending requires active session (SM-*)
// ===========================================================================

#[tokio::test]
async fn sm4_send_message_no_session_returns_error() {
    let svc = setup();
    let result = svc.send_message("nonexistent", "Hello", None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn sm1_send_message_with_active_session() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&created.id).await.unwrap();
    svc.send_message(&created.id, "Hello team", None).await.unwrap();
}

#[tokio::test]
async fn sa_send_message_to_agent_with_active_session() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&created.id).await.unwrap();
    let worker_slot = created.agents[1].slot_id.clone();
    svc.send_message_to_agent(&created.id, &worker_slot, "Do this", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn sa3_send_message_to_nonexistent_agent() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&created.id).await.unwrap();
    let result = svc
        .send_message_to_agent(&created.id, "nonexistent", "Hello", None)
        .await;
    assert!(result.is_err());
}

// ===========================================================================
// Test: dispose_all
// ===========================================================================

#[tokio::test]
async fn dispose_all_cleans_up_sessions() {
    let svc = setup();
    let t1 = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "A".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();
    let t2 = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "B".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&t1.id).await.unwrap();
    svc.ensure_session(&t2.id).await.unwrap();

    svc.dispose_all();

    // After dispose, sessions are cleaned up.
    assert!(svc.get_session_scheduler(&t1.id).is_none());
    assert!(svc.get_session_scheduler(&t2.id).is_none());
}

// ===========================================================================
// Test: Delete team stops active session (TD-2 + integration)
// ===========================================================================

#[tokio::test]
async fn td_delete_team_stops_session() {
    let svc = setup();
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&created.id).await.unwrap();
    svc.remove_team("user1", &created.id).await.unwrap();

    let result = svc.send_message(&created.id, "Hello", None).await;
    assert!(result.is_err());
}

// ===========================================================================
// Test: D9 ensure_session kill + rebuild closed loop
// ===========================================================================

#[tokio::test]
async fn d9_ensure_session_kills_and_rebuilds_every_agent() {
    let (svc, tm) = setup_with_factory(success_factory());
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    reset_auto_started_session(&svc, &tm, &created.id);
    svc.ensure_session(&created.id).await.unwrap();

    // Two agents → kill called 2x and get_or_build_task called 2x, each with
    // the corresponding conversation_id. Order is agents-iteration order.
    let calls = tm.snapshot();
    assert_eq!(calls.kill.len(), 2, "expected 2 kill calls");
    assert_eq!(calls.build.len(), 2, "expected 2 build calls");
    for (i, agent) in created.agents.iter().enumerate() {
        // task manager records ids as Strings; DTO id is i64 (Option A).
        assert_eq!(calls.kill[i].0, agent.conversation_id.to_string());
        assert_eq!(calls.kill[i].1, Some(AgentKillReason::TeamMcpRebuild));
        assert_eq!(calls.build[i], agent.conversation_id.to_string());
    }
}

#[tokio::test]
async fn d9_ensure_session_persists_team_mcp_stdio_config() {
    // Each agent's conversation.extra must carry a `team_mcp_stdio_config`
    // object by the time the factory is called — that is what the rebuilt
    // ACP process will read to reach the MCP server.
    use futures_util::FutureExt;
    let (svc, _tm) = setup_with_factory(Arc::new(|opts: BuildTaskOptions| {
        async move {
            let extra_has_cfg = opts
                .extra
                .get("team_mcp_stdio_config")
                .and_then(|v| v.as_object())
                .is_some_and(|o| o.contains_key("port") && o.contains_key("slot_id"));
            assert!(
                extra_has_cfg,
                "factory called without team_mcp_stdio_config in extra: {:?}",
                opts.extra
            );
            Ok(nomifun_ai_agent::AgentInstance::Mock(Arc::new(
                mock_agent::MockAgent::new(opts.conversation_id, opts.workspace),
            )))
        }
        .boxed()
    }));

    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    svc.ensure_session(&created.id).await.unwrap();
}

#[tokio::test]
async fn d9_ensure_session_is_idempotent() {
    let (svc, tm) = setup_with_factory(success_factory());
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    reset_auto_started_session(&svc, &tm, &created.id);
    svc.ensure_session(&created.id).await.unwrap();
    svc.ensure_session(&created.id).await.unwrap();

    // Second call short-circuits — no additional kill/build calls.
    let calls = tm.snapshot();
    assert_eq!(calls.kill.len(), 2, "second ensure_session must not re-kill");
    assert_eq!(calls.build.len(), 2, "second ensure_session must not re-build");
}

#[tokio::test]
async fn d9_ensure_session_rollbacks_when_build_fails() {
    // Factory always fails → ensure_session must propagate error and not
    // insert into sessions, so send_message afterwards still errors.
    use futures_util::FutureExt;
    let failing_factory: AgentFactory = Arc::new(|_opts: BuildTaskOptions| {
        async move { Err(AppError::Internal("simulated build failure".into())) }.boxed()
    });
    let (svc, tm) = setup_with_factory(failing_factory);
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    reset_auto_started_session(&svc, &tm, &created.id);
    let result = svc.ensure_session(&created.id).await;
    assert!(result.is_err(), "ensure_session should propagate build error");

    // Rebuild aborts on the first warmup failure, so only the first agent
    // is killed/built. No session is inserted, so send_message still errors.
    let calls = tm.snapshot();
    assert_eq!(calls.kill.len(), 1);
    assert_eq!(calls.build.len(), 1);

    let send_result = svc.send_message(&created.id, "Hello", None).await;
    assert!(
        send_result.is_err(),
        "session must not be registered after build failure"
    );
}

// ===========================================================================
// Test: D11.5 remove_team cascades kill to every agent process
// ===========================================================================

// ===========================================================================
// Test: W4-D23 add_agent_locks — per-team serialization prevents last-writer-
// wins when two tasks race on add_agent.
// ===========================================================================

#[tokio::test]
async fn w4_d23_concurrent_add_agent_preserves_every_insertion() {
    // Two concurrent add_agent calls on the same team must both be persisted
    // (no silent drop from unsynchronized read-modify-write on the agents
    // JSON blob).
    let svc = Arc::new(setup());
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: vec![TeamAgentInput {
                    name: "Lead".into(),
                    role: "lead".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                    conversation_id: None,
                }],
                workspace: None,
            },
        )
        .await
        .unwrap();

    let svc_a = svc.clone();
    let team_id_a = created.id.clone();
    let task_a = tokio::spawn(async move {
        svc_a
            .add_agent(
                "user1",
                &team_id_a,
                AddAgentRequest {
                    name: "WorkerA".into(),
                    role: "teammate".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                },
            )
            .await
    });

    let svc_b = svc.clone();
    let team_id_b = created.id.clone();
    let task_b = tokio::spawn(async move {
        svc_b
            .add_agent(
                "user1",
                &team_id_b,
                AddAgentRequest {
                    name: "WorkerB".into(),
                    role: "teammate".into(),
                    backend: "acp".into(),
                    model: "claude".into(),
                    custom_agent_id: None,
                },
            )
            .await
    });

    let (a, b) = tokio::join!(task_a, task_b);
    a.unwrap().unwrap();
    b.unwrap().unwrap();

    let got = svc.get_team(&created.id).await.unwrap();
    assert_eq!(
        got.agents.len(),
        3,
        "both concurrent add_agent calls must be persisted (1 lead + 2 workers)"
    );
    let names: std::collections::HashSet<_> = got.agents.iter().map(|a| a.name.clone()).collect();
    assert!(names.contains("Lead"));
    assert!(names.contains("WorkerA"));
    assert!(names.contains("WorkerB"));
}

#[tokio::test]
async fn d115_remove_team_kills_every_agent_process() {
    let (svc, tm) = setup_with_factory(success_factory());
    let created = svc
        .create_team(
            "user1",
            CreateTeamRequest {
                name: "T".into(),
                agents: two_agent_input(),
                workspace: None,
            },
        )
        .await
        .unwrap();

    reset_auto_started_session(&svc, &tm, &created.id);
    // Bring two agents online — after ensure_session, active_count == 2.
    svc.ensure_session(&created.id).await.unwrap();
    assert_eq!(tm.active_count(), 2, "ensure_session must register 2 live agents");

    let before_kill = tm.snapshot().kill.len();

    svc.remove_team("user1", &created.id).await.unwrap();

    // remove_team must have issued one kill per agent with reason TeamDeleted,
    // and the task manager's active_count must drop back to 0.
    let calls = tm.snapshot();
    let new_kills = &calls.kill[before_kill..];
    assert_eq!(
        new_kills.len(),
        created.agents.len(),
        "remove_team must kill every agent once"
    );
    for (i, agent) in created.agents.iter().enumerate() {
        assert_eq!(new_kills[i].0, agent.conversation_id.to_string());
        assert_eq!(new_kills[i].1, Some(AgentKillReason::TeamDeleted));
    }
    assert_eq!(
        tm.active_count(),
        0,
        "every agent worker must be torn down after remove_team"
    );
}
