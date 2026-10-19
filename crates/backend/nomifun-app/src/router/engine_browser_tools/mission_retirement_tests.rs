//! Real Browser dispatcher/service and canonical/domain repositories, with a
//! bounded in-memory native adapter. No user browser, network or product data.
use super::*;
use nomifun_agent_contracts::{ActionId, AgentBindingValue, AgentPresetId, CorrelationId, EventId,
    IdempotencyKey, PresetRevisionRef, SemanticSessionEventDraft, SessionEventAppend, SessionEventKind,
    SessionEventPayloadRef};
use nomifun_agent_domain_wave2::Wave2HostContext;
use nomifun_browser_platform::{run_guard::NativeInputGate, runtime::{BrowserRuntime, BrowserRuntimeFactory,
    CreateBrowserRuntime, BrowserNativeSurfacePort, BrowserTabLifecycle, BrowserTabSnapshot},
    workspace::BrowserRetentionReason};
use nomifun_common::{generate_id, now_ms, AgentExecutionActor, AgentExecutionEventKind, AgentExecutionStatus,
    ExecutionAttemptStatus, ExecutionStepStatus};
use nomifun_db::{IAgentExecutionRepository, IRequirementRepository, SqliteAgentExecutionRepository,
    AgentExecutionAttemptSessionKind, CreateAgentExecutionParams, NewAgentExecutionParticipant,
    NewAgentExecutionStep, ReconcileAgentExecutionPlanParams, CreateAgentExecutionAttemptParams,
    NewAgentExecutionEvent, SettleAgentExecutionAttemptParams};
use serde_json::{json, Value};
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct NativeCounts { creates: AtomicUsize, closes: Arc<AtomicUsize> }
struct PageRuntime { snapshot: tokio::sync::Mutex<BrowserRuntimeSnapshot>, closes: Arc<AtomicUsize> }
#[async_trait::async_trait]
impl NativeInputGate for PageRuntime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
}
#[async_trait::async_trait]
impl BrowserRuntime for PageRuntime {
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { None }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> { Ok(self.snapshot.lock().await.clone()) }
    async fn execute(&self, command: BrowserTabCommand, _: tokio_util::sync::CancellationToken)
        -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        let mut snapshot = self.snapshot.lock().await;
        if let BrowserTabCommand::Create { url } = command {
            let target = BrowserTabTarget { tab_id: generate_id(), runtime_generation: snapshot.runtime_generation, document_generation: 1 };
            snapshot.active_tab_id = Some(target.tab_id.clone());
            snapshot.tabs.push(BrowserTabSnapshot { target, title: "task page".into(), url,
                lifecycle: BrowserTabLifecycle::Ready, load: None, can_go_back: false, can_go_forward: false,
                zoom_percent: 100, blocked_permissions: vec![], permission_requests: vec![], script_dialog: None,
                diagnostics: Default::default() });
        }
        snapshot.revision += 1;
        Ok(snapshot.clone())
    }
    async fn close(&self) -> Result<(), WorkspaceError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        self.snapshot.lock().await.tabs.clear();
        Ok(())
    }
}
#[async_trait::async_trait]
impl BrowserRuntimeFactory for NativeCounts {
    async fn create(&self, request: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(PageRuntime { closes: self.closes.clone(), snapshot: tokio::sync::Mutex::new(
            BrowserRuntimeSnapshot { runtime_generation: request.runtime_generation, revision: 0,
                active_tab_id: None, tabs: vec![], downloads: vec![] }) }))
    }
}

fn event(kind: AgentExecutionEventKind, payload: Value) -> NewAgentExecutionEvent {
    NewAgentExecutionEvent { event_type: kind, step_id: None, attempt_id: None,
        actor: AgentExecutionActor::system(), payload: payload.to_string() }
}

struct Mission {
    database: nomifun_db::Database,
    _root: tempfile::TempDir,
    sessions: nomifun_conversation::CanonicalAgentSessionOwner,
    repository: Arc<SqliteAgentExecutionRepository>,
    principal: PrincipalRef,
    session: AgentSessionId,
    operation: OperationId,
    turn_key: String,
    started: EventId,
    execution: Option<String>,
    step: String,
    attempt: String,
    owner: Arc<BrowserRoleOwner>,
    resources: Arc<BrowserResourceService>,
    counts: Arc<NativeCounts>,
    admission: Arc<BrowserTurnAdmission>,
}

struct RetryTurn { attempt: String, key: String, turn: Option<(AgentSessionId, OperationId, EventId)> }

