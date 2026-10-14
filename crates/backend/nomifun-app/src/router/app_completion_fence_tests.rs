//! Admission/terminal race tests; no provider request or generated success.
use super::*;
use nomifun_agent_contracts::{SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef};
use nomifun_agent_session::{EffectEventRequest, EffectStrategy, EffectTerminalState};

pub(super) struct Fixture {
    _root: tempfile::TempDir,
    pub(super) services: crate::services::AppServices,
    pub(super) state: NomiCoreAgentApiState,
    pub(super) app: nomifun_app_hosting::AppRecord,
    pub(super) session: AgentSessionId,
    pub(super) principal: PrincipalRef,
}

impl Fixture {
    pub(super) async fn new() -> Self {
        use nomifun_db::{CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability, SqliteProviderRepository};
        let root = tempfile::tempdir().unwrap();
        let database = nomifun_db::init_database_memory().await.unwrap();
        let services = crate::services::AppServices::from_config(database, &crate::AppConfig {
            data_dir: root.path().join("data"), work_dir: root.path().join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some("completion-race-fixture".into()), ..Default::default()
        }).await.unwrap();
        let provider = Uuid::now_v7().to_string();
        let encrypted = nomifun_common::encrypt_string(r#"{"api_keys":["no-model-call-fixture"]}"#, &services.encryption_key).unwrap();
        SqliteProviderRepository::new(services.database.pool().clone()).create(CreateProviderParams {
            provider_id: Some(&provider), platform: "openai", name: "Completion fence fixture",
            base_url: "https://api.openai.invalid/v1", auth_scheme: "bearer", credentials_encrypted: &encrypted,
            enabled: true, bedrock_config: None, sort_order: Some(-10_000),
        }, &NewProviderModel { model: "completion-fence-fixture", enabled: true, sort_order: -10_000,
            description: Some("No model requests are made."), capabilities: &[NewProviderModelCapability {
                task: "chat", traits: r#"["function_calling"]"#, protocol: "openai.responses", connection_role: "default",
                endpoint: Some("/responses"), provider_params: "{}", context_limit: Some(128_000), ..Default::default()
            }] }, &[]).await.unwrap();
        let (states, _components) = super::super::state::try_build_module_states(&services).await.unwrap();
        let state = states.nomi_core_agent_api;
        let app = services.app_hosting.create("Protected application".into(), Default::default()).await.unwrap();
        let owner = services.authoritative_user_id.as_ref();
        let conversation = resolve_app_development_session_selection(&state, owner, &app, None, None,
            Some(AgentChatModelSelectionDto { provider_id: provider, model: "completion-fence-fixture".into() }), None).await.unwrap();
        let session = AgentSessionId::from(conversation.conversation_id);
        let reference = session.as_ref().to_owned();
        let app = services.app_hosting.development_session_with(&app.id, |_| async move { Ok(reference) }).await.unwrap();
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: owner.into() };
        Self { _root: root, services, state, app, session, principal }
    }

    pub(super) async fn turn(&self, key: &str, completed: bool) -> (String, OperationId, nomifun_agent_contracts::EventId) {
        let operation = NomiCoreSessionOwner::turn_operation_id(&self.principal.principal_id, self.session.as_ref(), key);
        let input = self.state.session_owner.canonical_turn_input_with_admission(&self.principal.principal_id, &self.session, &operation,
            &serde_json::from_value(json!({"content":"Inspect the current application without changing files"})).unwrap()).await.unwrap();
        self.state.session_owner.canonical().start_turn(&self.principal, &self.session, key, input).await.unwrap();
        let store = self.state.session_owner.canonical().store();
        let started = store.read_turn_receipt(&self.session, &operation).await.unwrap().started_event.unwrap();
        let SessionEventPayloadRef::InlineJson(payload) = &started.payload else { panic!("ordinary input is inline") };
        let source = payload.0["source_message_id"].as_str().unwrap().to_owned();
        if completed {
            self.terminal(&operation, &started.event_id, "turn/completed").await;
        }
        (source, operation, started.event_id)
    }

    pub(super) async fn terminal(&self, operation: &OperationId, started: &nomifun_agent_contracts::EventId, kind: &str) {
        let identity = Uuid::now_v7().to_string();
        self.state.session_owner.canonical().store().append_event(&SessionEventAppend {
            agent_session_id: self.session.clone(), event_id: identity.clone().into(), producer_id: "runtime_supervisor".into(),
            idempotency_key: identity.into(), semantic_event: SemanticSessionEventDraft { kind: SessionEventKind(kind.into()), kind_version: 1,
                correlation_id: operation.as_ref().into(), causation_event_id: Some(started.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"fixture":"admission protection only"}))) },
        }).await.unwrap();
    }

    pub(super) async fn finish(self) {
        self.services.cleanup_after_startup_failure(anyhow::anyhow!("completion fence fixture ended")).await;
    }
}

#[tokio::test]
async fn exact_completion_waits_for_the_published_terminal_admission_fence_release() {
    let fixture = Fixture::new().await;
    let (source, _, _) = fixture.turn("readonly-completion", true).await;
    let lock = fixture.state.session_owner.session_operation_lock(fixture.session.as_ref());
    let held = lock.clone().read_owned().await;
    let head = fixture.state.session_owner.canonical().store().head(&fixture.session).await.unwrap();
    let mut completion = Box::pin(guard_app_development_completed_config(&fixture.state, &fixture.principal.principal_id, &fixture.app, fixture.session.as_ref(), &source));
    assert!(tokio::time::timeout(Duration::from_millis(30), completion.as_mut()).await.is_err(), "completion waits instead of spuriously rejecting a just-published terminal");
    let manual = guard_app_development_workspace_change(&fixture.state, &fixture.principal.principal_id, &fixture.app).await;
    assert_eq!(manual.unwrap_err().code, "APP_DEVELOPMENT_BUSY", "ordinary manual changes still reject the held admission fence immediately");
    drop(held);
    let result = tokio::time::timeout(Duration::from_secs(1), completion.as_mut()).await.unwrap().unwrap();
    assert!(!result.generated_files_changed, "a readonly completed turn cannot authorize configuration discovery");
    assert!(result.fence.is_some());
    drop(result);
    assert_eq!(fixture.state.session_owner.canonical().store().head(&fixture.session).await.unwrap(), head);
    assert!(!fixture.app.directory.join(".nomifun-static-server.cjs").exists());
    drop(completion);
    fixture.finish().await;
}

