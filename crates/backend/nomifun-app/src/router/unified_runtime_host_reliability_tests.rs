//! Cleanup faults on the host created by the installed official factory.
use super::*;
use axum::{body::Body, http::Request};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, Weak};
use std::time::Duration;
use tower::ServiceExt;

type HostMap = BTreeMap<String, Weak<ConversationRuntimeHost>>;
static HOSTS: OnceLock<Mutex<HostMap>> = OnceLock::new();

pub(super) fn capture_host(host: &Arc<ConversationRuntimeHost>) {
    let mut hosts = HOSTS.get_or_init(Default::default).lock().unwrap();
    hosts.retain(|_, host| host.strong_count() > 0);
    hosts.insert(host.options.conversation_id.clone(), Arc::downgrade(host));
}

async fn api(router: &axum::Router, path: &str, body: Value) -> Value {
    let response = router.clone().oneshot(Request::builder().method("POST").uri(path)
        .header("x-nomi-local-trust", "cleanup-fixture-local-trust")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{path}: {status}: {value}");
    value["data"].clone()
}

struct Fixture {
    _directory: tempfile::TempDir,
    _router: axum::Router,
    runtime: nomifun_ai_agent::AgentRuntimeHandle,
    services: crate::services::AppServices,
    host: Arc<ConversationRuntimeHost>,
    owner: Arc<super::super::nomi_core_session::NomiCoreSessionOwner>,
    voice_control:Arc<nomifun_agent_control_plane::AgentControlPlane>,
    voice_execution:Arc<nomifun_agent_execution::AgentExecutionEngine>,
    message: SendMessageData,
    database_path: PathBuf,
}

impl Fixture {
    async fn new(scenario: &str) -> Self {
        Self::build_delivery(scenario, vec![], vec![], None, false,None,
            #[cfg(feature = "browser-use")] None,
        ).await
    }

    async fn with_delivery(scenario: &str, files: Vec<String>, inject_skills: Vec<String>, origin: Option<String>) -> Self {
        Self::build_delivery(scenario, files, inject_skills, origin, true,None,
            #[cfg(feature = "browser-use")] None,
        ).await
    }

    async fn build_delivery(scenario: &str, files: Vec<String>, inject_skills: Vec<String>, origin: Option<String>, explicit_metadata: bool,model_base_override:Option<&str>,
        #[cfg(feature = "browser-use")] browser: Option<(Arc<dyn nomifun_browser_platform::runtime::BrowserRuntimeFactory>, String)>,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::env::var_os("NOMIFUN_RELIABILITY_EVIDENCE_DIR")
            .map(|root| PathBuf::from(root).join(scenario))
            .unwrap_or_else(|| directory.path().to_path_buf());
        let config = crate::AppConfig {
            data_dir: root.join("data"), work_dir: root.join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some("cleanup-fixture-local-trust".into()),
            ..Default::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        let database_path = config.database_path();
        let database = nomifun_db::init_database(&database_path).await.unwrap();
        let services = crate::services::AppServices::from_config(database, &config).await.unwrap();
        #[cfg(feature = "browser-use")]
        let mut services = services;
        #[cfg(feature = "browser-use")]
        if let Some((factory, _)) = &browser {
            services.browser_resources = Some(Arc::new(nomifun_browser_platform::workspace::BrowserResourceService::new(factory.clone())
                .with_profile_store(nomifun_browser_platform::runtime::BrowserProfileStore::new(config.data_dir.clone()).unwrap())));
        }
        let (states, _components) = super::super::state::try_build_module_states(&services).await.unwrap();
        let owner = states.nomi_core_agent_api.session_owner.clone();
        let voice_control=states.nomi_core_agent_api.control_plane.clone();let voice_execution=states.agent_execution.clone();
        let router = super::super::create_router_with_states(&services, states);
        #[cfg(feature = "browser-use")]
        let model_base = browser.as_ref().map(|(_, url)| url.as_str()).unwrap_or("http://127.0.0.1:9/v1");
        #[cfg(not(feature = "browser-use"))]
        let model_base = "http://127.0.0.1:9/v1";
        let model_base=model_base_override.unwrap_or(model_base);
        let provider = api(&router, "/api/providers", json!({
            "platform":"custom", "name":"cleanup fixture", "base_url":model_base,
            "auth_scheme":"bearer", "credentials":{"api_keys":["local-fixture-not-a-secret"]}, "enabled":true,
            "initial_model":{"model":"cleanup-fixture", "enabled":true, "capabilities":[{
                "task":"chat", "traits":[], "protocol":"openai.chat_text", "connection_role":"default", "output_limit":4096
            }]}
        })).await;
        let model = json!({"provider_id":provider["provider_id"], "model":"cleanup-fixture"});
        let editor = api(&router, "/api/agent-presets/from-template/chat.minimal", json!({
            "reuse_existing":false, "display_name":"Cleanup fixture", "model":model
        })).await;
        let preset = editor["preset"]["preset_id"].as_str().unwrap();
        let mut draft = editor["draft"].clone();
        draft["document"]["enabled_capabilities"] = json!([{
            "capability":{"id":"workspace.files"}, "action_allowlist":["workspace.files/read"]
        }]);
        api(&router, &format!("/api/agent-presets/{preset}/revisions"), json!({
            "expected_current_revision":draft["current_revision"], "draft":draft,
            "reason":"isolated cleanup fault regression"
        })).await;
        let session = api(&router, "/api/agent-sessions", json!({
            "preset_id":preset, "model":model,
            "resource_selections":[{"resource_kind":"workspace", "resource_id":"default-workspace"}]
        })).await;
        let id = session["agent_session_id"].as_str().unwrap();
        #[cfg(feature = "browser-use")]
        if browser.is_some() {
            use nomifun_browser_platform::runtime::{BrowserProfileStore, BrowserProfilePersistence, BrowserTabCommand, WorkspaceError};
            let key = nomifun_browser_platform::workspace::managed_workspace_key(services.authoritative_user_id.as_ref(), id).unwrap();
            let profile = BrowserProfileStore::new(config.data_dir.clone()).unwrap()
                .profile_for(&key, BrowserProfilePersistence::Persistent).unwrap();
            let user = services.browser_resources.as_ref().unwrap().ensure_user(services.authoritative_user_id.as_ref(), id, profile).await.unwrap();
            if let Err(error) = user.user_command(BrowserTabCommand::Create { url: "http://127.0.0.1/browser-fixture".into() }).await {
                assert_eq!(error, WorkspaceError::NativeCommandFailed, "the fixture navigation failure poisons its protocol before Chat starts");
            }
        }
        let projection = owner.get_session(services.authoritative_user_id.as_ref(), id).await.unwrap();
        let workspace = projection.extra["workspace"].as_str().unwrap().to_owned();
        let options = AgentRuntimeBuildOptions {
            user_id: services.authoritative_user_id.to_string(), agent_type: projection.r#type,
            workspace: workspace.clone(), model: projection.model, conversation_id: id.into(),
            delegation_policy: projection.delegation_policy, extra: projection.extra,
            conversation_created_at: Some(projection.created_at), device_mcp_servers: vec![],
            workspace_binding_lease: Some(nomifun_knowledge::WorkspaceBindingLease::acquire_unbound(
                std::path::Path::new(&workspace), id.to_owned()).unwrap()),
        };
        let runtime = services.agent_runtime_sessions.get_or_create_runtime(id, options).await.unwrap();
        let host = HOSTS.get().unwrap().lock().unwrap().remove(id).unwrap().upgrade().unwrap();
        // Accept the durable root without starting a model task. This is the
        // cancellation-before-first-driver-poll boundary, not a UI scenario.
        let mut delivery = json!({ "content":"cancel before model dispatch", "admission":{
            "route_identity":host.route, "resolved_snapshot_ref":host.snapshot_ref
        }});
        if explicit_metadata {
            delivery["files"] = json!(files);
            delivery["inject_skills"] = json!(inject_skills);
            delivery["origin"] = json!(origin);
        }
        let (input, _) = owner.canonical().store().start_turn(&id.to_owned().into(), "session_api".into(),
            "cleanup-retry-turn".into(), "cleanup-retry-turn".into(), StrictJsonValue(delivery)).await.unwrap();
        let root = input.record.unwrap().event_id;
        let message = SendMessageData { content:"cancel before model dispatch".into(), msg_id:"cleanup-wire".into(),
            source_message_id:Some(root.as_ref().into()), files, inject_skills, origin };
        println!("CLEANUP_FIXTURE scenario={scenario} session={id} operation=cleanup-retry-turn database={}", database_path.display());
        Self { _directory:directory, _router:router, runtime, services, host, owner,voice_control,voice_execution, message, database_path }
    }

    fn pool(&self) -> &nomifun_db::SqlitePool { self.services.database.pool() }

    fn registry_options(&self) -> AgentRuntimeBuildOptions {
        let mut options = self.host.options.clone();
        options.workspace_binding_lease = Some(nomifun_knowledge::WorkspaceBindingLease::acquire_unbound(
            std::path::Path::new(&options.workspace), options.conversation_id.clone()).unwrap());
        options
    }

    async fn state(&self) -> String {
        sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=?")
            .bind(&self.host.options.conversation_id).fetch_one(self.pool()).await.unwrap()
    }

    async fn finish(self) {
        self.runtime.kill_and_wait(None).await.unwrap();
        self.services.shutdown_nomi_core_host().await.unwrap();
    }

    async fn assert_settled(&self) {
        assert_eq!(self.state().await, "cancelled");
        let events: Vec<String> = sqlx::query_scalar("SELECT kind FROM agent_events WHERE session_id=? AND kind IN ('turn/cancelled','turn/completed','turn/failed') ORDER BY seq")
            .bind(&self.host.options.conversation_id).fetch_all(self.pool()).await.unwrap();
        assert_eq!(events, vec!["turn/cancelled"]);
        let effects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=?")
            .bind(&self.host.options.conversation_id).fetch_one(self.pool()).await.unwrap();
        assert_eq!(effects, 0, "cleanup must not dispatch a model or effect");
        let models: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'")
            .bind(&self.host.options.conversation_id).fetch_one(self.pool()).await.unwrap();
        assert_eq!(models, 0, "cleanup must not admit a new model attempt");
        let head = self.host.session_host.canonical_store().unwrap()
            .head(&self.host.options.conversation_id.clone().into()).await.unwrap();
        assert_eq!(head.status, "ready");
        assert!(head.active_turn_id.is_none());
    }
}

fn voice_port(fixture:&Fixture)->super::super::voice_work_host::AppVoiceWorkHost {
    super::super::voice_work_host::AppVoiceWorkHost::new(fixture.owner.clone(),Arc::new(nomifun_voice::SharedVoiceJournal::new(fixture._directory.path().join("voice-only"))),
        fixture.voice_control.clone(),Some(fixture.voice_execution.clone()))
}

#[tokio::test]
async fn voice_actual_host_validates_assembler_active_context_without_projection_or_fake_refs() {
    use nomifun_voice::VoiceWorkPort;
    let fixture=Fixture::new("voice-source-complete-context").await;let port=voice_port(&fixture);
    let owner=&fixture.host.options.user_id;let session=&fixture.host.options.conversation_id;
    let binding=fixture.owner.canonical().store().get_live_session(&session.clone().into()).await.unwrap().agent_binding;
    let context=port.context(owner,session,binding.binding_version).await.unwrap();assert_eq!(context.facts.len(),1);
    let assembled=nomifun_voice::VoiceContextAssembler::initial_facts(&context);assert!(assembled.len()>context.initial_facts.len());
    assert!(port.validate_initial_facts(owner,session,&assembled).await.unwrap(),"real work receipt facts from assembler must validate");
    let authority=fixture.owner.canonical().store().chat_causality_facts(&session.clone().into(),&"voice-reader".into()).await.unwrap();
    assert!(authority.events.iter().any(|event|event.event_id.as_ref()==context.context_reference));
    let mut forged=assembled.clone();let receipt=forged.iter_mut().find(|fact|fact.work_context.is_some()).unwrap();receipt.canonical_receipt_id="binding:invented".into();
    assert!(!port.validate_initial_facts(owner,session,&forged).await.unwrap());
    let mut forged=assembled;let receipt=forged.iter_mut().find(|fact|fact.work_context.is_some()).unwrap();let mut value:Value=serde_json::from_str(&receipt.content).unwrap();
    value["target"]["execution_generation"]=json!(99999);receipt.content=value.to_string();
    assert!(!port.validate_initial_facts(owner,session,&forged).await.unwrap());fixture.finish().await;
}

#[tokio::test]
async fn voice_actual_host_restored_namespace_defers_old_queued_inputs_without_main_admission() {
    let fixture=Fixture::new("voice-restored-queued-namespace").await;
    fixture.voice_execution.shutdown().await.unwrap();
    let owner=fixture.host.options.user_id.clone();let session=fixture.host.options.conversation_id.clone();
    let store=fixture.owner.canonical().store();let binding=store.get_live_session(&session.clone().into()).await.unwrap().agent_binding;
    let data_dir=fixture.services.data_dir.clone();std::fs::write(data_dir.join("storage-generation"),nomifun_common::generate_id()).unwrap();
    let original=super::super::mobile_voice_authority::current_voice_namespace(&data_dir).unwrap();
    let shared=Arc::new(nomifun_voice::SharedVoiceJournal::new(data_dir.clone()));let journal=shared.get_or_open().await.unwrap();let mut keys=Vec::new();
    for (voice,lease) in [("known-namespace",Some(format!("ns:{original}:{}","b".repeat(64)))),("legacy-namespace",None)] {
        journal.activate(nomifun_voice::VoiceActivationFact {voice_session_id:voice.into(),epoch:1,owner_id:owner.clone(),agent_session_id:session.clone(),binding_version:binding.binding_version,
            context_floor:Some(0),lease_revision:lease,route_digest:"a".repeat(64),started_ms:0}).await.unwrap();
        let key=journal.reserve_trigger(voice.into(),1,"explicit-input".into(),"revision-1".into()).await.unwrap().0.operation_key;
        journal.record_pending_intent(owner.clone(),session.clone(),binding.binding_version,0,key.clone(),"This old input must not start after restore".into()).await.unwrap();keys.push(key);
    }
    drop(journal);drop(shared);
    // Reuse the exact Main Session/binding/context while the lifecycle owner
    // supplies a new generation, as a restore does. The sidecar is reopened.
    std::fs::write(data_dir.join("storage-generation"),nomifun_common::generate_id()).unwrap();
    assert_ne!(original,super::super::mobile_voice_authority::current_voice_namespace(&data_dir).unwrap());
    let shared=Arc::new(nomifun_voice::SharedVoiceJournal::new(data_dir));let journal=shared.get_or_open().await.unwrap();
    let port=super::super::voice_work_host::AppVoiceWorkHost::new(fixture.owner.clone(),shared,fixture.voice_control.clone(),Some(fixture.voice_execution.clone()));
    let before=store.head(&session.clone().into()).await.unwrap().last_seq;
    port.drain_inputs(owner.clone(),session.clone()).await;
    assert_eq!(store.head(&session.clone().into()).await.unwrap().last_seq,before,"restored voice inputs never mutate the Main fact chain");
    for key in keys {
        assert_eq!(journal.pending_intent(owner.clone(),session.clone(),key.clone()).await.unwrap().unwrap().phase,nomifun_voice::VoicePendingPhase::Deferred);
        assert_eq!(fixture.owner.voice_operation_receipt(&owner,&session.clone().into(),&key).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::NotFound);
    }
    assert!(journal.queued_intents().await.unwrap().is_empty());fixture.finish().await;
}

async fn recovery_input(journal:&nomifun_voice::VoiceJournal,owner:&str,session:&str,voice:&str,binding:u64,floor:u64,namespace:&str,dispatched:bool)->String {
    journal.activate(nomifun_voice::VoiceActivationFact {voice_session_id:voice.into(),epoch:1,owner_id:owner.into(),agent_session_id:session.into(),binding_version:binding,
        context_floor:Some(floor),lease_revision:Some(format!("ns:{namespace}:{}","b".repeat(64))),route_digest:"a".repeat(64),started_ms:0}).await.unwrap();
    let key=journal.reserve_trigger(voice.into(),1,"explicit-human-input".into(),"revision-1".into()).await.unwrap().0.operation_key;
    journal.record_pending_intent(owner.into(),session.into(),binding,floor,key.clone(),"The explicitly queued work must keep its original operation key".into()).await.unwrap();
    if dispatched {journal.begin_dispatch_intent(owner.into(),session.into(),key.clone(),1).await.unwrap();}key
}
#[tokio::test]
async fn voice_actual_lazy_recovery_defers_old_namespace_binding_and_cleared_context_without_main_admission(){
    use nomifun_voice::VoiceWorkPort;
    let fixture=Fixture::new("voice-lazy-recovery-stale-scopes").await;fixture.voice_execution.shutdown().await.unwrap();
    let owner=fixture.host.options.user_id.clone();let session=fixture.host.options.conversation_id.clone();let id=AgentSessionId::from(session.clone());let store=fixture.owner.canonical().store();
    let binding=store.get_live_session(&id).await.unwrap().agent_binding.binding_version;let data_dir=fixture.services.data_dir.clone();std::fs::write(data_dir.join("storage-generation"),nomifun_common::generate_id()).unwrap();
    let namespace=super::super::mobile_voice_authority::current_voice_namespace(&data_dir).unwrap();
    let shared=Arc::new(nomifun_voice::SharedVoiceJournal::new(data_dir.clone()));let journal=shared.get_or_open().await.unwrap();let mut keys=vec![];
    keys.push(recovery_input(&journal,&owner,&session,"old-root",binding,0,&"c".repeat(64),false).await);
    keys.push(recovery_input(&journal,&owner,&session,"old-binding",binding+1,0,&namespace,false).await);
    keys.push(recovery_input(&journal,&owner,&session,"old-context",binding,0,&namespace,false).await);
    fixture.owner.canonical().cancel_exact_turn(&PrincipalRef {principal_kind:"user".into(),principal_id:owner.clone()},&id,"recovery-fixture-close",&"cleanup-retry-turn".into()).await.unwrap();
    // This fixture's original Runtime descriptor does not expose API clear.
    // Use the original typed canonical event owner to test the floor fence;
    // this is not evidence that the fixture supports Runtime/UI clear.
    let facts=store.chat_causality_facts(&id,&"recovery-clear-reader".into()).await.unwrap();let identity=nomifun_common::generate_id();
    store.append_event(&SessionEventAppend {agent_session_id:id.clone(),event_id:identity.clone().into(),producer_id:"session_api".into(),idempotency_key:identity.into(),
        semantic_event:SemanticSessionEventDraft {kind:SessionEventKind("context/cleared".into()),kind_version:1,correlation_id:session.clone().into(),causation_event_id:facts.events.last().map(|event|event.event_id.clone()),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"reason":"isolated_fixture_floor_fence"})))}}).await.unwrap();
    drop(journal);drop(shared);let shared=Arc::new(nomifun_voice::SharedVoiceJournal::new(data_dir));let journal=shared.get_or_open().await.unwrap();
    let port=super::super::voice_work_host::AppVoiceWorkHost::new(fixture.owner.clone(),shared,fixture.voice_control.clone(),Some(fixture.voice_execution.clone()));let before=store.head(&id).await.unwrap().last_seq;
    assert!(port.recover_pending_inputs("wrong-owner",&session).await.is_err());port.recover_pending_inputs(&owner,&session).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2),async{loop {if journal.queued_intents().await.unwrap().is_empty(){break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    for key in keys {assert_eq!(journal.pending_intent(owner.clone(),session.clone(),key.clone()).await.unwrap().unwrap().phase,nomifun_voice::VoicePendingPhase::Deferred);assert_eq!(fixture.owner.voice_operation_receipt(&owner,&id,&key).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::NotFound);}
    assert_eq!(store.head(&id).await.unwrap().last_seq,before);port.shutdown_pending_inputs().await.unwrap();fixture.finish().await;
}
#[tokio::test]
async fn voice_actual_lazy_recovery_admits_known_queued_original_key_once_and_unknown_dispatch_is_lookup_only(){
    use nomifun_voice::VoiceWorkPort;use tokio::io::AsyncWriteExt;
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let server=tokio::spawn(async move{loop {let(mut socket,_)=listener.accept().await.unwrap();let _=socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await;let _=socket.shutdown().await;}});
    let model_base=format!("http://{address}/v1");
    let fixture=Fixture::build_delivery("voice-lazy-recovery-original-key",vec![],vec![],None,false,Some(&model_base),#[cfg(feature="browser-use")]None).await;
    fixture.voice_execution.shutdown().await.unwrap();let owner=fixture.host.options.user_id.clone();let session=fixture.host.options.conversation_id.clone();let id=AgentSessionId::from(session.clone());
    let store=fixture.owner.canonical().store();let binding=store.get_live_session(&id).await.unwrap().agent_binding.binding_version;let data_dir=fixture.services.data_dir.clone();std::fs::write(data_dir.join("storage-generation"),nomifun_common::generate_id()).unwrap();
    let namespace=super::super::mobile_voice_authority::current_voice_namespace(&data_dir).unwrap();let shared=Arc::new(nomifun_voice::SharedVoiceJournal::new(data_dir.clone()));let journal=shared.get_or_open().await.unwrap();
    let known=recovery_input(&journal,&owner,&session,"known-queued",binding,0,&namespace,false).await;let unknown=recovery_input(&journal,&owner,&session,"unknown-dispatch",binding,0,&namespace,true).await;
    fixture.owner.canonical().cancel_exact_turn(&PrincipalRef {principal_kind:"user".into(),principal_id:owner.clone()},&id,"recovery-fixture-close",&"cleanup-retry-turn".into()).await.unwrap();
    drop(journal);drop(shared);let shared=Arc::new(nomifun_voice::SharedVoiceJournal::new(data_dir));let journal=shared.get_or_open().await.unwrap();
    let port=super::super::voice_work_host::AppVoiceWorkHost::new(fixture.owner.clone(),shared,fixture.voice_control.clone(),Some(fixture.voice_execution.clone()));
    assert_eq!(fixture.runtime.status(),None,"the actual cold official Runtime has not opened any task; recovery must handle its pristine state");
    let(a,b)=tokio::join!(port.recover_pending_inputs(&owner,&session),port.recover_pending_inputs(&owner,&session));a.unwrap();b.unwrap();
    let receipt=tokio::time::timeout(Duration::from_secs(5),async{loop {let receipt=fixture.owner.voice_operation_receipt(&owner,&id,&known).await.unwrap();if receipt.started_event.is_some(){break receipt;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    let starts:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/started' AND correlation_id=?").bind(&session).bind(receipt.operation_id.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(starts,1);
    assert_eq!(fixture.owner.voice_operation_receipt(&owner,&id,&unknown).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::NotFound);
    assert_eq!(journal.pending_intent(owner.clone(),session.clone(),unknown).await.unwrap().unwrap().phase,nomifun_voice::VoicePendingPhase::Dispatched);
    port.shutdown_pending_inputs().await.unwrap();fixture.owner.canonical().cancel_exact_turn(&PrincipalRef {principal_kind:"user".into(),principal_id:owner},&id,"recovery-cleanup",&receipt.operation_id).await.unwrap();fixture.finish().await;server.abort();let _=server.await;
}

async fn voice_pending_decision_fixture(fixture:&Fixture)->nomifun_agent_contracts::AgentSessionId {
    use nomifun_db::{IAgentExecutionRepository,SqliteAgentExecutionRepository,CreateAgentExecutionParams,NewAgentExecutionParticipant,NewAgentExecutionEvent,ReconcileAgentExecutionPlanParams,
        NewAgentExecutionStep,CreateAgentExecutionAttemptParams,AgentExecutionAttemptSessionKind,SettleAgentExecutionAttemptParams,AgentExecutionActiveTurnGuard,AttemptConversationEffects};
    use nomifun_common::{AgentExecutionStatus,AgentExecutionEventKind,AgentExecutionActor,ExecutionStepStatus,ExecutionAttemptStatus};
    fixture.voice_execution.shutdown().await.unwrap();let repository=SqliteAgentExecutionRepository::new(fixture.pool().clone());
    let owner=&fixture.host.options.user_id;let session=&fixture.host.options.conversation_id;let participant=nomifun_common::generate_id();let step=nomifun_common::generate_id();
    let event=|kind|NewAgentExecutionEvent {event_type:kind,step_id:None,attempt_id:None,actor:AgentExecutionActor::system(),payload:"{}".into()};
    let projection=fixture.owner.get_session(owner,session).await.unwrap();
    let model=projection.model.as_ref().expect("the admitted fixture has a frozen model binding");
    let created=repository.create_execution_with_participants(owner,&CreateAgentExecutionParams {goal:"Choose a pending option".into(),status:AgentExecutionStatus::Planning,
        adaptation_policy:nomifun_common::AdaptationPolicy::Fixed,decision_policy:nomifun_common::DecisionPolicy::AskUser,delegation_policy:nomifun_common::DelegationPolicy::Automatic,max_parallel:1,
        work_dir:None,lead_conversation_id:None,initial_plan_input:r#"{"mode":"automatic"}"#.into()},&[NewAgentExecutionParticipant {participant_id:participant.clone(),
            // This is the original model-configured participant path. The
            // Desktop presentation snapshot is not AgentPreset lineage.
            source_agent_id:"0190f5fe-7c00-7a00-8000-000000000114".into(),preset_id:None,preset_revision:None,agent_snapshot:None,
            provider_id:Some(model.provider_id.to_string()),model:Some(model.model.clone()),role:Some("builder".into()),capability:None,constraints:None,description:None,system_prompt:None,
            enabled_skills:"[]".into(),disabled_builtin_skills:"[]".into(),sort_order:0}],&event(AgentExecutionEventKind::Created)).await.unwrap();
    let profile=nomifun_api_types::ExecutionStepProfile {kind:"general".into(),needs_vision:false,needs_web_search:false,needs_long_context:false,needs_high_reasoning:false,bulk:false,managed_process_only:false};
    let planned=repository.reconcile_plan(owner,&created.execution_id,created.version,&ReconcileAgentExecutionPlanParams {goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,
        keep_step_ids:vec![],new_participants:vec![],retire_participant_ids:vec![],new_dependencies:vec![],execution_status:AgentExecutionStatus::Running,new_steps:vec![NewAgentExecutionStep {step_id:step.clone(),
            title:"Choose option".into(),spec:"Await exact decision".into(),role:Some("builder".into()),tool_policy:nomifun_common::AgentToolPolicy::Full,kind:nomifun_common::ExecutionStepKind::Agent,
            agent_mode:Some(nomifun_common::AgentStepMode::Normal),profile:Some(serde_json::to_string(&profile).unwrap()),fanout_group:None,control_policy:None,status:ExecutionStepStatus::Pending,assigned_participant_id:Some(participant.clone()),
            assignment_score:Some(1.0),assignment_rationale:None,assignment_source:Some(nomifun_common::ParticipantAssignmentSource::Planner),assignment_locked:false,failure_policy:nomifun_common::StepFailurePolicy::FailExecution,
            preset_prompt:None,graph_x:None,graph_y:None}]},&event(AgentExecutionEventKind::PlanChanged)).await.unwrap();
    let queued=repository.create_attempt(owner,&created.execution_id,&step,planned.steps[0].version,None,&CreateAgentExecutionAttemptParams {participant_id:Some(participant),start_immediately:false,
        trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None},&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let attempt=queued.current_attempt.unwrap().attempt;
    let running=repository.start_attempt(owner,&created.execution_id,&step,queued.step.version,&attempt.attempt_id,attempt.version,session,AgentExecutionAttemptSessionKind::ChildAttempt,None,
        &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();let attempt=running.current_attempt.unwrap().attempt;let stop=nomifun_common::generate_id();
    let mut effects=AttemptConversationEffects::default();effects.push_stop_turn(stop.clone(),"cleanup-retry-turn".into()).unwrap();
    repository.settle_attempt(owner,&created.execution_id,&step,running.step.version,&attempt.attempt_id,attempt.version,None,&SettleAgentExecutionAttemptParams {
        expected_active_session_turn:Some(AgentExecutionActiveTurnGuard {conversation_id:session.clone(),canonical_operation_id:"cleanup-retry-turn".into()}),attempt_status:ExecutionAttemptStatus::WaitingInput,
        step_status:ExecutionStepStatus::WaitingInput,execution_status:Some(AgentExecutionStatus::WaitingInput),question:Some(Some("Choose option A or option B?".into())),error:None,output_summary:None,
        output_files:None,tokens:None,retry_after:None,runtime_state:Some(Some(effects.encode().unwrap())),started_at:None,finished_at:None,loop_repeat_reset:None},
        &NewAgentExecutionEvent {event_type:AgentExecutionEventKind::DecisionRequested,step_id:Some(step.clone()),attempt_id:Some(attempt.attempt_id.clone()),actor:AgentExecutionActor::system(),
            payload:json!({"question":"Choose option A or option B?","stop_turn_operation_id":stop}).to_string()}).await.unwrap();session.clone().into()
}

#[tokio::test]
async fn voice_actual_host_decision_uses_original_cas_and_stable_exact_answer_receipt() {
    use nomifun_voice::VoiceWorkPort;use nomifun_voice_contracts::{VoiceApprovalAnswer,ApprovalInteractionMode};
    let fixture=Fixture::new("voice-original-decision-cas").await;let session=voice_pending_decision_fixture(&fixture).await;let port=voice_port(&fixture);let owner=&fixture.host.options.user_id;
    let binding=fixture.owner.canonical().store().get_live_session(&session).await.unwrap().agent_binding;
    let context=port.context(owner,session.as_ref(),binding.binding_version).await.unwrap();assert_eq!(context.approvals.len(),1);assert!(port.validate_initial_facts(owner,session.as_ref(),&nomifun_voice::VoiceContextAssembler::initial_facts(&context)).await.unwrap());
    let target=context.approvals[0].target.clone();let answer=VoiceApprovalAnswer {target:target.clone(),answer:"Use option A".into(),presented_context_id:target.presentation_id.clone()};
    let mut isolated=answer.clone();isolated.presented_context_id.clear();assert!(port.answer_approval(owner,"isolated",isolated).await.is_err());
    let mut click=answer.clone();click.target.interaction_mode=ApprovalInteractionMode::ExplicitClick;assert!(port.answer_approval(owner,"click",click).await.is_err());
    let first=port.answer_approval(owner,"exact-answer",answer.clone()).await.unwrap();assert!(!first.duplicate);assert_eq!(first.status,nomifun_voice_contracts::VoiceWorkStatus::Applied);
    let replay=port.answer_approval(owner,"exact-answer",answer.clone()).await.unwrap();assert!(replay.duplicate);assert_eq!(replay.receipt_id,first.receipt_id);
    let mut different=answer;different.answer="Use option B".into();assert!(port.answer_approval(owner,"exact-answer",different).await.is_err());
    let events=fixture.voice_execution.events(owner,&target.execution_id,None,Some(128)).await.unwrap();assert_eq!(events.iter().filter(|event|event.event_type==nomifun_common::AgentExecutionEventKind::DecisionAnswered).count(),1);
    fixture.finish().await;
}

#[tokio::test]
async fn voice_off_original_decision_cas_keeps_its_unextended_payload() {
    use nomifun_voice::VoiceWorkPort;
    let fixture=Fixture::new("voice-off-original-decision").await;let session=voice_pending_decision_fixture(&fixture).await;let port=voice_port(&fixture);let owner=&fixture.host.options.user_id;
    let binding=fixture.owner.canonical().store().get_live_session(&session).await.unwrap().agent_binding;let context=port.context(owner,session.as_ref(),binding.binding_version).await.unwrap();let target=&context.approvals[0].target;
    fixture.voice_execution.answer_decision(owner,&nomifun_common::AgentExecutionActor::user(owner),&target.execution_id,&target.step_id,&target.attempt_id,
        nomifun_api_types::AnswerExecutionDecisionRequest {answer:"Use original UI answer".into(),expected_execution_version:target.expected_execution_version,expected_step_version:target.expected_step_version,expected_attempt_version:target.expected_attempt_version}).await.unwrap();
    let events=fixture.voice_execution.events(owner,&target.execution_id,None,Some(128)).await.unwrap();let answer=events.iter().find(|event|event.event_type==nomifun_common::AgentExecutionEventKind::DecisionAnswered).unwrap();
    assert_eq!(answer.payload.as_object().unwrap().len(),2);assert_eq!(answer.payload["answered"],true);assert!(answer.payload["operation_id"].is_string());
    assert!(answer.payload.get("voice_operation_key").is_none());fixture.finish().await;
}

#[tokio::test]
async fn admitted_workspace_context_reaches_the_formal_turn_without_a_probe() {
    let fixture=Fixture::new("admitted-workspace-context").await;
    let admitted=fixture.host.session_host.read_turn_receipt(
        &fixture.host.options,&fixture.host.binding,&fixture.host.snapshot_ref,&fixture.message,
    ).await.unwrap();
    // macOS temp roots may use /var while admission resolves /private/var.
    // Verify the exact admitted data, not the pre-admission path spelling.
    let admitted_root=admitted.session().workspace();
    assert_eq!(std::fs::canonicalize(admitted_root).unwrap(),
        std::fs::canonicalize(&fixture.host.options.workspace).unwrap());
    println!("WORKSPACE_CONTEXT_ROOT options={} admitted={admitted_root}",fixture.host.options.workspace);
    let prepared=fixture.host.prepare_turn(&fixture.message,CancellationToken::new()).await.unwrap();
    let expected=admitted_workspace_context(admitted_root).unwrap();
    assert_eq!(prepared.model_request.input.instructions.iter().filter(|instruction|*instruction==&expected).count(),1);
    let session=&fixture.host.options.conversation_id;
    let models:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'")
        .bind(session).fetch_one(fixture.pool()).await.unwrap();
    let effects:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=?")
        .bind(session).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(models,0,"context preparation must not spend a model request");
    assert_eq!(effects,0,"admitted workspace data must not trigger a cwd or listing command");
    fixture.finish().await;
}

#[tokio::test]
async fn cancellation_receipt_identity_rejects_altered_empty_delivery() {
    cancellation_identity_scenario(false).await;
}

#[tokio::test]
async fn cancellation_replay_and_delayed_stop_preserve_the_real_successor_runtime() {
    use nomifun_agent_execution::AgentExecutionSessionPort;
    let fixture = Fixture::new("cancel-target-isolation").await;
    let session = AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let cancel_path = format!("/api/agent-sessions/{}/turns/cancel", session.as_ref());
    let first = api(&fixture._router, &cancel_path, json!({"idempotency_key":"stable-stop-effect"})).await;
    assert_eq!(first["target_operation_id"], "cleanup-retry-turn");
    let store = fixture.owner.canonical().store();
    let (_, admitted) = store.start_turn(&session, "session_api".into(), "successor-admission".into(),
        "successor-operation".into(), StrictJsonValue(json!({"content":"a later user turn", "admission":{
            "route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref,
        }}))).await.unwrap();
    let successor_generation = store.native_execution_generation(&session, &"successor-operation".into()).await.unwrap();
    assert_eq!(successor_generation, admitted.record.as_ref().unwrap().seq);
    let runtimes = &fixture.services.agent_runtime_sessions;
    runtimes.get_or_create_runtime_for_turn(session.as_ref(), successor_generation,
        CancellationToken::new(), fixture.registry_options()).await.unwrap();
    assert_eq!(runtimes.active_turn_generation(session.as_ref()), Some(successor_generation));
    assert!(runtimes.get_runtime(session.as_ref()).is_some());

    let replay = api(&fixture._router, &cancel_path, json!({"idempotency_key":"stable-stop-effect"})).await;
    assert_eq!(replay["target_operation_id"], "cleanup-retry-turn");
    assert_eq!(replay["message_id"], first["message_id"]);
    assert_eq!(replay["duplicate"], true);
    fixture.owner.cancel_turn_for_execution(fixture.services.authoritative_user_id.as_ref(), session.as_ref(),
        "delayed-durable-stop", "cleanup-retry-turn").await.unwrap();
    let late_steer = fixture.owner.steer_turn_for_execution(fixture.services.authoritative_user_id.as_ref(), session.as_ref(),
        "first-late-durable-steer", "cleanup-retry-turn", nomifun_api_types::SendMessageRequest {
            content:"This belongs only to the cancelled original task".into(),files:vec![],inject_skills:vec![],
            hidden:false,origin:Some("agent_execution".into()),channel_platform:None,plugin_delivery:None,
        }).await.unwrap();
    assert_eq!(late_steer, first["message_id"].as_str().unwrap());
    assert_eq!(store.head(&session).await.unwrap().active_turn_id.as_deref(), Some("successor-operation"));
    assert_eq!(runtimes.active_turn_generation(session.as_ref()), Some(successor_generation));
    assert!(runtimes.get_runtime(session.as_ref()).is_some(), "late/replayed stop must not quarantine a successor runtime");
    assert_eq!(store.read_turn_receipt(&session, &"successor-operation".into()).await.unwrap().status,
        nomifun_agent_session::TurnReceiptStatus::Running);
    let cancellations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/cancelled'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(cancellations, 1, "late stop reuses the original terminal without manufacturing another cancellation");
    let steering: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/steer-accepted'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(steering, 0, "first late delivery cannot add steering to the successor");
    fixture.finish().await;
}

#[tokio::test]
async fn steering_replay_recovers_the_commit_to_runtime_queue_gap_once() {
    use nomifun_agent_execution::AgentExecutionSessionPort;
    let fixture = Fixture::new("steer-delivery-gap").await;
    let session = AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let target = OperationId::from("cleanup-retry-turn");
    let generation = fixture.owner.canonical().store().native_execution_generation(&session, &target).await.unwrap();
    let runtime = fixture.services.agent_runtime_sessions.get_or_create_runtime_for_turn(session.as_ref(), generation,
        CancellationToken::new(), fixture.registry_options()).await.unwrap();
    let host = fixture.host.clone();
    let mut message = fixture.message.clone();
    message.msg_id = message.source_message_id.clone().unwrap();
    host.admit_preparation(&message, CancellationToken::new()).await.unwrap();
    host.record_event(&message, &AgentEngineEvent::TurnStarted {
        binding:host.engine_binding.clone(),turn_operation_id:target.clone(),
    }).await.unwrap();
    let request = || nomifun_api_types::SendMessageRequest {
        content:"Original committed steering\n中文纠正必须保留".into(),files:vec![],inject_skills:vec![],hidden:false,
        origin:Some("agent_execution".into()),channel_platform:None,plugin_delivery:None,
    };
    // The real canonical admission commits, then its sender disappears before
    // calling the native queue. A retry sees duplicate=true but must deliver.
    let receipt = fixture.owner.canonical().steer_exact_turn(&PrincipalRef {
        principal_kind:"user".into(),principal_id:fixture.services.authoritative_user_id.to_string(),
    }, &session, "stable-steering-effect", &target, super::super::nomi_core_session::canonical_turn_input(&request())).await.unwrap();
    assert!(!receipt.duplicate);
    assert!(host.active.lock().await.as_ref().unwrap().steering.pending_receipt_ids().is_empty());
    for _ in 0..2 {
        let delivered = fixture.owner.steer_turn_for_execution(fixture.services.authoritative_user_id.as_ref(), session.as_ref(),
            "stable-steering-effect", target.as_ref(), request()).await.unwrap();
        assert_eq!(delivered, receipt.event_id.as_ref());
        assert_eq!(host.active.lock().await.as_ref().unwrap().steering.pending_receipt_ids(), vec![delivered]);
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/steer-accepted'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(count, 1);
    runtime.kill_and_wait(None).await.unwrap();
    fixture.finish().await;
}

#[tokio::test]
async fn claimed_preparation_cancel_keeps_same_cleanup_owner() {
    claimed_preparation_scenario(true).await;
}

#[tokio::test]
async fn claimed_preparation_drop_keeps_same_cleanup_owner() {
    claimed_preparation_scenario(false).await;
}

#[tokio::test]
async fn claimed_preparation_cannot_confirm_another_execution_owner() {
    let fixture = Fixture::new("claimed-foreign-owner").await;
    let store = fixture.host.session_host.canonical_store().unwrap();
    let foreign = uuid::Uuid::now_v7().to_string();
    let lease = store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
        owner:fixture.host.principal.clone(), agent_session_id:fixture.host.options.conversation_id.clone().into(),
        operation_id:"cleanup-retry-turn".into(), snapshot:fixture.host.snapshot_ref.clone(),
        active_set_generation:0, holder:foreign.clone(), expected_fence:0, checkpoint:None,
    }).await.unwrap();
    store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),
        "cancel-foreign-execution".into(), "session_api".into()).await.unwrap();
    let before = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    assert!(fixture.host.cleanup_turn(&fixture.message).await.is_err());
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    assert!(fixture.host.active.lock().await.is_none());
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    let actual: (i64, Option<String>, i64) = sqlx::query_as("SELECT execution_generation,execution_owner,execution_fence FROM agent_turns WHERE session_id=?")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(actual, (lease.generation() as i64, Some(foreign), 0));
    fixture.assert_settled().await;
    fixture.finish().await;
}

async fn claimed_preparation_scenario(canonical_cancel: bool) {
    use std::{future::Future, task::Poll};
    let fixture = Fixture::new(if canonical_cancel {"claimed-canonical-cancel"} else {"claimed-driver-drop"}).await;
    let cancellation = CancellationToken::new();
    let mut preparation = Box::pin(fixture.host.admit_preparation(&fixture.message, cancellation.clone()));
    let claimed: (i64, Option<String>) = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let pending = std::future::poll_fn(|context| Poll::Ready(preparation.as_mut().poll(context).is_pending())).await;
            assert!(pending, "preparation must expose its real database await before caller drop");
            let claim: (i64, Option<String>) = sqlx::query_as("SELECT execution_generation,execution_owner FROM agent_turns WHERE session_id=?")
                .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
            if claim.0 > 0 && claim.1.is_some() { break claim; }
        }
    }).await.expect("native claim must become visible within the preparation budget");
    let active_at_claim = fixture.host.active.lock().await.is_some();
    let store = fixture.host.session_host.canonical_store().unwrap();
    if canonical_cancel {
        store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),
            "cancel-during-native-claim".into(), "session_api".into()).await.unwrap();
    }
    cancellation.cancel();
    drop(preparation);
    println!("CLAIMED_PREPARATION canonical_cancel={canonical_cancel} generation={} active_at_claim={active_at_claim}", claimed.0);
    fixture.host.cleanup_turn(&fixture.message).await
        .expect("the original claimed preparation must retain its cleanup owner");
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    let final_claim: (i64, Option<String>, i64) = sqlx::query_as("SELECT execution_generation,execution_owner,execution_fence FROM agent_turns WHERE session_id=?")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(final_claim.0, claimed.0);
    assert_eq!(final_claim.1, claimed.1);
    assert_eq!(final_claim.2, 0, "cleanup must not recover or replace the original execution fence");
    let claim_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/execution-claimed'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(claim_count, 1);
    let before_repeat = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before_repeat);
    fixture.finish().await;
}