impl Mission {
    async fn new(kind: Option<AgentExecutionAttemptSessionKind>) -> Self {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner_id = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.clone() };
        let store = nomifun_agent_session::AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
        let sessions = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
        let mut admission = super::tests::render_admission(&owner_id, "before-open");
        Arc::get_mut(&mut admission).unwrap().binding.operations = std::collections::BTreeSet::from(["navigate".into()]);
        let opened = sessions.open(principal.clone(), AgentBindingValue {
            preset_revision_ref: PresetRevisionRef { preset_id: AgentPresetId::from("browser-mission-fixture"), revision: 1,
                revision_digest: "a".repeat(64).into() }, resolved_snapshot_ref: admission.snapshot.clone(),
            typed_resource_bindings: vec![admission.binding.clone()], binding_version: 1,
        }, None, vec![], "task browser", 1).await.unwrap();
        let session = opened.session.agent_session_id;
        let repository = Arc::new(SqliteAgentExecutionRepository::new(database.pool().clone()));
        let (execution, step, attempt) = if let Some(kind) = kind {
            let automation = kind == AgentExecutionAttemptSessionKind::AutomationLead;
            let initial = if automation {
                let requirements = nomifun_db::SqliteRequirementRepository::new(database.pool().clone());
                requirements.insert(&nomifun_db::models::NewRequirementRow {
                    title: "browser mission".into(), content: "finish the original root".into(), tag: "browser-mission".into(),
                    order_key: String::new(), sort_seq: String::new(), status: "pending".into(), priority: 0,
                    completion_note: None, owner_conversation_id: None, owner_terminal_id: None, active_turn_started_at: None,
                    lease_expires_at: None, started_at: None, completed_at: None, attempt_count: 0, created_by: "user".into(),
                    extra: "{}".into(), created_at: 1, updated_at: 1,
                }).await.unwrap();
                let claim = requirements.claim_next_for_runner("browser-mission", Some(session.as_ref()), None, 30_000, now_ms())
                    .await.unwrap().unwrap().row;
                json!({"mode":"automation","source":{"requirement_id":claim.requirement_id,
                    "claim_generation":claim.claim_generation,"operation_id":"browser-mission-source"},"plan":{"steps":[]}})
            } else { json!({"mode":"explicit","plan":{"steps":[]}}) };
            let participant = generate_id();
            let provider = generate_id();
            let source_agent: String = sqlx::query_scalar("SELECT agent_id FROM agent_metadata WHERE source_key='agent_builtin_nomi'")
                .fetch_one(database.pool()).await.expect("the fixture uses the actual seeded Nomi Agent lineage");
            sqlx::query("INSERT INTO providers(provider_id,platform,name,base_url,auth_scheme,credentials_encrypted,enabled,created_at,updated_at) \
                VALUES(?,'openai','Browser mission fixture','https://example.invalid','bearer','',1,1,1)")
                .bind(&provider).execute(database.pool()).await.unwrap();
            let created = repository.create_execution_with_participants(&owner_id, &CreateAgentExecutionParams {
                goal: "finish original Browser mission".into(), status: AgentExecutionStatus::Planning,
                adaptation_policy: nomifun_common::AdaptationPolicy::Fixed, decision_policy: nomifun_common::DecisionPolicy::Automatic,
                delegation_policy: nomifun_common::DelegationPolicy::Automatic, max_parallel: 2, work_dir: None,
                lead_conversation_id: automation.then(|| session.as_ref().to_owned()), initial_plan_input: initial.to_string(),
            }, &[NewAgentExecutionParticipant { participant_id: participant.clone(), source_agent_id: source_agent,
                preset_id: None, preset_revision: None, agent_snapshot: None, provider_id: Some(provider), model: Some("fixture".into()),
                role: Some("builder".into()), capability: None, constraints: None, description: None, system_prompt: None,
                enabled_skills: "[]".into(), disabled_builtin_skills: "[]".into(), sort_order: 0 }],
                &event(AgentExecutionEventKind::Created, json!({}))).await.unwrap();
            let step = generate_id();
            let plan = repository.reconcile_plan(&owner_id, &created.execution_id, created.version, &ReconcileAgentExecutionPlanParams {
                goal: None, adaptation_policy: None, decision_policy: None, delegation_policy: None, keep_step_ids: vec![],
                new_participants: vec![], retire_participant_ids: vec![], new_dependencies: vec![], execution_status: AgentExecutionStatus::Running,
                new_steps: vec![NewAgentExecutionStep { step_id: step.clone(), title: "Browser task".into(), spec: "open an isolated page".into(),
                    role: Some("builder".into()), tool_policy: nomifun_common::AgentToolPolicy::Full, kind: nomifun_common::ExecutionStepKind::Agent,
                    agent_mode: Some(nomifun_common::AgentStepMode::Normal), profile: Some(json!({"kind":"general","needs_vision":false,
                        "needs_web_search":false,"needs_long_context":false,"needs_high_reasoning":false,"bulk":false,"managed_process_only":false}).to_string()),
                    fanout_group: None, control_policy: None, status: ExecutionStepStatus::Pending, assigned_participant_id: Some(participant.clone()),
                    assignment_score: Some(1.), assignment_rationale: None, assignment_source: Some(nomifun_common::ParticipantAssignmentSource::Planner),
                    assignment_locked: false, failure_policy: nomifun_common::StepFailurePolicy::FailExecution, preset_prompt: None, graph_x: None, graph_y: None }],
            }, &event(AgentExecutionEventKind::PlanChanged, json!({}))).await.unwrap();
            let queued = repository.create_attempt(&owner_id, &created.execution_id, &step, plan.steps[0].version, None,
                &CreateAgentExecutionAttemptParams { participant_id: Some(participant), start_immediately: false, trigger_reason: "initial".into(),
                    effective_config: "{}".into(), retry_after: None, runtime_state: None }, &event(AgentExecutionEventKind::AttemptChanged, json!({}))).await.unwrap();
            let attempt = queued.current_attempt.unwrap().attempt;
            repository.start_attempt(&owner_id, &created.execution_id, &step, queued.step.version, &attempt.attempt_id, attempt.version,
                session.as_ref(), kind, None, &event(AgentExecutionEventKind::AttemptChanged, json!({}))).await.unwrap();
            (Some(created.execution_id), step, attempt.attempt_id)
        } else { (None, String::new(), String::new()) };
        let turn_key = if execution.is_some() { format!("{attempt}:initial-turn") } else { "ordinary-user-turn".into() };
        let turn = sessions.start_turn(&principal, &session, &turn_key, json!({"content":"Use the task browser."})).await.unwrap();
        let started = store.read_turn_receipt(&session, &turn.operation_id).await.unwrap().started_event.unwrap().event_id;
        let current = Arc::get_mut(&mut admission).unwrap();
        current.key = TurnKey::new(&owner_id, session.as_ref(), turn.operation_id.as_ref());
        let descriptor = browser_workspace_provider::provider_descriptor(&current.provider, &current.binding).unwrap();
        current.authority = BrowserSessionAuthority::from_action_ids(&owner_id, session.as_ref(), ["browser/navigate"],
            browser_workspace_provider::browser_resource_binding(current.binding.clone(), descriptor).unwrap()).unwrap();
        let root = tempfile::tempdir().unwrap();
        crate::config::load_or_create_storage_generation(root.path()).unwrap();
        let counts = Arc::new(NativeCounts::default());
        let resources = Arc::new(BrowserResourceService::new(counts.clone()));
        let owner = BrowserRoleOwner::new(Some(resources.clone()), None, root.path().into(), None, store);
        owner.install_task_repository(repository.clone()).unwrap();
        owner.turns.lock().unwrap().insert(admission.key.clone(), admission.clone());
        let fixture = Self { database, _root: root, sessions, repository, principal, session, operation: turn.operation_id,
            turn_key, started, execution, step, attempt, owner, resources, counts, admission };
        fixture.navigate().await;
        fixture
    }

    fn append(&self, kind: &str, correlation: &str, payload: Value) -> SessionEventAppend {
        let id = generate_id();
        SessionEventAppend { agent_session_id: self.session.clone(), event_id: id.clone().into(), producer_id: "runtime_supervisor".into(),
            idempotency_key: id.into(), semantic_event: SemanticSessionEventDraft { kind: SessionEventKind(kind.into()), kind_version: 1,
                correlation_id: correlation.into(), causation_event_id: Some(self.started.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)) } }
    }

    async fn navigate(&self) {
        let tool = OperationId::from("browser-mission-tool");
        self.owner.effect_store.append_event(&self.append("tool/call-started", self.operation.as_ref(),
            json!({"operation_id":tool,"capability_id":"browser","action_id":"browser/navigate"}))).await.unwrap();
        self.owner.invoke_typed(Wave2TypedHostRequest { context: Wave2HostContext { control: Default::default(),
            principal: self.principal.clone(), agent_session_id: self.session.clone(), turn_id: self.operation.as_ref().into(),
            operation_id: tool, idempotency_key: IdempotencyKey::from("browser-mission-dispatch"), correlation_id: CorrelationId::from("browser-mission"),
            resolved_snapshot_ref: self.admission.snapshot.clone(), registry_generation: self.admission.registry_generation,
            capability_id: CapabilityId::from(BROWSER_MODULE_ID), action_id: ActionId::from("browser/navigate"), role_provider: Some(self.admission.provider.clone()),
            resource_bindings: vec![self.admission.binding.clone()], state: super::super::agent_wave2_host::tests::test_state_handle() },
            operation: Wave2TypedCapabilityOperation::BrowserNavigate { input: StrictJsonValue(json!({"url":"https://example.test/task"})) },
        }).await.unwrap();
        assert_eq!(self.counts.creates.load(Ordering::SeqCst), 1);
        assert!(!self.owner.effect_store.has_unsettled_effects(&self.session).await.unwrap());
        assert_eq!(self.workspace().await.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1);
    }

    async fn workspace(&self) -> Arc<BrowserWorkspace> {
        self.resources.get_for_agent_session(&self.principal.principal_id, self.session.as_ref()).await.unwrap().unwrap()
    }

    async fn running_child_browser_after_root_settles(&self) -> (AgentSessionId, Arc<BrowserWorkspace>) {
        let execution = self.execution.as_ref().unwrap();
        let owner = &self.principal.principal_id;
        let current = self.repository.get_step_detail(owner, execution, &self.step).await.unwrap().unwrap();
        let parent = current.current_attempt.unwrap().attempt;
        let participant = parent.participant_id.unwrap();
        let child_step = generate_id();
        let mut delegation = event(AgentExecutionEventKind::PlanChanged, json!({"change":"delegated_steps_appended"}));
        delegation.actor = AgentExecutionActor::agent(self.session.as_ref().to_owned(), Some(self.attempt.clone()));
        self.repository.append_steps_from_attempt(owner, execution, &nomifun_db::AppendAgentExecutionStepsFromAttemptParams {
            operation_id: "browser-mission-delegation".into(), caller_conversation_id: self.session.as_ref().into(),
            caller_step_id: self.step.clone(), caller_attempt_id: self.attempt.clone(),
            expected_caller_step_version: current.step.version, expected_caller_attempt_version: parent.version,
            new_dependencies: vec![], new_steps: vec![NewAgentExecutionStep { step_id: child_step.clone(), title: "independent child".into(),
                spec: "use only this child's Browser".into(), role: Some("builder".into()), tool_policy: nomifun_common::AgentToolPolicy::Full,
                kind: nomifun_common::ExecutionStepKind::Agent, agent_mode: Some(nomifun_common::AgentStepMode::Normal),
                profile: Some(json!({"kind":"general","needs_vision":false,"needs_web_search":false,"needs_long_context":false,
                    "needs_high_reasoning":false,"bulk":false,"managed_process_only":false}).to_string()), fanout_group: None, control_policy: None,
                status: ExecutionStepStatus::Pending, assigned_participant_id: Some(participant.clone()), assignment_score: Some(1.),
                assignment_rationale: None, assignment_source: Some(nomifun_common::ParticipantAssignmentSource::Planner), assignment_locked: false,
                failure_policy: nomifun_common::StepFailurePolicy::FailExecution, preset_prompt: None, graph_x: None, graph_y: None }],
        }, &delegation).await.unwrap();
        // Full workspace mutators remain exclusive. The root durably appends
        // its child first, then ends its own invocation before the child's
        // original Attempt is allowed to claim that workspace reservation.
        self.terminal(ExecutionAttemptStatus::Completed).await;
        self.settle(ExecutionAttemptStatus::Completed, ExecutionStepStatus::Completed, None, None, false).await;
        let child = self.sessions.open(self.principal.clone(), AgentBindingValue {
            preset_revision_ref: PresetRevisionRef { preset_id: AgentPresetId::from("browser-mission-fixture"), revision: 1, revision_digest: "a".repeat(64).into() },
            resolved_snapshot_ref: self.admission.snapshot.clone(), typed_resource_bindings: vec![self.admission.binding.clone()], binding_version: 1,
        }, None, vec![], "independent child browser", 1).await.unwrap().session.agent_session_id;
        let step = self.repository.get_step_detail(owner, execution, &child_step).await.unwrap().unwrap();
        let queued = self.repository.create_attempt(owner, execution, &child_step, step.step.version, None,
            &CreateAgentExecutionAttemptParams { participant_id: Some(participant), start_immediately: false, trigger_reason: "initial".into(),
                effective_config: "{}".into(), retry_after: None, runtime_state: None }, &event(AgentExecutionEventKind::AttemptChanged, json!({}))).await.unwrap();
        let attempt = queued.current_attempt.unwrap().attempt;
        self.repository.start_attempt(owner, execution, &child_step, queued.step.version, &attempt.attempt_id, attempt.version, child.as_ref(),
            AgentExecutionAttemptSessionKind::ChildAttempt, None, &event(AgentExecutionEventKind::AttemptChanged, json!({}))).await.unwrap();
        let turn = self.sessions.start_turn(&self.principal, &child, &format!("{}:initial-turn", attempt.attempt_id), json!({"content":"child work"})).await.unwrap();
        let started = self.owner.effect_store.read_turn_receipt(&child, &turn.operation_id).await.unwrap().started_event.unwrap().event_id;
        let mut admission = super::tests::render_admission(owner, child.as_ref());
        let current = Arc::get_mut(&mut admission).unwrap();
        current.key = TurnKey::new(owner, child.as_ref(), turn.operation_id.as_ref());
        current.binding.operations = std::collections::BTreeSet::from(["navigate".into()]);
        let descriptor = browser_workspace_provider::provider_descriptor(&current.provider, &current.binding).unwrap();
        current.authority = BrowserSessionAuthority::from_action_ids(owner, child.as_ref(), ["browser/navigate"],
            browser_workspace_provider::browser_resource_binding(current.binding.clone(), descriptor).unwrap()).unwrap();
        self.owner.turns.lock().unwrap().insert(admission.key.clone(), admission.clone());
        let call = OperationId::from("child-browser-tool");
        self.owner.effect_store.append_event(&SessionEventAppend { agent_session_id: child.clone(), event_id: "child-browser-call".into(),
            producer_id: "capability_host".into(), idempotency_key: "child-browser-call".into(), semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("tool/call-started".into()), kind_version: 1, correlation_id: turn.operation_id.as_ref().into(), causation_event_id: Some(started),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":call,"capability_id":"browser","action_id":"browser/navigate"}))) },
        }).await.unwrap();
        self.owner.invoke_typed(Wave2TypedHostRequest { context: Wave2HostContext { control: Default::default(), principal: self.principal.clone(),
            agent_session_id: child.clone(), turn_id: turn.operation_id.as_ref().into(), operation_id: call, idempotency_key: "child-browser-dispatch".into(),
            correlation_id: "child-browser".into(), resolved_snapshot_ref: admission.snapshot.clone(), registry_generation: admission.registry_generation,
            capability_id: BROWSER_MODULE_ID.into(), action_id: "browser/navigate".into(), role_provider: Some(admission.provider.clone()),
            resource_bindings: vec![admission.binding.clone()], state: super::super::agent_wave2_host::tests::test_state_handle() },
            operation: Wave2TypedCapabilityOperation::BrowserNavigate { input: StrictJsonValue(json!({"url":"https://example.test/child"})) },
        }).await.unwrap();
        let workspace = self.resources.get_for_agent_session(owner, child.as_ref()).await.unwrap().unwrap();
        assert_eq!(workspace.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1);
        assert_eq!(self.counts.creates.load(Ordering::SeqCst), 2);
        (child, workspace)
    }

    async fn terminal(&self, status: ExecutionAttemptStatus) -> EventId {
        self.terminal_for(&self.session, &self.operation, &self.started, status).await
    }

    async fn terminal_for(&self, session: &AgentSessionId, operation: &OperationId, started: &EventId,
        status: ExecutionAttemptStatus) -> EventId {
        let append = |kind: &str, correlation: &str, payload: Value| {
            let mut event = self.append(kind, correlation, payload);
            event.agent_session_id = session.clone();
            event.semantic_event.causation_event_id = Some(started.clone());
            event
        };
        if status == ExecutionAttemptStatus::Cancelled {
            self.sessions.cancel_exact_turn(&self.principal, session, "browser-mission-stop", operation).await.unwrap();
        } else {
            if status == ExecutionAttemptStatus::Completed {
                let source = self.owner.effect_store.read_turn_receipt(session, operation).await.unwrap().started_event.unwrap();
                let SessionEventPayloadRef::InlineJson(start) = source.payload else { panic!("fixture inline start"); };
                let root = start.0["source_message_id"].as_str().unwrap();
                let message = nomifun_agent_contracts::canonical_assistant_step_message_id(root, 1).unwrap();
                self.owner.effect_store.append_event(&append("runtime/progress-recorded", operation.as_ref(),
                    json!({"producer_seq":1,"event":{"event":"model_step_started","step":1}}))).await.unwrap();
                self.owner.effect_store.append_event(&append("message/content-part", &message,
                    json!({"turn_id":root,"content":"Original task finished."}))).await.unwrap();
                self.owner.effect_store.append_chat_completion(&append("message/completed", &message,
                    json!({"part_count":1,"content_digest":nomifun_agent_contracts::digest_bytes(b"Original task finished.")})),
                    &append("turn/completed", operation.as_ref(), json!({"model_steps":1})), operation).await.unwrap();
            } else {
                self.owner.effect_store.append_turn_terminal(&append("turn/failed", operation.as_ref(),
                    json!({"error":"controlled task failure"})), operation).await.unwrap();
            }
        }
        self.owner.settle_turn(&self.principal.principal_id, session.as_ref(), operation.as_ref()).await.unwrap();
        self.owner.finish_turn(&self.principal.principal_id, session.as_ref(), operation.as_ref()).await.unwrap();
        self.owner.effect_store.read_turn_receipt(session, operation).await.unwrap().terminal_event.unwrap().event_id
    }

    async fn canonical_question(&self, operation: &OperationId, started: &EventId,
        request: &nomifun_agent_contracts::AgentUserInputRequest) -> EventId {
        let mut terminal = self.append("turn/completed", operation.as_ref(), json!({"user_input_request":request}));
        terminal.semantic_event.causation_event_id = Some(started.clone());
        self.owner.effect_store.append_turn_terminal(&terminal, operation).await.unwrap();
        self.owner.settle_turn(&self.principal.principal_id, self.session.as_ref(), operation.as_ref()).await.unwrap();
        self.owner.finish_turn(&self.principal.principal_id, self.session.as_ref(), operation.as_ref()).await.unwrap();
        self.owner.effect_store.read_turn_receipt(&self.session, operation).await.unwrap().terminal_event.unwrap().event_id
    }

    async fn waiting_question(&self, operation: &OperationId, started: &EventId,
        request: &nomifun_agent_contracts::AgentUserInputRequest) -> EventId {
        let terminal = self.canonical_question(operation, started, request).await;
        let owner = &self.principal.principal_id; let execution = self.execution.as_ref().unwrap();
        let current = self.repository.get_step_detail(owner, execution, &self.step).await.unwrap().unwrap();
        let attempt = current.current_attempt.unwrap().attempt;
        self.repository.settle_attempt(owner, execution, &self.step, current.step.version, &self.attempt, attempt.version, None,
            &SettleAgentExecutionAttemptParams { expected_active_session_turn: None,
                expected_terminal_turn: Some(nomifun_db::AgentExecutionTerminalTurnGuard::InputRequired { conversation_id: self.session.as_ref().into(),
                    canonical_operation_id: operation.as_ref().into(), terminal_event_id: terminal.as_ref().into(), user_input_request: request.clone() }),
                attempt_status: ExecutionAttemptStatus::WaitingInput, step_status: ExecutionStepStatus::WaitingInput,
                execution_status: Some(AgentExecutionStatus::WaitingInput), question: Some(Some(request.question.clone())), error: Some(None),
                output_summary: None, output_files: None, tokens: None, retry_after: Some(None), runtime_state: Some(None), started_at: None,
                finished_at: None, loop_repeat_reset: None }, &event(AgentExecutionEventKind::DecisionRequested,
                    json!({"canonical_operation_id":operation,"terminal_event_id":terminal,"question":request.question,"user_input_request":request}))).await.unwrap();
        terminal
    }

    async fn continue_question(&self) -> RetryTurn {
        let owner = &self.principal.principal_id; let execution = self.execution.as_ref().unwrap();
        let before = self.repository.get_execution(owner, execution).await.unwrap().unwrap();
        let current = self.repository.get_step_detail(owner, execution, &self.step).await.unwrap().unwrap();
        let attempt = current.current_attempt.unwrap().attempt;
        let key = generate_id();
        let mut effects = nomifun_db::AttemptConversationEffects::default();
        effects.push_decision(key.clone(), "Use the original selected report.".into()).unwrap();
        let mut answered = event(AgentExecutionEventKind::DecisionAnswered, json!({"answered":true,"operation_id":key}));
        answered.actor = AgentExecutionActor::User { user_id: owner.clone() };
        self.repository.resume_waiting_attempt(owner, execution, before.version, &self.step, current.step.version, &self.attempt, attempt.version,
            &nomifun_db::AttemptConversationEffectParams { runtime_state: Some(effects.encode().unwrap()) }, &answered).await.unwrap();
        let turn = self.sessions.start_turn(&self.principal, &self.session, &key, json!({"content":"The original explicit answer."})).await.unwrap();
        let started = self.owner.effect_store.read_turn_receipt(&self.session, &turn.operation_id).await.unwrap().started_event.unwrap().event_id;
        RetryTurn { attempt: self.attempt.clone(), key, turn: Some((self.session.clone(), turn.operation_id, started)) }
    }

    async fn acknowledge_backoff(&self) -> (nomifun_db::PendingConversationCleanup, i64) {
        self.acknowledge_backoff_after(ExecutionAttemptStatus::Failed).await
    }

    async fn acknowledge_backoff_after(&self, canonical_status: ExecutionAttemptStatus) -> (nomifun_db::PendingConversationCleanup, i64) {
        self.terminal(canonical_status).await;
        let backoff = now_ms() + 30_000;
        self.settle(ExecutionAttemptStatus::Failed, ExecutionStepStatus::Pending, None, Some(backoff), false).await;
        assert!(self.retire().await); self.assert_retained().await;
        let cleanup = self.repository.list_pending_conversation_cleanups(self.execution.as_deref(), None, 100).await.unwrap().cleanups.remove(0);
        assert!(self.repository.mark_conversation_cleanup_completed_exact(&cleanup, now_ms()).await.unwrap());
        (cleanup, backoff)
    }

    async fn retry_attempt(&self, start: bool) -> RetryTurn {
        let owner = &self.principal.principal_id; let execution = self.execution.as_ref().unwrap();
        let before = self.repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
        let step = before.steps.iter().find(|step| step.step_id == self.step).unwrap();
        let mut requested = event(AgentExecutionEventKind::StepChanged, json!({"change":"retry_requested"}));
        requested.actor = AgentExecutionActor::User { user_id: owner.clone() };
        // The existing explicit retry command can end an automatic backoff;
        // no clock, product deadline or old Attempt metadata is changed here.
        let reset = self.repository.reset_steps_for_retry(owner, execution, before.execution.version,
            &[nomifun_db::RetryAgentExecutionStep { step_id: self.step.clone(), expected_step_version: step.version }], &requested).await.unwrap();
        let step = reset.steps.iter().find(|step| step.step_id == self.step).unwrap();
        let original = self.repository.get_attempt(owner, execution, &self.step, &self.attempt).await.unwrap().unwrap();
        let queued = self.repository.create_attempt(owner, execution, &self.step, step.version, None,
            &CreateAgentExecutionAttemptParams { participant_id: original.attempt.participant_id, start_immediately: false,
                trigger_reason: "retry".into(), effective_config: "{}".into(), retry_after: None, runtime_state: None },
            &event(AgentExecutionEventKind::AttemptChanged, json!({}))).await.unwrap();
        let attempt = queued.current_attempt.unwrap().attempt;
        let key = format!("{}:initial-turn", attempt.attempt_id);
        if !start { return RetryTurn { attempt: attempt.attempt_id, key, turn: None }; }
        let session = self.sessions.open(self.principal.clone(), AgentBindingValue {
            preset_revision_ref: PresetRevisionRef { preset_id: AgentPresetId::from("browser-mission-fixture"), revision: 1, revision_digest: "a".repeat(64).into() },
            resolved_snapshot_ref: self.admission.snapshot.clone(), typed_resource_bindings: vec![self.admission.binding.clone()], binding_version: 1,
        }, None, vec![], "explicit successor", 1).await.unwrap().session.agent_session_id;
        self.repository.start_attempt(owner, execution, &self.step, queued.step.version, &attempt.attempt_id, attempt.version,
            session.as_ref(), AgentExecutionAttemptSessionKind::ChildAttempt, None, &event(AgentExecutionEventKind::AttemptChanged, json!({}))).await.unwrap();
        let turn = self.sessions.start_turn(&self.principal, &session, &key, json!({"content":"explicit successor attempt"})).await.unwrap();
        let started = self.owner.effect_store.read_turn_receipt(&session, &turn.operation_id).await.unwrap().started_event.unwrap().event_id;
        RetryTurn { attempt: attempt.attempt_id, key, turn: Some((session, turn.operation_id, started)) }
    }

    async fn finish_retry(&self, retry: &RetryTurn, status: ExecutionAttemptStatus) {
        let (session, operation, started) = retry.turn.as_ref().unwrap();
        self.terminal_for(session, operation, started, status).await;
        let current = self.repository.get_step_detail(&self.principal.principal_id, self.execution.as_ref().unwrap(), &self.step).await.unwrap().unwrap();
        let attempt = current.current_attempt.unwrap().attempt;
        assert_eq!(attempt.attempt_id, retry.attempt);
        let step_status = if status == ExecutionAttemptStatus::Completed { ExecutionStepStatus::Completed } else { ExecutionStepStatus::Failed };
        self.repository.settle_attempt(&self.principal.principal_id, self.execution.as_ref().unwrap(), &self.step, current.step.version,
            &retry.attempt, attempt.version, None, &SettleAgentExecutionAttemptParams { expected_active_session_turn: None, expected_terminal_turn: None,
                attempt_status: status, step_status, execution_status: None, question: Some(None), error: Some(None), output_summary: None,
                output_files: None, tokens: None, retry_after: Some(None), runtime_state: Some(None), started_at: None,
                finished_at: Some(Some(now_ms())), loop_repeat_reset: None }, &event(AgentExecutionEventKind::AttemptChanged,
                    json!({"attempt_status":status,"step_status":step_status,"turn_operation_key":retry.key}))).await.unwrap();
    }

    async fn independent_user_browser(&self) -> (AgentSessionId, Arc<BrowserWorkspace>) {
        let session = self.sessions.open(self.principal.clone(), AgentBindingValue {
            preset_revision_ref: PresetRevisionRef { preset_id: AgentPresetId::from("browser-mission-fixture"), revision: 1, revision_digest: "a".repeat(64).into() },
            resolved_snapshot_ref: self.admission.snapshot.clone(), typed_resource_bindings: vec![self.admission.binding.clone()], binding_version: 1,
        }, None, vec![], "independent user page", 1).await.unwrap().session.agent_session_id;
        let key = nomifun_browser_platform::workspace::managed_workspace_key(&self.principal.principal_id, session.as_ref()).unwrap();
        let profile = nomifun_browser_platform::runtime::BrowserProfileStore::new(self._root.path()).unwrap()
            .profile_for(&key, nomifun_browser_platform::runtime::BrowserProfilePersistence::Persistent).unwrap();
        let workspace = self.resources.ensure_user(&self.principal.principal_id, session.as_ref(), profile).await.unwrap();
        workspace.user_command(BrowserTabCommand::Create { url: "https://example.test/independent-edit".into() }).await.unwrap();
        (session, workspace)
    }

    async fn settle(&self, status: ExecutionAttemptStatus, step_status: ExecutionStepStatus,
        runtime_state: Option<String>, retry_after: Option<i64>, canonical_cancel: bool) {
        let execution = self.execution.as_ref().unwrap();
        let current = self.repository.get_step_detail(&self.principal.principal_id, execution, &self.step).await.unwrap().unwrap();
        let attempt = current.current_attempt.unwrap().attempt;
        let receipt = self.owner.effect_store.read_turn_receipt(&self.session, &self.operation).await.unwrap();
        let terminal = receipt.terminal_event.unwrap();
        let guard = canonical_cancel.then(|| nomifun_db::AgentExecutionTerminalTurnGuard::Cancelled {
            conversation_id: self.session.as_ref().into(), canonical_operation_id: self.operation.as_ref().into(), terminal_event_id: terminal.event_id.as_ref().into() });
        let payload = if canonical_cancel { json!({"attempt_status":status,"step_status":step_status,
            "canonical_operation_id":self.operation,"terminal_event_id":terminal.event_id}) }
            else { json!({"attempt_status":status,"step_status":step_status,"turn_operation_key":self.turn_key}) };
        self.repository.settle_attempt(&self.principal.principal_id, execution, &self.step, current.step.version, &self.attempt,
            attempt.version, None, &SettleAgentExecutionAttemptParams { expected_active_session_turn: None, expected_terminal_turn: guard,
                attempt_status: status, step_status, execution_status: None, question: Some(None), error: Some(None),
                output_summary: None, output_files: None, tokens: None, retry_after: Some(retry_after), runtime_state: Some(runtime_state),
                started_at: None, finished_at: Some(Some(now_ms())), loop_repeat_reset: None },
            &event(AgentExecutionEventKind::AttemptChanged, payload)).await.unwrap();
    }

    async fn retire(&self) -> bool {
        self.owner.retire_completed_task_resources(&self.principal.principal_id, &self.session, &self.operation).await.unwrap()
    }
    async fn assert_retired(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while self.resources.get_for_agent_session(&self.principal.principal_id, self.session.as_ref()).await.unwrap().is_some() {
                tokio::task::yield_now().await;
            }
        }).await.expect("the original native owner retires without closing Session history");
        assert_eq!(self.counts.closes.load(Ordering::SeqCst), 1);
        self.owner.effect_store.get_live_session(&self.session).await.unwrap();
    }
    async fn assert_retained(&self) {
        assert_eq!(self.counts.closes.load(Ordering::SeqCst), 0);
        assert!(!self.workspace().await.snapshot().await.unwrap().runtime.unwrap().tabs.is_empty());
    }
    async fn close(self) { self.resources.shutdown().await.unwrap(); self.database.close().await; }
}

