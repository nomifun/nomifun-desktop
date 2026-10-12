//! Actual ordinary owners and admission locks; no generated success or model
//! request is used to prove project file safety.
use super::*;
use super::app_completion_fence_tests::Fixture;
const SECOND_CREATION_KEY: &str = "0190f5fe-7c00-7a00-8000-000000000005";

#[tokio::test]
async fn cold_default_coding_session_creates_and_warms_its_owned_directory_without_treating_the_catalog_root_as_a_project() {
    let fixture = Fixture::new().await;
    assert!(!fixture.services.work_dir.exists(), "no fixture bootstrap may conceal the cold managed-workspace case");
    let owner = AuthenticatedOwner(UserId::from(fixture.principal.principal_id.clone()));
    let editor = fixture.state.control_plane.create_from_template(&owner.0, "coding.codex",
        CreateAgentPresetFromTemplateRequest { model: None, reuse_existing: true, display_name: "coding.codex".into(),
            description: None, model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new() }).await.unwrap();
    let binding = fixture.state.control_plane.resolve_agent_session_binding_with_model(&owner.0, editor.preset.preset_id.as_ref(), None).await.unwrap();
    let (_, _, snapshot) = fixture.state.control_plane.saved_binding_artifacts(&owner.0, &binding).await.unwrap();
    let selections = snapshot.content.required_resource_kinds.iter().map(|kind| {
        let resource_id = match kind.as_ref() {
            "workspace" => super::super::nomi_core_resource_bindings::DEFAULT_WORKSPACE_RESOURCE_ID,
            "process_session" => super::super::nomi_core_resource_bindings::MANAGED_PROCESS_SESSION_RESOURCE_ID,
            "project_memory" => super::super::nomi_core_resource_bindings::DEFAULT_PROJECT_MEMORY_RESOURCE_ID,
            other => panic!("unexpected cold Coding resource {other}"),
        };
        AgentResourceSelectionDto { resource_kind: kind.as_ref().into(), resource_id: resource_id.into() }
    }).collect();
    let Json(opened) = create_nomi_core_agent_session(State(fixture.state.clone()), Extension(owner.clone()), HeaderMap::new(),
        Json(CreateAgentSessionRequestDto { preset_id: editor.preset.preset_id, title: Some("Default coding Session".into()),
            model: None, reasoning_effort: None, resource_selections: selections, knowledge_policy: None,
            workspace: None, session_capabilities: Some(Default::default()), required_modules: BTreeSet::new() })).await.unwrap();
    let opened = opened.data.unwrap();
    let session = parse_agent_session_id(&opened.agent_session_id).unwrap();
    let projection = fixture.state.session_owner.canonical_conversation_projection(&fixture.principal.principal_id, &session).await.unwrap().unwrap();
    assert_eq!(projection.extra["workspace"].as_str(), fixture.services.work_dir.join("conversations").join(session.as_ref()).to_str());
    assert!(fixture.services.work_dir.join("conversations").join(session.as_ref()).is_dir());
    let _ = warm_nomi_core_agent_session(State(fixture.state.clone()), Extension(owner), Path(session.as_ref().to_owned())).await.unwrap();
    assert!(fixture.services.agent_runtime_sessions.has_owned_runtime(session.as_ref()));
    fixture.finish().await;
}

async fn second_session(fixture: &Fixture) -> AgentSessionId {
    let conversation = resolve_app_development_session_selection(&fixture.state, &fixture.principal.principal_id,
        &fixture.app, None, Some(SECOND_CREATION_KEY), None, None).await.unwrap();
    AgentSessionId::from(conversation.conversation_id)
}