#[tokio::test]
async fn cancellation_receipt_identity_preserves_unopened_attachment_delivery() {
    cancellation_identity_scenario(true).await;
}

async fn cancellation_identity_scenario(attachments: bool) {
    let fixture = if attachments {
        Fixture::with_delivery("cancel-identity-attachments", vec!["cancel-note.txt".into(), "cancel-context.txt".into()],
            vec![], Some("fixture-source".into())).await
    } else {
        Fixture::with_delivery("cancel-identity-empty", vec![], vec![], None).await
    };
    let store = fixture.host.session_host.canonical_store().unwrap();
    let (_, terminal) = store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),
        "cancel-delivery-identity".into(), "session_api".into()).await.unwrap();
    let original_terminal = terminal.record.unwrap().event_id;
    let before = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    let mut altered_files = fixture.message.clone();
    if attachments { altered_files.files.reverse(); }
    else { altered_files.files.push("unaccepted.txt".into()); }
    let mut altered_skills = fixture.message.clone();
    altered_skills.inject_skills.push("unaccepted-skill".into());
    let mut altered_origin = fixture.message.clone();
    altered_origin.origin = Some("altered-origin".into());
    let mut missing_origin = fixture.message.clone();
    missing_origin.origin = None;
    let mut failures = Vec::new();
    for (field, candidate) in [("files", altered_files), ("inject_skills", altered_skills), ("origin", altered_origin)] {
        let cleanup_rejected = fixture.host.cleanup_turn(&candidate).await.is_err();
        let terminal_rejected = fixture.host.record_event(&candidate, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err();
        if !cleanup_rejected || !terminal_rejected { failures.push((field, cleanup_rejected, terminal_rejected)); }
    }
    if attachments && fixture.host.cleanup_turn(&missing_origin).await.is_ok() {
        failures.push(("missing_origin", false, false));
    }
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    assert!(fixture.host.active.lock().await.is_none());
    // An accepted cancelled delivery closes through the actual SDK without
    // reading its attachment references or dispatching a model or tool.
    fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), fixture.runtime.cancel()).await.unwrap().unwrap();
    let receipt = store.read_turn_receipt(&fixture.host.options.conversation_id.clone().into(),
        &"cleanup-retry-turn".into()).await.unwrap();
    assert_eq!(receipt.terminal_event.unwrap().event_id, original_terminal);
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    let lease: (i64, Option<String>) = sqlx::query_as("SELECT execution_generation,execution_owner FROM agent_turns WHERE session_id=?")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(lease, (0, None));
    fixture.assert_settled().await;
    println!("CANCEL_DELIVERY_IDENTITY attachments={attachments} unaccepted_fields={failures:?}");
    fixture.finish().await;
    assert!(failures.is_empty(), "cancel acknowledgement accepted altered delivery fields: {failures:?}");
}