fn replacement_task(participant: &str) -> NewAgentExecutionStep {
    NewAgentExecutionStep { step_id: generate_id(), title: "replacement task".into(), spec: "new explicitly configured task".into(),
        role: Some("builder".into()), tool_policy: nomifun_common::AgentToolPolicy::Full, kind: nomifun_common::ExecutionStepKind::Agent,
        agent_mode: Some(nomifun_common::AgentStepMode::Normal), profile: Some(json!({"kind":"general","needs_vision":false,"needs_web_search":false,
            "needs_long_context":false,"needs_high_reasoning":false,"bulk":false,"managed_process_only":false}).to_string()), fanout_group: None, control_policy: None,
        status: ExecutionStepStatus::Pending, assigned_participant_id: Some(participant.into()), assignment_score: Some(1.), assignment_rationale: None,
        assignment_source: Some(nomifun_common::ParticipantAssignmentSource::Planner), assignment_locked: false,
        failure_policy: nomifun_common::StepFailurePolicy::FailExecution, preset_prompt: None, graph_x: None, graph_y: None }
}

#[tokio::test]
async fn formal_autowork_root_completed_retires_its_own_browser_before_aggregate_finishes() {
    let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::AutomationLead)).await;
    let (child, child_workspace) = fixture.running_child_browser_after_root_settles().await;
    let execution = fixture.repository.get_execution(&fixture.principal.principal_id, fixture.execution.as_ref().unwrap()).await.unwrap().unwrap();
    assert_eq!(execution.status, "running", "aggregate ancestry alone is not another Browser purpose");
    let links = fixture.repository.resolve_conversation_link(&fixture.principal.principal_id, fixture.session.as_ref()).await.unwrap();
    assert!(links.iter().any(|link| link.relation == "automation" && !link.active));
    assert!(!links.iter().any(|link| link.relation == "attempt"));
    fixture.retire().await; fixture.assert_retired().await;
    assert!(fixture.owner.effect_store.head(&child).await.unwrap().active_turn_id.is_some());
    assert!(Arc::ptr_eq(&child_workspace, &fixture.resources.get_for_agent_session(&fixture.principal.principal_id, child.as_ref()).await.unwrap().unwrap()));
    assert_eq!(child_workspace.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1, "root retirement preserves the running child's exact Browser");
    fixture.close().await;
}