#[tokio::test]
async fn project_fence_blocks_new_sessions_forks_warmup_and_turn_admission_before_mutation() {
    let fixture = Fixture::new().await;
    let store = fixture.state.session_owner.canonical().store();
    let before = store.head(&fixture.session).await.unwrap();
    let count = store.list_live_sessions_for_workspace(&fixture.principal, None, 1000,
        fixture.app.directory.to_str().unwrap()).await.unwrap().total;
    let held = guard_app_development_workspace_change(&fixture.state, &fixture.principal.principal_id, &fixture.app).await.unwrap();
    assert!(held.is_some());
    let new = second_session_attempt(&fixture).await;
    assert!(new.is_err(), "a newly opened same-root Session cannot cross the physical file operation fence");
    let owner = AuthenticatedOwner(UserId::from(fixture.principal.principal_id.clone()));
    let binding = store.get_live_session(&fixture.session).await.unwrap().agent_binding;
    let fork = fork_nomi_core_agent_session(State(fixture.state.clone()), Extension(owner.clone()),
        Path(fixture.session.as_ref().to_owned()), HeaderMap::new(), Json(ForkAgentSessionRequestDto {
            target_agent_binding: agent_binding_dto(&binding).unwrap(), parent_through_seq: 0, title: None,
        })).await;
    assert!(fork.is_err(), "the ordinary fork route must check the root fence before canonical child creation");
    let warm = warm_nomi_core_agent_session(State(fixture.state.clone()), Extension(owner), Path(fixture.session.as_ref().to_owned())).await;
    assert!(warm.is_err());
    let turn = fixture.state.session_owner.dispatch_canonical_turn(&fixture.principal.principal_id, &fixture.session,
        "must-not-admit-during-overwrite", serde_json::from_value(json!({"content":"modify the project"})).unwrap(), false, None).await;
    assert!(turn.is_err());
    assert_eq!(store.head(&fixture.session).await.unwrap(), before);
    assert_eq!(store.list_live_sessions_for_workspace(&fixture.principal, None, 1000,
        fixture.app.directory.to_str().unwrap()).await.unwrap().total, count);
    assert!(!fixture.services.agent_runtime_sessions.has_owned_runtime(fixture.session.as_ref()));
    drop(held);
    let _ = second_session(&fixture).await;
    fixture.finish().await;
}

async fn second_session_attempt(fixture: &Fixture) -> Result<ConversationResponse, NomiCoreApiError> {
    resolve_app_development_session_selection(&fixture.state, &fixture.principal.principal_id,
        &fixture.app, None, Some(SECOND_CREATION_KEY), None, None).await
}

#[tokio::test]
async fn noncurrent_project_session_turn_blocks_changes_and_exact_completed_turn_is_not_navigation_bound() {
    let fixture = Fixture::new().await;
    let second = second_session(&fixture).await;
    let (source, _, _) = fixture.turn("completed-original-session", true).await;
    let mut app = fixture.app.clone();
    app.development_session_id = Some(second.as_ref().to_owned());
    let completed = guard_app_development_completed_config(&fixture.state, &fixture.principal.principal_id, &app,
        fixture.session.as_ref(), &source).await.unwrap();
    assert!(!completed.generated_files_changed);
    drop(completed);
    let (_, _, _) = fixture.turn("original-session-still-active", false).await;
    let error = guard_app_development_workspace_change(&fixture.state, &fixture.principal.principal_id, &app).await.unwrap_err();
    assert_eq!(error.code, "APP_DEVELOPMENT_BUSY");
    assert_eq!(app.development_session_id.as_deref(), Some(second.as_ref()));
    fixture.finish().await;
}