#[tokio::test]
async fn cleanup_retry_keeps_half_committed_initialization_after_suspension_fault() {
    let fixture = Fixture::new("initialization").await;
    fixture.host.admit_preparation(&fixture.message, CancellationToken::new()).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_input_scope BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='turn_input_scope' BEGIN SELECT RAISE(FAIL,'fixture input scope write failure'); END")
        .execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_cleanup_pause BEFORE INSERT ON agent_events WHEN NEW.kind='turn/paused' BEGIN SELECT RAISE(FAIL,'fixture pause write failure'); END")
        .execute(fixture.pool()).await.unwrap();
    let failure = fixture.host.cleanup_turn(&fixture.message).await.unwrap_err();
    assert!(failure.to_string().contains("fixture input scope write failure"));
    assert_eq!(fixture.host.active.lock().await.as_ref().unwrap().journal.sequence(), 1);
    assert!(fixture.host.suspend_after_cleanup_failure(&fixture.message).await.is_err());
    assert_eq!(fixture.state().await, "running");
    sqlx::query("DROP TRIGGER reject_input_scope").execute(fixture.pool()).await.unwrap();
    sqlx::query("DROP TRIGGER reject_cleanup_pause").execute(fixture.pool()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    let events: Vec<String> = sqlx::query_scalar("SELECT json_extract(inline_json,'$.event.event') FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' ORDER BY seq")
        .bind(&fixture.host.options.conversation_id).fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(events, vec!["turn_started", "turn_input_scope", "host_cleanup_proven", "turn_cancelled"]);
    fixture.finish().await;
}

#[tokio::test]
async fn cleanup_retry_rejects_unclaimed_turn_under_writer_lock_then_settles() {
    let fixture = Fixture::new("unclaimed-writer-lock").await;
    assert!(fixture.host.active.lock().await.is_none());
    let writer_pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(&fixture.database_path)
            .busy_timeout(Duration::from_secs(2))).await.unwrap();
    let writer = writer_pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(8), fixture.host.cleanup_turn(&fixture.message)).await
        .expect("claim failure must be bounded");
    assert_eq!(fixture.state().await, "running");
    assert!(fixture.host.active.lock().await.is_none());
    assert!(result.is_err(), "cleanup must retain the canonical admission/claim error");
    assert!(result.unwrap_err().to_string().contains("locked"));
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    assert!(!fixture.host.terminal_already_recorded(fixture.host.root(&fixture.message)).unwrap());
    writer.rollback().await.unwrap();
    writer_pool.close().await;
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    fixture.finish().await;
}