#[tokio::test]
async fn formal_child_failure_and_canonical_cancellation_retire_agent_only_pages() {
    for status in [ExecutionAttemptStatus::Failed, ExecutionAttemptStatus::Cancelled] {
        let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
        fixture.terminal(status).await;
        fixture.settle(status, if status == ExecutionAttemptStatus::Failed { ExecutionStepStatus::Failed } else { ExecutionStepStatus::Cancelled },
            None, None, status == ExecutionAttemptStatus::Cancelled).await;
        fixture.retire().await; fixture.assert_retired().await; fixture.close().await;
    }
}

#[tokio::test]
async fn formal_step_stop_whole_cancel_and_soft_delete_retire_the_original_browser() {
    for mode in ["step", "whole", "delete"] {
        let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
        let owner = &fixture.principal.principal_id; let execution = fixture.execution.as_ref().unwrap();
        let before = fixture.repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
        if mode == "step" {
            let mut stop = event(AgentExecutionEventKind::StepChanged, json!({}));
            stop.actor = AgentExecutionActor::User { user_id: owner.clone() };
            fixture.repository.cancel_step(owner, execution, before.execution.version, &fixture.step, before.steps[0].version,
                nomifun_common::AgentExecutionCancellationScope::Task, &stop).await.unwrap();
        } else if mode == "whole" {
            fixture.repository.cancel_execution(owner, execution, before.execution.version,
                &event(AgentExecutionEventKind::StatusChanged, json!({"status":"cancelled","reason":"cancelled_by_caller"}))).await.unwrap();
        } else {
            fixture.repository.delete_execution(owner, execution, before.execution.version,
                &event(AgentExecutionEventKind::Deleted, json!({"deleted":true}))).await.unwrap();
            assert!(fixture.repository.get_execution(owner, execution).await.unwrap().is_none(), "business visibility stays closed");
        }
        fixture.terminal(ExecutionAttemptStatus::Cancelled).await;
        fixture.retire().await; fixture.assert_retired().await; fixture.close().await;
    }
}