#[tokio::test]
async fn a_closed_input_handoff_cannot_authorize_application_configuration_completion() {
    let fixture = Fixture::new().await;
    let (source, operation, started) = fixture.turn("input-handoff-not-app-delivery", false).await;
    let identity = Uuid::now_v7().to_string();
    fixture.state.session_owner.canonical().store().append_event(&SessionEventAppend {
        agent_session_id:fixture.session.clone(), event_id:identity.clone().into(),
        producer_id:"runtime_supervisor".into(), idempotency_key:identity.into(),
        semantic_event:SemanticSessionEventDraft { kind:SessionEventKind("turn/completed".into()),
            kind_version:1, correlation_id:operation.as_ref().into(), causation_event_id:Some(started),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "user_input_request":{"request_id":"project-name", "question":"请提供项目名称。",
                    "remaining_work":"收到名称后创建应用。"}
            }))) },
    }).await.unwrap();
    let error = match guard_app_development_completed_config(&fixture.state, &fixture.principal.principal_id,
        &fixture.app, fixture.session.as_ref(), &source).await {
        Err(error) => error,
        Ok(_) => panic!("input handoff must not authorize application completion"),
    };
    assert_eq!(error.code, "APP_DEVELOPMENT_INPUT_REQUIRED");
    let history = canonical_creative_studio_history(&fixture.state,
        &AuthenticatedOwner(nomifun_agent_contracts::UserId::from(fixture.principal.principal_id.clone())),
        &fixture.session).await.unwrap();
    let question = history.iter().find(|message| message.status == CreativeStudioAgentHistoryStatus::InputRequired)
        .expect("a pure control handoff must survive canonical Canvas history");
    assert_eq!(question.role, CreativeStudioAgentHistoryRole::Assistant);
    assert_eq!(question.user_input_request.as_ref().unwrap().request_id.as_ref(), "project-name");
    assert!(question.text.is_empty(), "the structured question is not fabricated assistant prose");
    fixture.finish().await;
}

#[tokio::test]
async fn completion_wait_rechecks_successor_active_turn_and_closed_unknown_effects() {
    for unknown in [false, true] {
        let fixture = Fixture::new().await;
        let (source, operation, started) = fixture.turn("not-settled", false).await;
        if unknown {
            let tool_id = Uuid::now_v7().to_string();
            let store = fixture.state.session_owner.canonical().store();
            let tool = store.append_event(&SessionEventAppend { agent_session_id: fixture.session.clone(), event_id: tool_id.clone().into(),
                producer_id: "runtime_supervisor".into(), idempotency_key: tool_id.into(), semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("tool/call-started".into()), kind_version: 1, correlation_id: "unconfirmed-operation".into(),
                    causation_event_id: Some(started.clone()), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "operation_id":"unconfirmed-operation","call_id":"unconfirmed-call","capability_id":"app.hosting","action_id":"app.hosting/start"}))) }
            }).await.unwrap().ack.unwrap().event_id;
            let effect = EffectEventRequest { agent_session_id: fixture.session.clone(), effect_id: "unconfirmed-effect".into(), turn_id: operation.clone(),
                operation_id: "unconfirmed-operation".into(), owner_domain: "app.hosting".into(), capability_module: "app.hosting".into(),
                action_id: "app.hosting/start".into(), resource_binding_id: None, resource_key: Some(format!("app.hosting:{}", fixture.app.id)),
                input_digest: "8".repeat(64).into(), recorded_at: 2000, event_id: "unconfirmed-started".into(), producer_id: "capability_host".into(),
                idempotency_key: "unconfirmed-started".into(), correlation_id: "unconfirmed-effect".into(), strategy: EffectStrategy::ExternalUncertainEffect,
                causation_event_id: Some(tool), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"app_id":fixture.app.id}))) };
            let effect_started = store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
            store.record_effect_terminal(EffectEventRequest { event_id: "unconfirmed-outcome".into(), recorded_at: 2010,
                producer_id: "application_owner".into(), causation_event_id: Some(effect_started), ..effect }, EffectTerminalState::Uncertain).await.unwrap();
            fixture.terminal(&operation, &started, "turn/failed").await;
            assert!(store.head(&fixture.session).await.unwrap().active_turn_id.is_none());
            assert!(store.has_unsettled_effects(&fixture.session).await.unwrap());
        }
        let held = fixture.state.session_owner.session_operation_lock(fixture.session.as_ref()).read_owned().await;
        let mut completion = Box::pin(guard_app_development_completed_config(&fixture.state, &fixture.principal.principal_id, &fixture.app, fixture.session.as_ref(), &source));
        assert!(tokio::time::timeout(Duration::from_millis(20), completion.as_mut()).await.is_err());
        drop(held);
        let error = completion.as_mut().await.err().expect("waiting for a fence cannot make active or unknown work safe");
        assert_eq!(error.code, "APP_DEVELOPMENT_BUSY");
        assert!(!fixture.app.directory.join(".nomifun-static-server.cjs").exists());
        drop(completion);
        fixture.finish().await;
    }
}