#[tokio::test]
async fn closed_unknown_effect_on_a_noncurrent_project_session_preserves_the_project_fence() {
    use nomifun_agent_contracts::{SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef};
    use nomifun_agent_session::{EffectEventRequest, EffectStrategy, EffectTerminalState};
    let fixture = Fixture::new().await;
    let second = second_session(&fixture).await;
    let (_, operation, started) = fixture.turn("unconfirmed-project-change", false).await;
    let store = fixture.state.session_owner.canonical().store();
    let tool = store.append_event(&SessionEventAppend {
        agent_session_id: fixture.session.clone(), event_id: "project-unknown-tool".into(), producer_id: "runtime_supervisor".into(),
        idempotency_key: "project-unknown-tool".into(), semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("tool/call-started".into()), kind_version: 1, correlation_id: "project-unknown-operation".into(),
            causation_event_id: Some(started.clone()), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "operation_id":"project-unknown-operation","call_id":"project-unknown-call","capability_id":"app.hosting","action_id":"app.hosting/import"}))),
        },
    }).await.unwrap().ack.unwrap().event_id;
    let effect = EffectEventRequest { agent_session_id: fixture.session.clone(), effect_id: "project-unknown-effect".into(),
        turn_id: operation.clone(), operation_id: "project-unknown-operation".into(), owner_domain: "app_hosting".into(),
        capability_module: "app.hosting".into(), action_id: "app.hosting/import".into(), resource_binding_id: None,
        resource_key: Some(format!("app.hosting:{}", fixture.app.id)), input_digest: "8".repeat(64).into(), recorded_at: 2000,
        event_id: "project-unknown-started".into(), producer_id: "capability_host".into(), idempotency_key: "project-unknown-started".into(),
        correlation_id: "project-unknown-effect".into(), strategy: EffectStrategy::ExternalUncertainEffect, causation_event_id: Some(tool),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"app_id":fixture.app.id}))),
    };
    let started_effect = store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
    store.record_effect_terminal(EffectEventRequest { event_id: "project-unknown-terminal".into(), recorded_at: 2010,
        producer_id: "application_owner".into(), causation_event_id: Some(started_effect), ..effect }, EffectTerminalState::Uncertain).await.unwrap();
    fixture.terminal(&operation, &started, "turn/failed").await;
    let before = serde_json::to_value(store.observe(&fixture.session, None, 1000).await.unwrap()).unwrap();
    let mut app = fixture.app.clone();
    app.development_session_id = Some(second.as_ref().to_owned());
    let error = guard_app_development_workspace_change(&fixture.state, &fixture.principal.principal_id, &app).await.unwrap_err();
    assert_eq!(error.code, "APP_DEVELOPMENT_BUSY");
    assert!(store.head(&fixture.session).await.unwrap().active_turn_id.is_none());
    assert_eq!(serde_json::to_value(store.observe(&fixture.session, None, 1000).await.unwrap()).unwrap(), before);
    assert!(fixture.app.directory.exists());
    fixture.finish().await;
}

#[tokio::test]
async fn idle_formal_app_project_sessions_release_caches_without_changing_any_canonical_fact() {
    let fixture = Fixture::new().await;
    let second = second_session(&fixture).await;
    let owner = AuthenticatedOwner(UserId::from(fixture.principal.principal_id.clone()));
    for id in [&fixture.session, &second] {
        let _ = warm_nomi_core_agent_session(State(fixture.state.clone()), Extension(owner.clone()), Path(id.as_ref().to_owned())).await.unwrap();
        assert!(fixture.services.agent_runtime_sessions.has_owned_runtime(id.as_ref()));
    }
    let store = fixture.state.session_owner.canonical().store();
    let original = store.observe(&fixture.session, None, 1000).await.unwrap();
    let newer = store.observe(&second, None, 1000).await.unwrap();
    let held = guard_app_development_workspace_change(&fixture.state, &fixture.principal.principal_id, &fixture.app).await.unwrap();
    release_app_development_workspace_runtime(&fixture.state, &fixture.app).await.unwrap();
    assert!(!fixture.services.agent_runtime_sessions.has_owned_runtime(fixture.session.as_ref()));
    assert!(!fixture.services.agent_runtime_sessions.has_owned_runtime(second.as_ref()));
    assert_eq!(serde_json::to_value(store.observe(&fixture.session, None, 1000).await.unwrap()).unwrap(), serde_json::to_value(original).unwrap());
    assert_eq!(serde_json::to_value(store.observe(&second, None, 1000).await.unwrap()).unwrap(), serde_json::to_value(newer).unwrap());
    assert!(fixture.app.directory.exists());
    drop(held);
    fixture.finish().await;
}

struct ImportInvocation {
    turn: OperationId,
    operation: OperationId,
    receipt: nomifun_agent_session::EffectEventRequest,
    snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
}