#[tokio::test]
async fn cleanup_retry_rejects_terminal_ack_without_admitted_turn() {
    let fixture = Fixture::new("unadmitted-terminal").await;
    assert!(fixture.host.active.lock().await.is_none());
    let result = fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await;
    assert_eq!(fixture.state().await, "running");
    assert!(result.is_err(), "missing host admission cannot acknowledge a canonical terminal");
    assert!(!fixture.host.terminal_already_recorded(fixture.host.root(&fixture.message)).unwrap());
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    fixture.finish().await;
}

#[tokio::test]
async fn steering_cleanup_retry_retains_tail_after_private_record_failure() {
    steering_cleanup_scenario(SteeringCleanupFault::PrivateRecord).await;
}

#[tokio::test]
async fn steering_cleanup_retry_retains_tail_after_public_projection_failure() {
    steering_cleanup_scenario(SteeringCleanupFault::PublicProjection).await;
}

#[tokio::test]
async fn steering_cleanup_retry_retains_deferred_receipt_until_ack() {
    steering_cleanup_scenario(SteeringCleanupFault::DeferredReceipt).await;
}

#[derive(Clone, Copy)]
enum SteeringCleanupFault { PrivateRecord, PublicProjection, DeferredReceipt }

#[tokio::test]
async fn steering_cleanup_retry_acknowledges_exact_cancelled_root_before_first_poll() {
    let fixture = Fixture::new("cancelled-before-first-poll").await;
    let store = fixture.host.session_host.canonical_store().unwrap();
    let (_, cancelled) = store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),
        "cancel-before-first-poll".into(), "session_api".into()).await.unwrap();
    let cancelled_id = cancelled.record.unwrap().event_id;
    let before = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    assert_eq!(fixture.state().await, "cancelled");
    assert!(fixture.host.active.lock().await.is_none());
    let mut wrong_root = fixture.message.clone();
    wrong_root.source_message_id = Some(uuid::Uuid::now_v7().to_string());
    assert!(fixture.host.cleanup_turn(&wrong_root).await.is_err());
    assert!(fixture.host.record_event(&wrong_root, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    let mut wrong_text = fixture.message.clone();
    wrong_text.content = "different accepted body".into();
    assert!(fixture.host.cleanup_turn(&wrong_text).await.is_err());
    assert!(fixture.host.record_event(&wrong_text, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:1}).await.is_err());
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCompleted {
        model_steps:0, finish_reason:ChatFinishReason::Completed,
    }).await.is_err());
    let mut foreign_owner = fixture.host.options.clone();
    foreign_owner.user_id = uuid::Uuid::now_v7().to_string();
    assert!(fixture.host.session_host.confirm_cancelled_before_execution(&foreign_owner,
        &fixture.host.binding, &fixture.host.snapshot_ref, &fixture.message).await.is_err());
    let mut foreign_session = fixture.host.options.clone();
    foreign_session.conversation_id = uuid::Uuid::now_v7().to_string();
    assert!(fixture.host.session_host.confirm_cancelled_before_execution(&foreign_session,
        &fixture.host.binding, &fixture.host.snapshot_ref, &fixture.message).await.is_err());
    let mut foreign_snapshot = fixture.host.snapshot_ref.clone();
    foreign_snapshot.snapshot_digest = "0".repeat(64).into();
    assert!(fixture.host.session_host.confirm_cancelled_before_execution(&fixture.host.options,
        &fixture.host.binding, &foreign_snapshot, &fixture.message).await.is_err());
    let mut foreign_binding = fixture.host.binding.clone();
    foreign_binding.build_digest = "0".repeat(64);
    assert!(fixture.host.session_host.confirm_cancelled_before_execution(&fixture.host.options,
        &foreign_binding, &fixture.host.snapshot_ref, &fixture.message).await.is_err());
    // On this current-thread runtime send returns after spawning; no yield
    // occurs before cancel sets the token, so the SDK's biased cancel wins
    // before the first driver poll. The canonical cancel is already durable.
    fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), fixture.runtime.cancel()).await
        .expect("cancel before first driver poll must remain bounded")
        .expect("a durable cancel before first driver poll must not strand teardown");
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    // The special witness must not turn later altered messages into a match.
    assert!(fixture.host.cleanup_turn(&wrong_text).await.is_err());
    assert!(fixture.host.record_event(&wrong_root, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    let receipt = store.read_turn_receipt(&fixture.host.options.conversation_id.clone().into(),
        &"cleanup-retry-turn".into()).await.unwrap();
    assert_eq!(receipt.terminal_event.unwrap().event_id, cancelled_id);
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    let lease: (i64, Option<String>) = sqlx::query_as("SELECT execution_generation,execution_owner FROM agent_turns WHERE session_id=?")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(lease, (0, None));
    fixture.assert_settled().await;
    fixture.finish().await;
}

#[tokio::test]
async fn steering_cleanup_retry_cannot_acknowledge_completed_root_as_cancelled() {
    let fixture = Fixture::new("completed-before-first-poll").await;
    let store = fixture.host.session_host.canonical_store().unwrap();
    let started = store.read_turn_receipt(&fixture.host.options.conversation_id.clone().into(),
        &"cleanup-retry-turn".into()).await.unwrap().started_event.unwrap();
    let completion_id: EventId = "fixture-completed-before-poll".into();
    store.append_turn_terminal(&SessionEventAppend {
        agent_session_id: fixture.host.options.conversation_id.clone().into(), event_id: completion_id,
        producer_id:"session_api".into(), idempotency_key:"fixture-completed-before-poll".into(),
        semantic_event:SemanticSessionEventDraft { kind:SessionEventKind("turn/completed".into()), kind_version:1,
            correlation_id:"cleanup-retry-turn".into(), causation_event_id:Some(started.event_id),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"model_steps":0,"finish_reason":ChatFinishReason::Completed}))),
        },
    }, &"cleanup-retry-turn".into()).await.unwrap();
    let before = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    assert!(fixture.host.cleanup_turn(&fixture.message).await.is_err());
    assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.is_err());
    assert_eq!(fixture.state().await, "completed");
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before);
    assert!(!fixture.host.terminal_already_recorded(fixture.host.root(&fixture.message)).unwrap());
    fixture.finish().await;
}