#[tokio::test]
async fn formal_fatal_aggregate_failure_retires_its_cancelled_original_browser_task() {
    let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
    let owner = &fixture.principal.principal_id; let execution = fixture.execution.as_ref().unwrap();
    let before = fixture.repository.get_execution(owner, execution).await.unwrap().unwrap();
    let lease = nomifun_db::AgentExecutionLeaseToken::new("browser-mission-fatal-owner".into());
    fixture.repository.try_acquire_lease(execution, before.version, lease.owner(), now_ms() + 30_000).await.unwrap().unwrap();
    fixture.repository.fail_active_execution(owner, execution, before.version, &lease, "controlled scheduler failure",
        &event(AgentExecutionEventKind::StatusChanged,
            json!({"status":"failed","lead_report_operation_id":null,"reason":"controlled scheduler failure"}))).await.unwrap();
    fixture.terminal(ExecutionAttemptStatus::Cancelled).await;
    fixture.retire().await; fixture.assert_retired().await; fixture.close().await;
}

#[tokio::test]
async fn formal_retirement_preserves_retry_review_pending_control_and_user_pages() {
    for reason in ["retry", "review", "pending", "human", "no_stop"] {
        let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
        if reason == "human" { fixture.workspace().await.user_command(BrowserTabCommand::Create { url: "https://example.test/user-edit".into() }).await.unwrap(); }
        fixture.terminal(ExecutionAttemptStatus::Failed).await;
        let mut effects = nomifun_db::AttemptConversationEffects::default();
        if reason == "review" { effects.review_blocked = Some(nomifun_db::models::RecoveryReviewBlock { kind: "fixture".into(),
            operation_id: fixture.operation.as_ref().into(), receipt_state: "unknown".into(), reason: "original recovery responsibility".into() }); }
        if reason == "pending" { effects.push_stop_turn(generate_id(), fixture.operation.as_ref().into()).unwrap(); }
        let step = match reason { "retry" => ExecutionStepStatus::Pending, "no_stop" => ExecutionStepStatus::Cancelled, _ => ExecutionStepStatus::Failed };
        fixture.settle(ExecutionAttemptStatus::Failed, step, Some(effects.encode().unwrap()),
            matches!(reason, "retry" | "no_stop").then(|| now_ms() + 30_000), false).await;
        fixture.retire().await; fixture.assert_retained().await; fixture.close().await;
    }
}