impl ImportInvocation {
    fn authority<'a>(&'a self, fixture: &'a Fixture) -> app_project_fences::AppImportAuthority<'a> {
        app_project_fences::AppImportAuthority { principal: &fixture.principal, session: &fixture.session,
            turn: &self.turn, operation: &self.operation, snapshot: &self.snapshot }
    }
}

async fn import_invocation(fixture: &Fixture, target: &nomifun_app_hosting::AppRecord) -> ImportInvocation {
    use nomifun_agent_contracts::{SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef};
    let (_, turn, started) = fixture.turn("exact-project-import", false).await;
    let operation = OperationId::from("exact-project-import-tool");
    let store = fixture.state.session_owner.canonical().store();
    let call = store.append_event(&SessionEventAppend { agent_session_id: fixture.session.clone(),
        event_id: "exact-project-import-tool-event".into(), producer_id: "runtime_supervisor".into(),
        idempotency_key: "exact-project-import-tool-event".into(), semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("tool/call-started".into()), kind_version: 1, correlation_id: operation.as_ref().into(),
            causation_event_id: Some(started), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "operation_id":operation,"call_id":"exact-project-import-call","capability_id":"app.hosting","action_id":"app.hosting/import"}))),
        } }).await.unwrap().ack.unwrap().event_id;
    let resource = super::super::hosted_effect_receipts::application_resource_key(&fixture.principal.principal_id,
        "app.hosting/import", &json!({"app_id":target.id})).await.unwrap();
    let receipt = nomifun_agent_session::EffectEventRequest { agent_session_id: fixture.session.clone(),
        effect_id: "exact-project-import-effect".into(), turn_id: turn.clone(), operation_id: operation.clone(),
        owner_domain: "app_hosting".into(), capability_module: "app.hosting".into(), action_id: "app.hosting/import".into(),
        resource_binding_id: None, resource_key: Some(resource), input_digest: "a".repeat(64).into(), recorded_at: 2000,
        event_id: "exact-project-import-effect-event".into(), producer_id: "capability_host".into(),
        idempotency_key: "exact-project-import-effect".into(), correlation_id: "exact-project-import-effect".into(),
        strategy: nomifun_agent_session::EffectStrategy::ManagedEffect, causation_event_id: Some(call),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"app_id":target.id}))),
    };
    store.record_effect_started(receipt.clone()).await.unwrap();
    let snapshot = store.get_live_session(&fixture.session).await.unwrap().agent_binding.resolved_snapshot_ref;
    ImportInvocation { turn, operation, receipt, snapshot }
}