async fn steering_cleanup_scenario(fault: SteeringCleanupFault) {
    let scenario = match fault {
        SteeringCleanupFault::PrivateRecord => "steering-private-record",
        SteeringCleanupFault::PublicProjection => "steering-public-projection",
        SteeringCleanupFault::DeferredReceipt => "steering-deferred-receipt",
    };
    let fixture = Fixture::new(scenario).await;
    fixture.host.admit_preparation(&fixture.message, CancellationToken::new()).await.unwrap();
    let (operation, generation) = {
        let active = fixture.host.active.lock().await;
        let turn = active.as_ref().unwrap();
        (turn.operation.clone(), turn.epoch as u64)
    };
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnStarted {
        binding: fixture.host.engine_binding.clone(), turn_operation_id: operation.clone().into(),
    }).await.unwrap();
    let store = fixture.host.session_host.canonical_store().unwrap();
    let queued_text = "Keep the correction queued.\n中文纠正：保留原指令";
    let (_, accepted) = store.steer_active_turn(&fixture.host.options.conversation_id.clone().into(),
        "pending-steering".into(), "session_api".into(), StrictJsonValue(json!({"content":queued_text}))).await.unwrap();
    let receipt = accepted.record.unwrap().event_id.as_ref().to_owned();
    let delivery = nomifun_ai_agent::RuntimeSteerDelivery {
        receipt_operation_id: receipt.clone(), wire_turn_id: fixture.message.msg_id.clone(),
        turn_generation: generation, text: queued_text.into(), files: vec![], inject_skills: vec![],
    };
    assert!(fixture.host.queue_steer(delivery.clone()).await.unwrap());
    assert!(fixture.host.queue_steer(delivery.clone()).await.unwrap());
    let tail = "Partial response before cancellation.\n中文尾部：只写一次";
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::OutputTextDelta {
        step: 1, text: tail.into(),
    }).await.unwrap();
    assert_eq!(fixture.host.active.lock().await.as_ref().unwrap().journal.sequence(), 2);
    let predicate = match fault {
        SteeringCleanupFault::PrivateRecord => "NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='output_text_delta'",
        SteeringCleanupFault::PublicProjection => "NEW.kind='message/content-part'",
        SteeringCleanupFault::DeferredReceipt => "NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='steering_deferred'",
    };
    sqlx::query(&format!("CREATE TRIGGER reject_steering_tail BEFORE INSERT ON agent_events WHEN {predicate} BEGIN SELECT RAISE(FAIL,'fixture steering tail write failure'); END"))
        .execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_steering_cleanup_pause BEFORE INSERT ON agent_events WHEN NEW.kind='turn/paused' BEGIN SELECT RAISE(FAIL,'fixture steering pause write failure'); END")
        .execute(fixture.pool()).await.unwrap();
    let failure = fixture.host.cleanup_turn(&fixture.message).await.unwrap_err();
    assert!(failure.to_string().contains("fixture steering tail write failure"));
    assert!(!fixture.host.queue_steer(delivery).await.unwrap(), "cleanup must keep the inbox closed");
    let pause_failure = fixture.host.suspend_after_cleanup_failure(&fixture.message).await.unwrap_err();
    assert!(pause_failure.to_string().contains("fixture steering pause write failure"));
    assert_eq!(fixture.state().await, "running");
    {
        let active = fixture.host.active.lock().await;
        let turn = active.as_ref().unwrap();
        assert!(turn.cleanup_started);
        assert!(!turn.cleanup_proven);
        assert!(!turn.steering.permits_resource_dispatch());
        assert_eq!(turn.steering.pending_receipt_ids(), vec![receipt.clone()]);
    }
    let public_parts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='message/content-part'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(public_parts, i64::from(matches!(fault, SteeringCleanupFault::DeferredReceipt)));
    let private_parts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='output_text_delta'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(private_parts, i64::from(!matches!(fault, SteeringCleanupFault::PrivateRecord)));
    sqlx::query("DROP TRIGGER reject_steering_tail").execute(fixture.pool()).await.unwrap();
    sqlx::query("DROP TRIGGER reject_steering_cleanup_pause").execute(fixture.pool()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    {
        let active = fixture.host.active.lock().await;
        let turn = active.as_ref().unwrap();
        assert!(turn.cleanup_proven);
        assert!(!turn.steering.permits_resource_dispatch());
        assert!(turn.steering.pending_receipt_ids().is_empty());
    }
    let parts: Vec<String> = sqlx::query_scalar("SELECT json_extract(inline_json,'$.content') FROM agent_events WHERE session_id=? AND kind='message/content-part' ORDER BY seq")
        .bind(&fixture.host.options.conversation_id).fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(parts, vec![tail]);
    let private: Vec<String> = sqlx::query_scalar("SELECT json_extract(inline_json,'$.event.text') FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='output_text_delta'")
        .bind(&fixture.host.options.conversation_id).fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(private, vec![tail]);
    let deferred: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='steering_deferred'")
        .bind(&fixture.host.options.conversation_id).fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(deferred.len(), 1);
    let deferred: Value = serde_json::from_str(&deferred[0]).unwrap();
    let inputs = deferred["event"]["inputs"].as_array().unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["receipt_operation_id"], receipt);
    assert_eq!(inputs[0]["message_id"], inputs[0]["receipt_operation_id"]);
    assert_eq!(inputs[0]["text"], queued_text);
    let recorded_input: AgentSteeringInput = serde_json::from_value(inputs[0].clone()).unwrap();
    assert!(recorded_input.files.is_empty());
    assert!(recorded_input.inject_skills.is_empty());
    let cleanup_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_cleanup_proven'")
        .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(cleanup_count, 1);
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.assert_settled().await;
    let before_repeat = store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    assert_eq!(store.current_cursor(&fixture.host.options.conversation_id.clone().into()).await.unwrap(), before_repeat);
    fixture.finish().await;
}