#[tokio::test]
async fn formal_retirement_does_not_close_an_active_successor_or_an_ordinary_user_turn() {
    let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
    fixture.terminal(ExecutionAttemptStatus::Failed).await;
    fixture.settle(ExecutionAttemptStatus::Failed, ExecutionStepStatus::Failed, None, None, false).await;
    let successor = fixture.sessions.start_turn(&fixture.principal, &fixture.session, "explicit-successor", json!({"content":"continue working"})).await.unwrap();
    let original = fixture.repository.list_pending_conversation_cleanups(fixture.execution.as_deref(), None, 100).await.unwrap().cleanups.remove(0);
    assert!(!fixture.retire().await, "the old Browser purpose has not been handed to its worker while a canonical successor is active");
    fixture.assert_retained().await;
    assert!(fixture.repository.list_pending_conversation_cleanups(fixture.execution.as_deref(), None, 100).await.unwrap().cleanups
        .iter().any(|cleanup| cleanup.link_id == original.link_id), "deferred responsibility must leave the original outbox pending");
    fixture.sessions.cancel_exact_turn(&fixture.principal, &fixture.session, "end-only-successor", &successor.operation_id).await.unwrap();
    assert!(fixture.retire().await, "the same original resource-only retry is accepted after its successor ends");
    fixture.assert_retired().await;
    assert!(fixture.repository.mark_conversation_cleanup_completed_exact(&original, now_ms()).await.unwrap());
    assert_eq!(fixture.counts.creates.load(Ordering::SeqCst), 1, "resource retry cannot dispatch Browser work or create another carrier");
    fixture.close().await;
    let ordinary = Mission::new(None).await;
    ordinary.terminal(ExecutionAttemptStatus::Completed).await;
    ordinary.retire().await; ordinary.assert_retained().await;
    assert_eq!(ordinary.resources.observe_workspace(&ordinary.principal.principal_id, ordinary.session.as_ref()).await.unwrap().retention,
        BrowserRetentionReason::TaskCompletionUnknown);
    ordinary.close().await;
}

#[tokio::test]
async fn formal_stop_after_acknowledged_failed_backoff_retires_without_rewriting_the_attempt() {
    for mode in ["step", "whole", "delete"] {
        let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
        let terminal = fixture.terminal(ExecutionAttemptStatus::Failed).await;
        let backoff = now_ms() + 30_000;
        fixture.settle(ExecutionAttemptStatus::Failed, ExecutionStepStatus::Pending, None, Some(backoff), false).await;
        assert!(fixture.retire().await);
        fixture.assert_retained().await;
        let owner = &fixture.principal.principal_id; let execution = fixture.execution.as_ref().unwrap();
        let old = fixture.repository.list_pending_conversation_cleanups(Some(execution), None, 100).await.unwrap().cleanups.remove(0);
        assert!(fixture.repository.mark_conversation_cleanup_completed_exact(&old, now_ms()).await.unwrap());
        assert!(fixture.repository.list_pending_conversation_cleanups(Some(execution), None, 100).await.unwrap().cleanups.is_empty());
        let before = fixture.repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
        if mode == "step" {
            let mut stop = event(AgentExecutionEventKind::StepChanged, json!({}));
            stop.actor = AgentExecutionActor::User { user_id: owner.clone() };
            fixture.repository.cancel_step(owner, execution, before.execution.version, &fixture.step, before.steps[0].version,
                nomifun_common::AgentExecutionCancellationScope::Task, &stop).await.unwrap();
        } else if mode == "whole" {
            fixture.repository.cancel_execution(owner, execution, before.execution.version,
                &event(AgentExecutionEventKind::StatusChanged, json!({"status":"cancelled","reason":"cancelled_by_caller"}))).await.unwrap();
        } else {
            fixture.repository.delete_execution(owner, execution, before.execution.version,
                &event(AgentExecutionEventKind::Deleted, json!({"deleted":true}))).await.unwrap();
        }
        let rearmed = fixture.repository.list_pending_conversation_cleanups(Some(execution), None, 100).await.unwrap().cleanups.remove(0);
        assert_eq!(rearmed.link_id, old.link_id);
        assert_eq!(rearmed.target, old.target, "Stop re-arms only the original invocation's cleanup");
        let facts = fixture.repository.read_task_retirement_facts(owner, fixture.session.as_ref(), execution, &fixture.step, &fixture.attempt).await.unwrap().unwrap();
        let attempt = &facts.detail.current_attempt.as_ref().unwrap().attempt;
        assert_eq!(facts.detail.step.status, "cancelled");
        assert_eq!(attempt.status, "failed");
        assert_eq!(attempt.retry_after, Some(backoff), "retry history is retained rather than normalized into a new result");
        assert!(fixture.retire().await); fixture.assert_retired().await;
        let receipt = fixture.owner.effect_store.read_turn_receipt(&fixture.session, &fixture.operation).await.unwrap();
        assert_eq!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Failed);
        assert_eq!(receipt.terminal_event.unwrap().event_id, terminal, "a resource-only Stop cannot rewrite or duplicate the original failed Turn");
        assert_eq!(fixture.counts.creates.load(Ordering::SeqCst), 1);
        assert!(fixture.repository.mark_conversation_cleanup_completed_exact(&rearmed, now_ms()).await.unwrap());
        fixture.close().await;
    }
}