#[tokio::test]
async fn agent_import_accepts_only_its_exact_effect_and_holds_the_existing_native_mutation_lock() {
    let fixture = Fixture::new().await;
    let invocation = import_invocation(&fixture, &fixture.app).await;
    let native = super::super::nomi_core_wave2::action_host_port(&fixture.services, fixture.state.session_owner.canonical().store().clone());
    std::fs::write(fixture.app.directory.join("index.html"), "ORIGINAL_PROJECT").unwrap();
    let held = app_project_fences::guard_agent_import_exact(&fixture.state.session_owner, &native,
        invocation.authority(&fixture), &invocation.receipt, &fixture.app).await.unwrap();
    assert!(native.try_guard_workspace_mutation(&fixture.app.directory).is_err(),
        "an own-Turn parallel file mutation must use the same physical lock as application import");
    let other = fixture.services.app_hosting.create("Independent project".into(), Default::default()).await.unwrap();
    let other_guard = native.try_guard_workspace_mutation(&other.directory).unwrap();
    drop(other_guard);
    let source = fixture.services.data_dir.join("external-import-source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("index.html"), "IMPORTED_PROJECT").unwrap();
    let app = fixture.services.app_hosting.import_into(&fixture.app.id, &source, None, nomifun_app_hosting::ImportKind::Artifact).await.unwrap();
    assert_eq!(app.id, fixture.app.id);
    assert_eq!(std::fs::read_to_string(fixture.app.directory.join("index.html")).unwrap(), "IMPORTED_PROJECT");
    assert_eq!(std::fs::read_to_string(source.join("index.html")).unwrap(), "IMPORTED_PROJECT");
    drop(held);
    let released = native.try_guard_workspace_mutation(&fixture.app.directory).unwrap();
    drop(released);
    drop(native);
    fixture.finish().await;
}

#[tokio::test]
async fn agent_import_rejects_another_project_session_active_turn_and_does_not_ignore_own_pending_or_unknown_effects() {
    use nomifun_agent_contracts::{SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef};
    use nomifun_agent_session::{EffectEventRequest, EffectTerminalState};
    for case in [0, 1, 2] {
        let fixture = Fixture::new().await;
        let other = second_session(&fixture).await;
        let invocation = import_invocation(&fixture, &fixture.app).await;
        let store = fixture.state.session_owner.canonical().store();
        if case != 0 {
            let unknown = case == 2;
            let capability = if unknown { "workspace.vcs" } else { "app.hosting" };
            let action = if unknown { "workspace.vcs/push" } else { "app.hosting/import" };
            let started = store.read_turn_receipt(&fixture.session, &invocation.turn).await.unwrap().started_event.unwrap().event_id;
            let tool = store.append_event(&SessionEventAppend { agent_session_id: fixture.session.clone(),
                event_id: "other-unconfirmed-tool-event".into(), producer_id: "runtime_supervisor".into(),
                idempotency_key: "other-unconfirmed-tool-event".into(), semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("tool/call-started".into()), kind_version: 1, correlation_id: "other-unconfirmed-operation".into(),
                    causation_event_id: Some(started), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "operation_id":"other-unconfirmed-operation","call_id":"other-unconfirmed-call","capability_id":capability,"action_id":action}))),
                } }).await.unwrap().ack.unwrap().event_id;
            let workspace_binding = store.session_resources(&fixture.session).await.unwrap().into_iter().find(|binding| binding.resource_kind.as_ref() == "workspace").unwrap();
            let effect = EffectEventRequest { effect_id: "other-unconfirmed-effect".into(), operation_id: "other-unconfirmed-operation".into(),
                resource_key: Some("a-different-application-resource".into()), event_id: "other-unconfirmed-start".into(),
                idempotency_key: "other-unconfirmed-start".into(), correlation_id: "other-unconfirmed-effect".into(),
                owner_domain: if unknown { "workspace".into() } else { "app_hosting".into() },
                capability_module: capability.into(), action_id: action.into(),
                resource_binding_id: unknown.then_some(workspace_binding.binding_id),
                strategy: if unknown { nomifun_agent_session::EffectStrategy::ExternalUncertainEffect } else { nomifun_agent_session::EffectStrategy::ManagedEffect },
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(if unknown { json!({"remote":"origin"}) } else { json!({"app_id":fixture.app.id}) })),
                causation_event_id: Some(tool), ..invocation.receipt.clone() };
            let effect_started = store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
            if case == 2 { store.record_effect_terminal(EffectEventRequest { event_id: "other-unconfirmed-outcome".into(),
                producer_id: "application_owner".into(), causation_event_id: Some(effect_started), recorded_at: 2010, ..effect }, EffectTerminalState::Uncertain).await.unwrap();
            }
        } else {
            let operation = NomiCoreSessionOwner::turn_operation_id(&fixture.principal.principal_id, other.as_ref(), "other-session-development");
            let input = fixture.state.session_owner.canonical_turn_input_with_admission(&fixture.principal.principal_id, &other, &operation,
                &serde_json::from_value(json!({"content":"Edit this shared application project"})).unwrap()).await.unwrap();
            fixture.state.session_owner.canonical().start_turn(&fixture.principal, &other, "other-session-development", input).await.unwrap();
        }
        let before = serde_json::to_value(store.observe(&fixture.session, None, 1000).await.unwrap()).unwrap();
        let native = super::super::nomi_core_wave2::action_host_port(&fixture.services, store.clone());
        std::fs::write(fixture.app.directory.join("user-work.txt"), "PRESERVE_SOURCE").unwrap();
        let guarded = app_project_fences::guard_agent_import_exact(&fixture.state.session_owner, &native,
            invocation.authority(&fixture), &invocation.receipt, &fixture.app).await;
        assert!(guarded.is_err());
        assert_eq!(std::fs::read_to_string(fixture.app.directory.join("user-work.txt")).unwrap(), "PRESERVE_SOURCE");
        assert_eq!(serde_json::to_value(store.observe(&fixture.session, None, 1000).await.unwrap()).unwrap(), before);
        drop(native);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn agent_import_does_not_accept_a_forged_turn_operation_or_target_receipt() {
    let fixture = Fixture::new().await;
    let invocation = import_invocation(&fixture, &fixture.app).await;
    let native = super::super::nomi_core_wave2::action_host_port(&fixture.services, fixture.state.session_owner.canonical().store().clone());
    let mut forged = invocation.receipt.clone();
    forged.operation_id = "different-operation".into();
    assert!(app_project_fences::guard_agent_import_exact(&fixture.state.session_owner, &native,
        invocation.authority(&fixture), &forged, &fixture.app).await.is_err());
    let other = fixture.services.app_hosting.create("Different target".into(), Default::default()).await.unwrap();
    assert!(app_project_fences::guard_agent_import_exact(&fixture.state.session_owner, &native,
        invocation.authority(&fixture), &invocation.receipt, &other).await.is_err());
    let mut forged = invocation.receipt.clone();
    forged.causation_event_id = Some("a-different-tool-call".into());
    assert!(app_project_fences::guard_agent_import_exact(&fixture.state.session_owner, &native,
        invocation.authority(&fixture), &forged, &fixture.app).await.is_err());
    assert_eq!(fixture.state.session_owner.canonical().store().list_effects(&fixture.session).await.unwrap().len(), 1);
    drop(native);
    fixture.finish().await;
}