#[cfg(feature = "browser-use")]
mod browser_terminal_recovery {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use nomifun_browser_platform::{
        run_guard::{NativeInputGate, RunAdmissionError},
        runtime::{BrowserNativeSurfacePort, BrowserRuntime, BrowserRuntimeFactory, BrowserRuntimeSnapshot, BrowserTabCommand, CreateBrowserRuntime, WorkspaceError},
    };

    #[derive(Default)]
    struct FaultBrowserFactory { creates: AtomicUsize, closes: Arc<AtomicUsize>, poison_only_final_unlock: bool }
    struct FaultBrowser { generation: u64, poisoned: bool, poison_only_final_unlock: bool, closed: AtomicBool, closes: Arc<AtomicUsize>, locked: AtomicBool }
    #[async_trait]
    impl BrowserRuntimeFactory for FaultBrowserFactory {
        async fn create(&self, request: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
            Ok(Arc::new(FaultBrowser { generation: request.runtime_generation,
                poisoned: self.creates.fetch_add(1, Ordering::SeqCst) == 0, poison_only_final_unlock: self.poison_only_final_unlock, closed: AtomicBool::new(false),
                closes: self.closes.clone(), locked: AtomicBool::new(!request.user_input_enabled) }))
        }
    }
    #[async_trait]
    impl NativeInputGate for FaultBrowser {
        async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
            self.locked.store(true, Ordering::SeqCst);
            if self.poisoned && !self.poison_only_final_unlock { Err(RunAdmissionError::InputGateFailed) } else { Ok(()) }
        }
        async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
        async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
            if self.poisoned { return Err(RunAdmissionError::InputGateFailed); }
            self.locked.store(false, Ordering::SeqCst); Ok(())
        }
    }
    #[async_trait]
    impl BrowserRuntime for FaultBrowser {
        fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { None }
        async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
            Ok(BrowserRuntimeSnapshot { runtime_generation: self.generation, revision: 1, active_tab_id: None, tabs: vec![], downloads: vec![] })
        }
        async fn execute(&self, _: BrowserTabCommand, _: CancellationToken) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
            if self.poisoned && !self.poison_only_final_unlock { Err(WorkspaceError::NativeCommandFailed) } else { self.snapshot().await }
        }
        async fn close(&self) -> Result<(), WorkspaceError> {
            if !self.closed.swap(true, Ordering::SeqCst) { self.closes.fetch_add(1, Ordering::SeqCst); }
            Ok(())
        }
    }

    async fn model_fixture(expected_calls: usize) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 4096];
                let header_end = loop {
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert!(read > 0); bytes.extend_from_slice(&chunk[..read]);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") { break index + 4; }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length = headers.lines().filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse::<usize>().unwrap()).unwrap_or(0);
                while bytes.len() < header_end + length {
                    let read = socket.read(&mut chunk).await.unwrap(); assert!(read > 0); bytes.extend_from_slice(&chunk[..read]);
                }
                if headers.starts_with("GET ") {
                    let body = r#"{"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}"#;
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    socket.write_all(response.as_bytes()).await.unwrap(); continue;
                }
                counted.fetch_add(1, Ordering::SeqCst);
                let body = concat!(
                    "data: {\"id\":\"browser-recovery\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"cleanup-fixture\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"The successor completed after Browser reopen.\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"id\":\"browser-recovery\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"cleanup-fixture\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n");
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
                if counted.load(Ordering::SeqCst) == expected_calls { break; }
            }
        });
        (url, calls, task)
    }

    #[tokio::test]
    async fn published_failed_turn_retries_native_finish_after_user_reopen_and_the_sdk_successor_completes() {
        recovery_scenario(false).await;
    }

    #[tokio::test]
    async fn a_valid_native_guard_with_committed_terminal_can_reopen_after_failed_unlock_and_run_a_successor() {
        recovery_scenario(true).await;
    }

    async fn recovery_scenario(poison_only_final_unlock: bool) {
        let factory = Arc::new(FaultBrowserFactory { poison_only_final_unlock, ..Default::default() });
        let (model, calls, model_task) = model_fixture(if poison_only_final_unlock { 2 } else { 1 }).await;
        let fixture = Fixture::build_delivery("browser-native-failure-reopen-successor", vec![], vec![], None, false,None,
            Some((factory.clone(), model))).await;
        let session: AgentSessionId = fixture.host.options.conversation_id.clone().into();
        let store = fixture.owner.canonical().store();
        let binding_before = store.get_live_session(&session).await.unwrap().agent_binding;
        assert!(fixture.host.resources.compiled().resolved_capability(&CapabilityId::from("browser")).is_none(), "ordinary Chat has no Browser Tool authority");
        fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            while fixture.runtime.is_transport_healthy() { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(fixture.state().await, if poison_only_final_unlock { "completed" } else { "failed" });
        assert!(fixture.host.active.lock().await.is_none());
        assert!(fixture.host.terminal_already_recorded(fixture.host.root(&fixture.message)).unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(poison_only_final_unlock));
        let original = store.read_turn_receipt(&session, &"cleanup-retry-turn".into()).await.unwrap();
        let original_event = original.terminal_event.unwrap();
        let cursor = store.current_cursor(&session).await.unwrap();
        let resources = fixture.services.browser_resources.as_ref().unwrap();
        let user = resources.get_for_agent_session(fixture.services.authoritative_user_id.as_ref(), session.as_ref()).await.unwrap().unwrap();
        let generation = user.runtime_generation();
        let response = fixture._router.clone().oneshot(Request::builder().method("DELETE")
            .uri(format!("/api/agent-sessions/{}/browser", session.as_ref()))
            .header("x-nomi-local-trust", "cleanup-fixture-local-trust").header("content-type", "application/json")
            .body(Body::from(json!({"runtime_generation":generation}).to_string())).unwrap()).await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
        api(&fixture._router, &format!("/api/agent-sessions/{}/browser", session.as_ref()), json!({})).await;
        api(&fixture._router, &format!("/api/agent-sessions/{}/browser/commands", session.as_ref()),
            json!({"command":"create","url":"http://127.0.0.1/browser-fixture"})).await;
        assert_eq!(factory.creates.load(Ordering::SeqCst), 2);
        // Strict retry rejects altered terminal outcome and altered delivery.
        assert!(fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled { model_steps: 0 }).await.is_err());
        let mut altered = fixture.message.clone(); altered.content.push_str(" changed");
        let facts = store.chat_causality_facts(&session, &"cleanup-retry-turn".into()).await.unwrap();
        let terminal_name = if poison_only_final_unlock { "turn_completed" } else { "turn_failed" };
        let recorded: AgentEngineEvent = facts.event_payloads.values().filter_map(|payload| payload.get("event"))
            .find(|event| event.get("event").and_then(Value::as_str) == Some(terminal_name))
            .cloned().map(|value| serde_json::from_value(value).unwrap()).unwrap();
        assert!(fixture.host.record_event(&altered, &recorded).await.is_err());
        assert_eq!(store.current_cursor(&session).await.unwrap(), cursor);
        // SDK teardown retries the already published failure and only finishes
        // the closed native owner; it must not append another terminal.
        fixture.runtime.kill_and_wait(None).await.unwrap();
        assert_eq!(store.current_cursor(&session).await.unwrap(), cursor);
        assert_eq!(store.read_turn_receipt(&session, &"cleanup-retry-turn".into()).await.unwrap().terminal_event.unwrap().event_id, original_event.event_id);
        let (input, accepted) = store.start_turn(&session, "session_api".into(), "browser-recovery-successor".into(),
            "browser-recovery-successor".into(), StrictJsonValue(json!({"content":"complete the successor", "admission":{
                "route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref}}))).await.unwrap();
        let generation = accepted.record.unwrap().seq;
        let runtime = fixture.services.agent_runtime_sessions.get_or_create_runtime_for_turn(session.as_ref(), generation,
            CancellationToken::new(), fixture.registry_options()).await.unwrap();
        let message = SendMessageData { content: "complete the successor".into(), msg_id: "browser-recovery-successor-wire".into(),
            source_message_id: Some(input.record.unwrap().event_id.as_ref().into()), files: vec![], inject_skills: vec![], origin: None };
        let successor_host = HOSTS.get().unwrap().lock().unwrap().remove(session.as_ref()).unwrap().upgrade().unwrap();
        assert!(fixture.host.resources.teardown_proven().unwrap());
        assert!(!Arc::ptr_eq(&fixture.host.resources, &successor_host.resources),
            "a still-referenced closed resource context cannot be reused by replacement");
        fixture.host.cleanup_session().await.unwrap();
        let admitted = successor_host.session_host.read_turn_receipt(&successor_host.options, &successor_host.binding,
            &successor_host.snapshot_ref, &message).await.unwrap();
        assert!(Arc::ptr_eq(&successor_host.resources, &successor_host.session_host.open_kernel_session(admitted.session()).unwrap()),
            "late old teardown cannot evict the successor's exact resource context");
        runtime.send_message(message).await.unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let receipt = store.read_turn_receipt(&session, &"browser-recovery-successor".into()).await.unwrap();
                if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed { break; }
                assert_eq!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Running, "unexpected successor terminal: {receipt:?}");
                assert!(runtime.is_transport_healthy());
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), if poison_only_final_unlock { 2 } else { 1 });
        assert_eq!(store.get_live_session(&session).await.unwrap().agent_binding, binding_before);
        assert_eq!(store.read_turn_receipt(&session, &"cleanup-retry-turn".into()).await.unwrap().terminal_event.unwrap().event_id, original_event.event_id);
        runtime.kill_and_wait(None).await.unwrap();
        model_task.await.unwrap();
        fixture.services.shutdown_nomi_core_host().await.unwrap();
    }
}