#[tokio::test]
async fn formal_successor_result_retires_the_acknowledged_old_attempt_and_keeps_its_original_facts() {
    for status in [ExecutionAttemptStatus::Completed, ExecutionAttemptStatus::Failed] {
        let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
        let (old, backoff) = fixture.acknowledge_backoff().await;
        let cursor = fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap();
        let retry = fixture.retry_attempt(true).await;
        fixture.retire().await; fixture.assert_retained().await;
        fixture.finish_retry(&retry, status).await;
        let facts = fixture.repository.read_task_retirement_facts(&fixture.principal.principal_id, fixture.session.as_ref(), fixture.execution.as_ref().unwrap(),
            &fixture.step, &fixture.attempt).await.unwrap().unwrap();
        assert_eq!(facts.original_attempt.attempt.attempt_id, fixture.attempt);
        assert_eq!(facts.original_attempt.attempt.status, "failed");
        assert_eq!(facts.original_attempt.attempt.retry_after, Some(backoff));
        assert_eq!(facts.detail.current_attempt.as_ref().unwrap().attempt.attempt_id, retry.attempt);
        assert_ne!(facts.original_attempt.conversation_id, facts.detail.current_attempt.as_ref().unwrap().conversation_id);
        let rearmed = fixture.repository.list_pending_conversation_cleanups(fixture.execution.as_deref(), None, 100).await.unwrap().cleanups;
        assert!(rearmed.iter().any(|cleanup| cleanup.link_id == old.link_id && cleanup.target == old.target));
        assert!(fixture.retire().await); fixture.assert_retired().await;
        assert_eq!(fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap(), cursor, "retirement never emits input, another terminal or new tools");
        assert_eq!(fixture.counts.creates.load(Ordering::SeqCst), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn formal_stop_of_a_queued_successor_retires_old_backoff_without_starting_it_or_closing_another_session() {
    let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
    let (old, backoff) = fixture.acknowledge_backoff().await;
    let cursor = fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap();
    let retry = fixture.retry_attempt(false).await;
    assert!(retry.turn.is_none());
    let (independent, independent_workspace) = fixture.independent_user_browser().await;
    let owner = &fixture.principal.principal_id; let execution = fixture.execution.as_ref().unwrap();
    let before = fixture.repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
    let mut stop = event(AgentExecutionEventKind::StepChanged, json!({}));
    stop.actor = AgentExecutionActor::User { user_id: owner.clone() };
    fixture.repository.cancel_step(owner, execution, before.execution.version, &fixture.step, before.steps[0].version,
        nomifun_common::AgentExecutionCancellationScope::Task, &stop).await.unwrap();
    let facts = fixture.repository.read_task_retirement_facts(owner, fixture.session.as_ref(), execution, &fixture.step, &fixture.attempt).await.unwrap().unwrap();
    let current = facts.detail.current_attempt.as_ref().unwrap();
    assert_eq!(facts.original_attempt.attempt.retry_after, Some(backoff));
    assert_eq!(current.attempt.attempt_id, retry.attempt);
    assert_eq!(current.attempt.status, "cancelled");
    assert!(current.attempt.started_at.is_none() && current.conversation_id.is_none(), "queued Stop cannot manufacture an invocation");
    assert!(fixture.repository.list_pending_conversation_cleanups(Some(execution), None, 100).await.unwrap().cleanups
        .iter().any(|cleanup| cleanup.link_id == old.link_id && cleanup.target == old.target));
    assert!(fixture.retire().await); fixture.assert_retired().await;
    assert!(Arc::ptr_eq(&independent_workspace, &fixture.resources.get_for_agent_session(owner, independent.as_ref()).await.unwrap().unwrap()));
    assert_eq!(independent_workspace.snapshot().await.unwrap().runtime.unwrap().tabs.len(), 1);
    assert_eq!(fixture.counts.creates.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap(), cursor);
    fixture.close().await;
}

#[tokio::test]
async fn formal_replan_replacement_dependency_skip_and_verified_adoption_end_the_old_browser_purpose() {
    for mode in ["replan", "replacement", "dependency", "adopt"] {
        let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
        let (old, backoff) = fixture.acknowledge_backoff_after(if mode == "adopt" {
            ExecutionAttemptStatus::Completed } else { ExecutionAttemptStatus::Failed }).await;
        let cursor = fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap();
        let owner = &fixture.principal.principal_id; let execution = fixture.execution.as_ref().unwrap();
        let before = fixture.repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
        let step = before.steps.iter().find(|step| step.step_id == fixture.step).unwrap();
        if mode == "adopt" {
            assert!(matches!(fixture.owner.effect_store.turn_outcome(&fixture.session, &fixture.operation).await.unwrap(),
                Some(nomifun_agent_contracts::AgentTurnOutcome::Answered { .. })), "adoption consumes the original sealed public delivery");
            fixture.repository.adopt_step_output(owner, execution, before.execution.version, &fixture.step, step.version,
                &nomifun_db::AdoptAgentExecutionStepOutputParams { output_summary: "Original task finished.".into(), output_files: "[]".into(),
                    tokens: None, runtime_state: None }, &event(AgentExecutionEventKind::AttemptChanged, json!({"change":"output_adopted"}))).await.unwrap();
        } else if mode == "dependency" {
            fixture.repository.transition_step_status(owner, execution, &fixture.step, before.execution.version, step.version, None,
                ExecutionStepStatus::Skipped, &event(AgentExecutionEventKind::StepChanged,
                    json!({"status":"skipped","reason":"dependency_failed"}))).await.unwrap();
        } else {
            let participant = before.attempts.iter().find(|attempt| attempt.attempt.attempt_id == fixture.attempt).unwrap().attempt.participant_id.as_ref().unwrap();
            let replacement = replacement_task(participant);
            let mut changed = if mode == "replan" { event(AgentExecutionEventKind::PlanChanged, json!({"change":"replanned"})) }
                else { NewAgentExecutionEvent { step_id: Some(fixture.step.clone()), ..event(AgentExecutionEventKind::StepChanged,
                    json!({"change":"step_snapshot_replaced","command":"configured",
                        "before":{"plan_revision":before.execution.plan_revision,"title":step.title,"spec":step.spec,
                            "participant_id":step.assigned_participant_id,"assignment_source":step.assignment_source,"assignment_locked":step.assignment_locked,"preset_prompt":step.preset_prompt},
                        "after":{"plan_revision":before.execution.plan_revision+1,"title":replacement.title,"spec":replacement.spec,
                            "participant_id":replacement.assigned_participant_id,"assignment_source":replacement.assignment_source,"assignment_locked":replacement.assignment_locked,"preset_prompt":replacement.preset_prompt}})) } };
            changed.actor = AgentExecutionActor::User { user_id: owner.clone() };
            fixture.repository.reconcile_plan(owner, execution, before.execution.version, &ReconcileAgentExecutionPlanParams {
                goal: None, adaptation_policy: None, decision_policy: None, delegation_policy: None, keep_step_ids: vec![], new_participants: vec![],
                retire_participant_ids: vec![], new_steps: vec![replacement], new_dependencies: vec![], execution_status: AgentExecutionStatus::Running,
            }, &changed).await.unwrap();
            assert!(fixture.repository.get_step_detail(owner, execution, &fixture.step).await.unwrap().is_none(), "superseded cleanup never changes ordinary Step visibility");
        }
        let facts = fixture.repository.read_task_retirement_facts(owner, fixture.session.as_ref(), execution, &fixture.step, &fixture.attempt).await.unwrap().unwrap();
        assert_eq!(facts.original_attempt.attempt.status, "failed");
        assert_eq!(facts.original_attempt.attempt.retry_after, Some(backoff));
        assert!(fixture.repository.list_pending_conversation_cleanups(Some(execution), None, 100).await.unwrap().cleanups
            .iter().any(|cleanup| cleanup.link_id == old.link_id && cleanup.target == old.target));
        assert!(fixture.retire().await); fixture.assert_retired().await;
        assert_eq!(fixture.counts.creates.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap(), cursor);
        fixture.close().await;
    }
}

#[tokio::test]
async fn formal_stop_of_waiting_input_retires_the_browser_without_answering_or_rewriting_the_question() {
    let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
    let request = nomifun_agent_contracts::AgentUserInputRequest { request_id: "browser-task-question".into(),
        question: "Which report should this task produce?".into(), remaining_work: "Produce the report after the original answer.".into() };
    fixture.owner.effect_store.append_turn_terminal(&fixture.append("turn/completed", fixture.operation.as_ref(),
        json!({"user_input_request":request})), &fixture.operation).await.unwrap();
    fixture.owner.settle_turn(&fixture.principal.principal_id, fixture.session.as_ref(), fixture.operation.as_ref()).await.unwrap();
    fixture.owner.finish_turn(&fixture.principal.principal_id, fixture.session.as_ref(), fixture.operation.as_ref()).await.unwrap();
    let receipt = fixture.owner.effect_store.read_turn_receipt(&fixture.session, &fixture.operation).await.unwrap();
    let terminal = receipt.terminal_event.unwrap().event_id;
    let owner = &fixture.principal.principal_id; let execution = fixture.execution.as_ref().unwrap();
    let current = fixture.repository.get_step_detail(owner, execution, &fixture.step).await.unwrap().unwrap();
    let attempt = current.current_attempt.unwrap().attempt;
    fixture.repository.settle_attempt(owner, execution, &fixture.step, current.step.version, &fixture.attempt, attempt.version, None,
        &SettleAgentExecutionAttemptParams { expected_active_session_turn: None,
            expected_terminal_turn: Some(nomifun_db::AgentExecutionTerminalTurnGuard::InputRequired { conversation_id: fixture.session.as_ref().into(),
                canonical_operation_id: fixture.operation.as_ref().into(), terminal_event_id: terminal.as_ref().into(), user_input_request: request.clone() }),
            attempt_status: ExecutionAttemptStatus::WaitingInput, step_status: ExecutionStepStatus::WaitingInput,
            execution_status: Some(AgentExecutionStatus::WaitingInput), question: Some(Some(request.question.clone())), error: Some(None),
            output_summary: None, output_files: None, tokens: None, retry_after: Some(None), runtime_state: Some(None), started_at: None,
            finished_at: None, loop_repeat_reset: None }, &event(AgentExecutionEventKind::DecisionRequested,
                json!({"canonical_operation_id":fixture.operation,"terminal_event_id":terminal,"question":request.question,"user_input_request":request}))).await.unwrap();
    let cursor = fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap();
    assert!(!fixture.retire().await, "an unanswered task retains its original Browser purpose");
    fixture.assert_retained().await;
    let before = fixture.repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
    let mut stop = event(AgentExecutionEventKind::StepChanged, json!({}));
    stop.actor = AgentExecutionActor::User { user_id: owner.clone() };
    fixture.repository.cancel_step(owner, execution, before.execution.version, &fixture.step, before.steps[0].version,
        nomifun_common::AgentExecutionCancellationScope::Task, &stop).await.unwrap();
    assert!(fixture.retire().await); fixture.assert_retired().await;
    let after = fixture.owner.effect_store.read_turn_receipt(&fixture.session, &fixture.operation).await.unwrap();
    assert_eq!(after.status, nomifun_agent_session::TurnReceiptStatus::Completed);
    let after = after.terminal_event.unwrap(); assert_eq!(after.event_id, terminal);
    let SessionEventPayloadRef::InlineJson(payload) = after.payload else { panic!("original question payload"); };
    assert_eq!(payload.0["user_input_request"], json!(request));
    assert!(matches!(fixture.owner.effect_store.turn_outcome(&fixture.session, &fixture.operation).await.unwrap(),
        Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired { request_id }) if request_id == request.request_id.as_ref()));
    assert_eq!(fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap(), cursor);
    assert_eq!(fixture.repository.get_execution_detail(owner, execution).await.unwrap().unwrap().attempts.len(), 1);
    assert_eq!(fixture.counts.creates.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[tokio::test]
async fn formal_attempt_scope_retires_an_earlier_question_page_after_a_later_phase_without_more_browser_work() {
    for ending in ["stop", "completed", "failed"] {
        let fixture = Mission::new(Some(AgentExecutionAttemptSessionKind::ChildAttempt)).await;
        let first = nomifun_agent_contracts::AgentUserInputRequest { request_id: "first-browser-question".into(),
            question: "Which report?".into(), remaining_work: "Finish the selected report.".into() };
        let first_terminal = fixture.waiting_question(&fixture.operation, &fixture.started, &first).await;
        let continuation = fixture.continue_question().await;
        let (_, operation, started) = continuation.turn.as_ref().unwrap();
        if ending == "stop" {
            let second = nomifun_agent_contracts::AgentUserInputRequest { request_id: "second-browser-question".into(),
                question: "Which output folder?".into(), remaining_work: "Write the selected report into that folder.".into() };
            fixture.waiting_question(operation, started, &second).await;
            let owner = &fixture.principal.principal_id; let execution = fixture.execution.as_ref().unwrap();
            let before = fixture.repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
            let mut stop = event(AgentExecutionEventKind::StepChanged, json!({}));
            stop.actor = AgentExecutionActor::User { user_id: owner.clone() };
            fixture.repository.cancel_step(owner, execution, before.execution.version, &fixture.step, before.steps[0].version,
                nomifun_common::AgentExecutionCancellationScope::Task, &stop).await.unwrap();
        } else {
            fixture.finish_retry(&continuation, if ending == "completed" { ExecutionAttemptStatus::Completed } else { ExecutionAttemptStatus::Failed }).await;
        }
        let cleanup = fixture.repository.list_pending_conversation_cleanups(fixture.execution.as_deref(), None, 100).await.unwrap().cleanups.remove(0);
        assert_eq!(cleanup.target, nomifun_db::ConversationCleanupTurnTarget::Exact { operation_id: operation.as_ref().into() });
        assert_eq!(fixture.resources.observe_workspace(&fixture.principal.principal_id, fixture.session.as_ref()).await.unwrap().task_operation_id.as_deref(),
            Some(fixture.operation.as_ref()), "the later phase did not borrow or relabel the earlier native page");
        let cursor = fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap();
        assert!(fixture.owner.retire_completed_task_resources(&fixture.principal.principal_id, &fixture.session, operation).await.unwrap());
        fixture.assert_retained().await;
        assert!(!fixture.retire().await, "a late exact-Turn callback cannot broaden itself into the later phase's task authority");
        fixture.assert_retained().await;
        assert!(fixture.owner.retire_completed_attempt_resources(&fixture.principal.principal_id, &fixture.session, &fixture.attempt, operation).await.unwrap());
        fixture.assert_retired().await;
        let old = fixture.owner.effect_store.read_turn_receipt(&fixture.session, &fixture.operation).await.unwrap();
        assert_eq!(old.status, nomifun_agent_session::TurnReceiptStatus::Completed);
        assert_eq!(old.terminal_event.unwrap().event_id, first_terminal);
        assert!(matches!(fixture.owner.effect_store.turn_outcome(&fixture.session, &fixture.operation).await.unwrap(),
            Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired { request_id }) if request_id == first.request_id.as_ref()));
        assert_eq!(fixture.repository.get_execution_detail(&fixture.principal.principal_id, fixture.execution.as_ref().unwrap()).await.unwrap().unwrap().attempts.len(), 1);
        assert_eq!(fixture.owner.effect_store.current_cursor(&fixture.session).await.unwrap(), cursor);
        assert_eq!(fixture.counts.creates.load(Ordering::SeqCst), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn ended_source_handoff_retires_only_the_exact_source_purpose_and_preserves_user_pages() {
    let cron = Mission::new(None).await;
    cron.terminal(ExecutionAttemptStatus::Completed).await;
    cron.retire().await; cron.assert_retained().await;
    assert!(cron.owner.retire_ended_source_task_resources(&cron.principal.principal_id, &cron.session, &"unrelated-source-operation".into(),
        EndedBrowserTaskSource::CronNewConversation).await.unwrap());
    cron.assert_retained().await;
    assert!(cron.owner.retire_ended_source_task_resources(&cron.principal.principal_id, &cron.session, &cron.operation,
        EndedBrowserTaskSource::CronNewConversation).await.unwrap());
    cron.assert_retired().await; cron.close().await;
    let user = Mission::new(None).await;
    user.workspace().await.user_command(BrowserTabCommand::Create { url: "https://example.test/user-note".into() }).await.unwrap();
    user.terminal(ExecutionAttemptStatus::Completed).await;
    assert!(user.owner.retire_ended_source_task_resources(&user.principal.principal_id, &user.session, &user.operation,
        EndedBrowserTaskSource::CancelledRemoteSession).await.unwrap());
    user.assert_retained().await; user.close().await;
}

#[tokio::test]
async fn ended_remote_source_preserves_a_completed_question_that_is_not_a_completed_cron_mission() {
    let remote = Mission::new(None).await;
    let request = nomifun_agent_contracts::AgentUserInputRequest { request_id: "remote-browser-question".into(),
        question: "What should the source do next?".into(), remaining_work: "Continue only after the original user answer.".into() };
    let terminal = remote.canonical_question(&remote.operation, &remote.started, &request).await;
    let cursor = remote.owner.effect_store.current_cursor(&remote.session).await.unwrap();
    assert!(!remote.owner.retire_ended_source_task_resources(&remote.principal.principal_id, &remote.session, &remote.operation,
        EndedBrowserTaskSource::CronNewConversation).await.unwrap());
    remote.assert_retained().await;
    assert!(remote.owner.retire_ended_source_task_resources(&remote.principal.principal_id, &remote.session, &remote.operation,
        EndedBrowserTaskSource::CancelledRemoteSession).await.unwrap());
    remote.assert_retired().await;
    assert_eq!(remote.owner.effect_store.read_turn_receipt(&remote.session, &remote.operation).await.unwrap().terminal_event.unwrap().event_id, terminal);
    assert!(matches!(remote.owner.effect_store.turn_outcome(&remote.session, &remote.operation).await.unwrap(),
        Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired { request_id }) if request_id == request.request_id.as_ref()));
    assert_eq!(remote.owner.effect_store.current_cursor(&remote.session).await.unwrap(), cursor);
    remote.close().await;
}