#[tokio::test]
async fn agent_import_blocks_closed_pending_and_unknown_effects_in_another_project_session() {
    use nomifun_agent_contracts::{SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef};
    use nomifun_agent_session::{EffectEventRequest, EffectTerminalState};
    for unknown in [false, true] {
        let fixture = Fixture::new().await;
        let other = second_session(&fixture).await;
        let invocation = import_invocation(&fixture, &fixture.app).await;
        let store = fixture.state.session_owner.canonical().store();
        let operation = NomiCoreSessionOwner::turn_operation_id(&fixture.principal.principal_id, other.as_ref(), "other-project-file-effect");
        let input = fixture.state.session_owner.canonical_turn_input_with_admission(&fixture.principal.principal_id, &other, &operation,
            &serde_json::from_value(json!({"content":"Modify the application project files"})).unwrap()).await.unwrap();
        fixture.state.session_owner.canonical().start_turn(&fixture.principal, &other, "other-project-file-effect", input).await.unwrap();
        let started = store.read_turn_receipt(&other, &operation).await.unwrap().started_event.unwrap().event_id;
        let capability = if unknown { "workspace.vcs" } else { "workspace.files" };
        let action = if unknown { "workspace.vcs/push" } else { "workspace.files/write" };
        let tool = store.append_event(&SessionEventAppend { agent_session_id: other.clone(), event_id: "foreign-file-call".into(),
            producer_id: "runtime_supervisor".into(), idempotency_key: "foreign-file-call".into(), semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("tool/call-started".into()), kind_version: 1, correlation_id: "foreign-file-operation".into(),
                causation_event_id: Some(started.clone()), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "operation_id":"foreign-file-operation","call_id":"foreign-file-call","capability_id":capability,"action_id":action}))),
            } }).await.unwrap().ack.unwrap().event_id;
        let binding = store.session_resources(&other).await.unwrap().into_iter().find(|binding| binding.resource_kind.as_ref() == "workspace").unwrap();
        let effect = EffectEventRequest { agent_session_id: other.clone(), effect_id: "foreign-file-effect".into(), turn_id: operation.clone(),
            operation_id: "foreign-file-operation".into(), owner_domain: "workspace".into(), capability_module: capability.into(),
            action_id: action.into(), resource_binding_id: Some(binding.binding_id), resource_key: Some("foreign-project-file-resource".into()),
            input_digest: "b".repeat(64).into(), recorded_at: 2000, event_id: "foreign-file-started".into(), producer_id: "capability_host".into(),
            idempotency_key: "foreign-file-started".into(), correlation_id: "foreign-file-effect".into(),
            strategy: if unknown { nomifun_agent_session::EffectStrategy::ExternalUncertainEffect } else { nomifun_agent_session::EffectStrategy::ManagedEffect },
            causation_event_id: Some(tool), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(if unknown { json!({"remote":"origin"}) } else { json!({"path":"user-work.txt"}) })),
        };
        let effect_started = store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
        if unknown { store.record_effect_terminal(EffectEventRequest { event_id: "foreign-file-unknown".into(), producer_id: "workspace_owner".into(),
            causation_event_id: Some(effect_started), recorded_at: 2010, ..effect }, EffectTerminalState::Uncertain).await.unwrap(); }
        store.append_event(&SessionEventAppend { agent_session_id: other.clone(), event_id: "foreign-file-turn-failed".into(),
            producer_id: "runtime_supervisor".into(), idempotency_key: "foreign-file-turn-failed".into(), semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("turn/failed".into()), kind_version: 1, correlation_id: operation.as_ref().into(), causation_event_id: Some(started),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"error":"The exact file outcome is unconfirmed"}))),
            } }).await.unwrap();
        assert!(store.head(&other).await.unwrap().active_turn_id.is_none());
        let before = serde_json::to_value(store.observe(&other, None, 1000).await.unwrap()).unwrap();
        std::fs::write(fixture.app.directory.join("index.html"), "PRESERVE_PROJECT_BYTES").unwrap();
        let native = super::super::nomi_core_wave2::action_host_port(&fixture.services, store.clone());
        assert!(app_project_fences::guard_agent_import_exact(&fixture.state.session_owner, &native,
            invocation.authority(&fixture), &invocation.receipt, &fixture.app).await.is_err());
        assert_eq!(std::fs::read_to_string(fixture.app.directory.join("index.html")).unwrap(), "PRESERVE_PROJECT_BYTES");
        assert_eq!(serde_json::to_value(store.observe(&other, None, 1000).await.unwrap()).unwrap(), before);
        drop(native);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn deleting_project_sessions_keep_unknown_outcomes_protected_without_blocking_another_owner_or_directory() {
    use nomifun_agent_contracts::{SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef};
    use nomifun_agent_session::{EffectEventRequest, EffectTerminalState};
    let fixture = Fixture::new().await;
    let other_app = fixture.services.app_hosting.create("Another project".into(), Default::default()).await.unwrap();
    let other_conversation = resolve_app_development_session_selection(&fixture.state, &fixture.principal.principal_id,
        &other_app, None, None, None, None).await.unwrap();
    let other_directory_session = parse_agent_session_id(&other_conversation.conversation_id).unwrap();
    fixture.state.session_owner.canonical().fence_delete(&fixture.principal, &other_directory_session,
        "delete-independent-project-session", now_ms()).await.unwrap();
    let unrelated = guard_app_development_workspace_change(&fixture.state, &fixture.principal.principal_id, &fixture.app).await.unwrap();
    drop(unrelated);
    let other = second_session(&fixture).await;
    let store = fixture.state.session_owner.canonical().store();
    let operation = NomiCoreSessionOwner::turn_operation_id(&fixture.principal.principal_id, other.as_ref(), "unconfirmed-vcs-before-delete");
    let input = fixture.state.session_owner.canonical_turn_input_with_admission(&fixture.principal.principal_id, &other, &operation,
        &serde_json::from_value(json!({"content":"An external VCS outcome remains unconfirmed"})).unwrap()).await.unwrap();
    fixture.state.session_owner.canonical().start_turn(&fixture.principal, &other, "unconfirmed-vcs-before-delete", input).await.unwrap();
    let started = store.read_turn_receipt(&other, &operation).await.unwrap().started_event.unwrap().event_id;
    let tool = store.append_event(&SessionEventAppend { agent_session_id: other.clone(), event_id: "deleting-vcs-call".into(),
        producer_id: "runtime_supervisor".into(), idempotency_key: "deleting-vcs-call".into(), semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("tool/call-started".into()), kind_version: 1, correlation_id: "deleting-vcs-operation".into(),
            causation_event_id: Some(started.clone()), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "operation_id":"deleting-vcs-operation","call_id":"deleting-vcs-call","capability_id":"workspace.vcs","action_id":"workspace.vcs/push"}))),
        } }).await.unwrap().ack.unwrap().event_id;
    let resource = store.session_resources(&other).await.unwrap().into_iter().find(|binding| binding.resource_kind.as_ref() == "workspace").unwrap();
    let effect = EffectEventRequest { agent_session_id: other.clone(), effect_id: "deleting-vcs-effect".into(), turn_id: operation.clone(),
        operation_id: "deleting-vcs-operation".into(), owner_domain: "workspace".into(), capability_module: "workspace.vcs".into(),
        action_id: "workspace.vcs/push".into(), resource_binding_id: Some(resource.binding_id), resource_key: Some("deleting-project-vcs-resource".into()),
        input_digest: "c".repeat(64).into(), recorded_at: 2000, event_id: "deleting-vcs-started".into(), producer_id: "capability_host".into(),
        idempotency_key: "deleting-vcs-started".into(), correlation_id: "deleting-vcs-effect".into(), strategy: nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
        causation_event_id: Some(tool), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"remote":"origin"}))),
    };
    let effect_started = store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
    store.record_effect_terminal(EffectEventRequest { event_id: "deleting-vcs-uncertain".into(), producer_id: "workspace_owner".into(),
        causation_event_id: Some(effect_started), recorded_at: 2010, ..effect }, EffectTerminalState::Uncertain).await.unwrap();
    store.append_event(&SessionEventAppend { agent_session_id: other.clone(), event_id: "deleting-vcs-turn-failed".into(),
        producer_id: "runtime_supervisor".into(), idempotency_key: "deleting-vcs-turn-failed".into(), semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("turn/failed".into()), kind_version: 1, correlation_id: operation.as_ref().into(), causation_event_id: Some(started),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"error":"External outcome is unconfirmed"}))),
        } }).await.unwrap();
    fixture.state.session_owner.canonical().fence_delete(&fixture.principal, &other, "delete-unconfirmed-project-session", now_ms()).await.unwrap();
    let before = serde_json::to_value(store.get_deleting_session(&other).await.unwrap()).unwrap();
    let blockers = serde_json::to_value(store.delete_blockers(&other).await.unwrap()).unwrap();
    let foreign = PrincipalRef { principal_kind: "user".into(), principal_id: Uuid::now_v7().to_string() };
    assert!(app_project_fences::project_sessions(store, &foreign, fixture.app.directory.to_str().unwrap()).await.unwrap().is_empty());
    std::fs::write(fixture.app.directory.join("index.html"), "PRESERVE_DELETING_PROJECT").unwrap();
    assert!(guard_app_development_workspace_change(&fixture.state, &fixture.principal.principal_id, &fixture.app).await.is_err());
    let invocation = import_invocation(&fixture, &fixture.app).await;
    let native = super::super::nomi_core_wave2::action_host_port(&fixture.services, store.clone());
    assert!(app_project_fences::guard_agent_import_exact(&fixture.state.session_owner, &native,
        invocation.authority(&fixture), &invocation.receipt, &fixture.app).await.is_err());
    assert_eq!(std::fs::read_to_string(fixture.app.directory.join("index.html")).unwrap(), "PRESERVE_DELETING_PROJECT");
    assert_eq!(serde_json::to_value(store.get_deleting_session(&other).await.unwrap()).unwrap(), before);
    assert_eq!(serde_json::to_value(store.delete_blockers(&other).await.unwrap()).unwrap(), blockers);
    drop(native);
    fixture.finish().await;
}
