//! Cleanup faults on the host created by the installed official factory.
use super::*;
use axum::{body::Body, http::Request};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, Weak};
use std::time::Duration;
use tower::ServiceExt;

#[path = "engine_knowledge_writeback_tests.rs"]
mod knowledge_writeback_tests;

#[path = "companion_directed_flow_tests.rs"]
mod companion_directed_flow_tests;

#[path = "robot_authored_execution_tests.rs"]
mod robot_authored_execution_tests;

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
    api_state: super::super::nomi_core_session::NomiCoreAgentApiState,
    voice_control:Arc<nomifun_agent_control_plane::AgentControlPlane>,
    voice_execution:Arc<nomifun_agent_execution::AgentExecutionEngine>,
    message: SendMessageData,
    database_path: PathBuf,
    knowledge_base: Option<(nomifun_common::KnowledgeBaseId, PathBuf)>,
}

impl Fixture {
    async fn new(scenario: &str) -> Self {
        Self::build_delivery(scenario, vec![], vec![], None, false,None,false,
            #[cfg(feature = "browser-use")] None,
        ).await
    }

    async fn with_delivery(scenario: &str, files: Vec<String>, inject_skills: Vec<String>, origin: Option<String>) -> Self {
        Self::build_delivery(scenario, files, inject_skills, origin, true,None,false,
            #[cfg(feature = "browser-use")] None,
        ).await
    }

    async fn with_knowledge(scenario: &str, model_base: &str) -> Self {
        Self::build_delivery(scenario, vec![], vec![], None, false,Some(model_base),true,
            #[cfg(feature = "browser-use")] None,
        ).await
    }

    async fn with_input_model(scenario: &str, model_base: &str) -> Self {
        Self::build_delivery(scenario, vec![], vec![], None, false, Some(model_base), false,
            #[cfg(feature = "browser-use")] None,
        ).await
    }

    async fn build_delivery(scenario: &str, files: Vec<String>, inject_skills: Vec<String>, origin: Option<String>, explicit_metadata: bool,model_base_override:Option<&str>, knowledge: bool,
        #[cfg(feature = "browser-use")] browser: Option<(Arc<dyn nomifun_browser_platform::runtime::BrowserRuntimeFactory>, String)>,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::env::var_os("NOMIFUN_RELIABILITY_EVIDENCE_DIR")
            .map(|root| PathBuf::from(root).join(scenario))
            .unwrap_or_else(|| std::fs::canonicalize(directory.path()).unwrap());
        let config = crate::AppConfig {
            data_dir: root.join("data"), work_dir: root.join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some("cleanup-fixture-local-trust".into()),
            ..Default::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        crate::config::load_or_create_storage_generation(&config.data_dir).unwrap();
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
        let api_state = states.nomi_core_agent_api.clone();
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
        if knowledge {
            draft["document"]["enabled_capabilities"].as_array_mut().unwrap().push(json!({
                "capability":{"id":"knowledge"}, "action_allowlist":["knowledge/search","knowledge/read","knowledge/write"]
            }));
        }
        api(&router, &format!("/api/agent-presets/{preset}/revisions"), json!({
            "expected_current_revision":draft["current_revision"], "draft":draft,
            "reason":"isolated cleanup fault regression"
        })).await;
        let knowledge_base = if knowledge {
            let base = services.knowledge_service.create_base("Writeback fixture", "", None, None).await.unwrap();
            Some((base.knowledge_base_id, PathBuf::from(base.root_path)))
        } else { None };
        let mut create_session = json!({
            "preset_id":preset, "model":model,
            "resource_selections":[{"resource_kind":"workspace", "resource_id":"default-workspace"}]
        });
        if let Some((base, _)) = &knowledge_base {
            create_session["resource_selections"].as_array_mut().unwrap().push(json!({"resource_kind":"knowledge_base", "resource_id":base}));
            create_session["knowledge_policy"] = json!({"writeback":true,"writeback_eagerness":"auto"});
        }
        let session = api(&router, "/api/agent-sessions", create_session).await;
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
            conversation_created_at: Some(projection.created_at),
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
        Self { _directory:directory, _router:router, runtime, services, host, owner,api_state,voice_control,voice_execution, message, database_path, knowledge_base }
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
    fixture.owner.clear_session_context(&owner, &id, "recovery-fixture-clear", "user_requested").await.unwrap();
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
    let fixture=Fixture::build_delivery("voice-lazy-recovery-original-key",vec![],vec![],None,false,Some(&model_base),false,#[cfg(feature="browser-use")]None).await;
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

async fn voice_running_decision_attempt_fixture(fixture:&Fixture)->(String,String,nomifun_db::AgentExecutionStepDetailRow) {
    use nomifun_db::{IAgentExecutionRepository,SqliteAgentExecutionRepository,CreateAgentExecutionParams,NewAgentExecutionParticipant,NewAgentExecutionEvent,ReconcileAgentExecutionPlanParams,
        NewAgentExecutionStep,CreateAgentExecutionAttemptParams,AgentExecutionAttemptSessionKind};
    use nomifun_common::{AgentExecutionStatus,AgentExecutionEventKind,AgentExecutionActor,ExecutionStepStatus};
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
        &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    (created.execution_id,step,running)
}

async fn voice_pending_decision_fixture(fixture:&Fixture)->nomifun_agent_contracts::AgentSessionId {
    use nomifun_db::{IAgentExecutionRepository,SqliteAgentExecutionRepository,NewAgentExecutionEvent,SettleAgentExecutionAttemptParams,AgentExecutionActiveTurnGuard,AttemptConversationEffects};
    use nomifun_common::{AgentExecutionStatus,AgentExecutionEventKind,AgentExecutionActor,ExecutionStepStatus,ExecutionAttemptStatus};
    let (execution,step,running)=voice_running_decision_attempt_fixture(fixture).await;
    let repository=SqliteAgentExecutionRepository::new(fixture.pool().clone());let owner=&fixture.host.options.user_id;let session=&fixture.host.options.conversation_id;
    let attempt=running.current_attempt.unwrap().attempt;let stop=nomifun_common::generate_id();
    let mut effects=AttemptConversationEffects::default();effects.push_stop_turn(stop.clone(),"cleanup-retry-turn".into()).unwrap();
    repository.settle_attempt(owner,&execution,&step,running.step.version,&attempt.attempt_id,attempt.version,None,&SettleAgentExecutionAttemptParams {
        expected_terminal_turn:None,
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
async fn voice_actual_normal_input_handoff_presents_the_exact_native_turn_and_answers_once() {
    use nomifun_db::{IAgentExecutionRepository,SqliteAgentExecutionRepository,NewAgentExecutionEvent,
        SettleAgentExecutionAttemptParams,AgentExecutionTerminalTurnGuard,AttemptConversationEffects};
    use nomifun_common::{AgentExecutionStatus,AgentExecutionEventKind,AgentExecutionActor,ExecutionStepStatus,ExecutionAttemptStatus};
    use nomifun_voice_contracts::{VoiceApprovalAnswer,VoiceWorkStatus};

    let server=InputHandoffModelServer::new().await;
    let fixture=Fixture::with_input_model("voice-normal-input-handoff",&server.url).await;
    let (execution,step,running)=voice_running_decision_attempt_fixture(&fixture).await;
    let attempt=running.current_attempt.unwrap().attempt;
    let owner=&fixture.host.options.user_id;let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let store=fixture.owner.canonical().store();
    // The default fixture root was never dispatched. The real Attempt uses
    // its durable initial-input identity, which the WaitingInput writer checks.
    let (_,cancelled)=store.cancel_exact_turn(&session,&"cleanup-retry-turn".into(),
        "voice-handoff-fixture-root-cancel".into(),"session_api".into()).await.unwrap();
    let other_terminal=cancelled.record.unwrap().event_id;
    let key=AttemptConversationEffects::default().current_turn_operation_key(&attempt.attempt_id).unwrap();
    let operation:OperationId=format!("turn:user:{owner}:{}:{key}",session.as_ref()).into();
    let content="Prepare the report and ask for its destination.";
    let (input,_)=store.start_turn(&session,"session_api".into(),key.into(),operation.clone(),StrictJsonValue(json!({
        "content":content,"admission":{"route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref}
    }))).await.unwrap();
    let message=SendMessageData {content:content.into(),msg_id:"voice-normal-input-handoff-wire".into(),
        source_message_id:Some(input.record.unwrap().event_id.as_ref().into()),files:vec![],inject_skills:vec![],origin:None};
    let events=fixture.runtime.subscribe();fixture.runtime.send_message(message).await.unwrap();
    let received=receive_input_handoff_terminal(events).await;
    assert!(received.iter().any(|event|matches!(event,nomifun_ai_agent::AgentStreamEvent::Finish(data)
        if data.stop_reason==Some(nomifun_ai_agent::protocol::events::TurnStopReason::InputRequired))));
    assert_eq!(server.requests.lock().unwrap().len(),1,"the native handoff closes before another model request");
    let receipt=store.read_turn_receipt(&session,&operation).await.unwrap();
    assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Completed);
    let terminal=receipt.terminal_event.as_ref().unwrap();
    let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload)=&terminal.payload else{panic!("inline native terminal expected");};
    let request:nomifun_agent_contracts::AgentUserInputRequest=serde_json::from_value(payload.0["user_input_request"].clone()).unwrap();
    assert_eq!(request.request_id.as_ref(),"ask-report-path");
    assert_eq!(request.question,"Where should the report be saved?");
    let source=store.turn_output_facts(&session,&operation).await.unwrap();
    let delivery=nomifun_agent_execution::canonical_turn_delivery(&source,&receipt,true).unwrap();
    assert_eq!(delivery.user_input_request.as_ref(),Some(&request));
    assert!(source.execution_generation>0,"the presentation must preserve a real native generation");

    let repository=SqliteAgentExecutionRepository::new(fixture.pool().clone());
    let guard=AgentExecutionTerminalTurnGuard::InputRequired {conversation_id:session.as_ref().into(),canonical_operation_id:operation.as_ref().into(),
        terminal_event_id:terminal.event_id.as_ref().into(),user_input_request:request.clone()};
    let params=|guard|SettleAgentExecutionAttemptParams {expected_active_session_turn:None,expected_terminal_turn:Some(guard),
        attempt_status:ExecutionAttemptStatus::WaitingInput,step_status:ExecutionStepStatus::WaitingInput,execution_status:Some(AgentExecutionStatus::WaitingInput),
        question:Some(Some(request.question.clone())),error:Some(None),output_summary:None,output_files:None,tokens:None,retry_after:None,
        runtime_state:Some(None),started_at:None,finished_at:None,loop_repeat_reset:None};
    let payload=json!({"canonical_operation_id":operation,"terminal_event_id":terminal.event_id,"question":request.question,"user_input_request":request});
    let event=|payload:Value|NewAgentExecutionEvent {event_type:AgentExecutionEventKind::DecisionRequested,step_id:Some(step.clone()),
        attempt_id:Some(attempt.attempt_id.clone()),actor:AgentExecutionActor::system(),payload:payload.to_string()};
    let mut wrong_guard=guard.clone();if let AgentExecutionTerminalTurnGuard::InputRequired {terminal_event_id,..}=&mut wrong_guard {*terminal_event_id=other_terminal.as_ref().into();}
    assert!(repository.settle_attempt(owner,&execution,&step,running.step.version,&attempt.attempt_id,attempt.version,None,
        &params(wrong_guard),&event(payload.clone())).await.is_err(),"a different canonical terminal cannot settle this handoff");
    let mut changed_guard=guard.clone();if let AgentExecutionTerminalTurnGuard::InputRequired {user_input_request,..}=&mut changed_guard {user_input_request.question="Use a changed question?".into();}
    assert!(repository.settle_attempt(owner,&execution,&step,running.step.version,&attempt.attempt_id,attempt.version,None,
        &params(changed_guard),&event(payload.clone())).await.is_err(),"the canonical question cannot be replaced during settlement");

    // Keep the real canonical source fixed while presenting stale Execution
    // request metadata. The Host must verify that metadata against that source.
    let mut wrong_terminal=payload.clone();wrong_terminal["terminal_event_id"]=json!(other_terminal);
    repository.settle_attempt(owner,&execution,&step,running.step.version,&attempt.attempt_id,attempt.version,None,
        &params(guard),&event(wrong_terminal)).await.unwrap();
    let waiting=fixture.voice_execution.get(owner,&execution).await.unwrap();
    assert_eq!(waiting.execution.status,AgentExecutionStatus::WaitingInput);
    assert_eq!(waiting.attempts.iter().find(|current|current.attempt_id==attempt.attempt_id).unwrap().status,ExecutionAttemptStatus::WaitingInput);
    let port=voice_port(&fixture);let binding=source.session.agent_binding.binding_version;
    assert!(port.current_decision_presentations(owner,session.as_ref(),binding).await.unwrap().is_empty(),
        "a WaitingInput event cannot borrow another terminal in the same Session");
    let mut changed_question=payload.clone();changed_question["user_input_request"]["question"]=json!("Use a changed question?");
    let version=fixture.voice_execution.get(owner,&execution).await.unwrap().execution.version;
    repository.append_event(owner,&execution,version,&event(changed_question)).await.unwrap();
    assert!(port.current_decision_presentations(owner,session.as_ref(),binding).await.unwrap().is_empty(),
        "a changed typed question must not become a presentation");
    let version=fixture.voice_execution.get(owner,&execution).await.unwrap().execution.version;
    repository.append_event(owner,&execution,version,&event(payload)).await.unwrap();
    let presentations=port.current_decision_presentations(owner,session.as_ref(),binding).await.unwrap();
    assert_eq!(presentations.len(),1);let presentation=&presentations[0];let target=&presentation.target;
    assert_eq!(presentation.question,request.question);
    assert_eq!(target.work_target.agent_session_id,session);assert_eq!(target.work_target.turn_operation_id,operation);
    assert_eq!(target.work_target.binding_version,binding);assert_eq!(target.work_target.execution_generation,source.execution_generation);
    assert_eq!(target.interaction_agent_session_id,session);assert_eq!(target.action_id.as_ref(),"agent/request_user_decision");
    assert_eq!(target.question_digest,nomifun_agent_contracts::digest_bytes(request.question.as_bytes()));
    assert_eq!(target.execution_id,execution);assert_eq!(target.step_id,step);assert_eq!(target.attempt_id,attempt.attempt_id);
    let events=fixture.voice_execution.events(owner,&execution,None,Some(128)).await.unwrap();
    let requested=events.iter().find(|event|event.sequence==target.request_event_sequence).unwrap();
    assert!(requested.payload.get("stop_turn_operation_id").is_none(),"this is the ordinary completed input handoff path");
    assert_eq!(requested.payload["terminal_event_id"],json!(terminal.event_id));
    let answer=VoiceApprovalAnswer {target:target.clone(),answer:"Save it in reports/.".into(),presented_context_id:target.presentation_id.clone()};
    let mut altered=answer.clone();altered.target.question_digest=nomifun_agent_contracts::digest_bytes(b"a different question");
    assert!(port.answer_decision_presentation(owner,"normal-handoff-altered-question",altered).await.is_err());
    // The fixture stops the scheduler. This proves the original answer CAS
    // and durable continuation intent, rather than another model dispatch.
    let first=port.answer_decision_presentation(owner,"normal-handoff-answer",answer.clone()).await.unwrap();
    assert_eq!(first.status,VoiceWorkStatus::Applied);assert!(!first.duplicate);
    let resumed=fixture.voice_execution.get(owner,&execution).await.unwrap();
    assert_eq!(resumed.execution.status,AgentExecutionStatus::Running);
    assert_eq!(resumed.attempts.iter().find(|current|current.attempt_id==attempt.attempt_id).unwrap().status,ExecutionAttemptStatus::Running);
    let replay=port.answer_decision_presentation(owner,"normal-handoff-answer",answer.clone()).await.unwrap();
    assert!(replay.duplicate);assert_eq!(replay.receipt_id,first.receipt_id);
    assert!(port.answer_decision_presentation(owner,"normal-handoff-late-answer",answer.clone()).await.is_err(),
        "the original pending presentation cannot be answered twice under another key");
    let mut different=answer;different.answer="Save it somewhere else.".into();
    assert!(port.answer_decision_presentation(owner,"normal-handoff-answer",different).await.is_err());
    assert!(port.current_decision_presentations(owner,session.as_ref(),binding).await.unwrap().is_empty());
    let events=fixture.voice_execution.events(owner,&execution,None,Some(128)).await.unwrap();
    let answered=events.iter().filter(|event|event.event_type==AgentExecutionEventKind::DecisionAnswered).collect::<Vec<_>>();
    assert_eq!(answered.len(),1);assert_eq!(answered[0].payload["request_event_sequence"],json!(target.request_event_sequence));
    assert_eq!(answered[0].payload["voice_source_target"],json!(target.work_target));
    assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().terminal_event.unwrap().event_id,terminal.event_id,
        "answering resumes the Attempt; it cannot reopen or replace the completed native Turn");
    fixture.finish().await;server.close().await;
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
    assert!(prepared.preactivated_tools.is_empty(), "a workspace-only Agent must not receive application schemas");
    fixture.finish().await;
}

#[test]
fn selected_application_tools_preserve_the_exact_frozen_active_action_subset() {
    let binding = |name: &str, capability: &str, action: &str| {
        let schema = StrictJsonValue(json!({"type":"object","additionalProperties":false,"properties":{}}));
        AgentToolBinding {
            model_name: name.to_owned(), definition: ChatToolDefinition { name: name.to_owned(),
                description: "isolated presentation-policy fixture".into(), input_schema: schema.clone(), deferred: true },
            schema_digest: nomifun_engine_core::input_schema_digest(&schema).unwrap(),
            canonical_input_schema_ref: "schema://nomifun/presentation-policy-fixture@1".into(),
            capability_contract_digest: "a".repeat(64).into(), capability_id: capability.into(), action_id: action.into(),
            resource_binding_ids: BTreeSet::new(), effect_class: AgentEffectClass::ReadOnly, parallel_safe: true,
        }
    };
    let plan = AgentToolPlan::new([
        binding("owned_current", "app.hosting", "app.hosting/current"),
        binding("owned_update", "app.hosting", "app.hosting/update"),
        binding("foreign_update", "another.module", "app.hosting/update"),
    ]).unwrap();
    let before = plan.model_definitions();
    assert_eq!(selected_application_tools(&plan), ["owned_current", "owned_update"]);
    assert_eq!(selected_application_tools(&plan), ["owned_current", "owned_update"], "a subsequent turn restores the same selected schemas without discovery history");
    assert_eq!(plan.model_definitions(), before, "preactivation cannot author new bindings or replace a frozen schema");
    assert!(plan.model_name_for_action("app.hosting", "app.hosting/preview").is_none(), "the policy cannot manufacture an ungranted preview action");
    assert!(selected_application_tools(&AgentToolPlan::default()).is_empty(), "inactive or unselected application modules supply no tools");
}

#[test]
fn robot_interaction_context_uses_only_the_frozen_active_tool_mapping() {
    let binding = |name: &str, action: &str, deferred: bool| {
        let schema = StrictJsonValue(json!({"type":"object","additionalProperties":false,"properties":{}}));
        AgentToolBinding {
            model_name: name.into(), definition: ChatToolDefinition {
                name: name.into(), description: "bound device tool".into(), input_schema: schema.clone(), deferred,
            },
            schema_digest: nomifun_engine_core::input_schema_digest(&schema).unwrap(),
            canonical_input_schema_ref: "schema://nomifun/robot-interaction-fixture@1".into(),
            capability_contract_digest: "a".repeat(64).into(), capability_id: "robot".into(), action_id: action.into(),
            resource_binding_ids: BTreeSet::from(["exact-owned-robot".into()]),
            effect_class: AgentEffectClass::ExternalUncertainEffect, parallel_safe: false,
        }
    };
    let active = AgentToolPlan::new([
        binding("current_motion", "robot/motion", false),
        binding("deferred_extension", "robot/device", true),
    ]).unwrap();
    let context = robot_interaction_context(&active).unwrap();
    let data = context.split_once(": ").unwrap().1.split_once(". Use ").unwrap().0;
    assert_eq!(serde_json::from_str::<Value>(data).unwrap(), json!([
        {"name":"current_motion","action_id":"robot/motion","schema_visible":true},
        {"name":"deferred_extension","action_id":"robot/device","schema_visible":false},
    ]));
    assert!(context.contains("not proof of completed movement"));
    let absent = robot_interaction_context(&AgentToolPlan::default()).unwrap();
    assert!(absent.contains("[]"));
    assert!(absent.contains("no callable Robot tools"));
    assert!(!absent.contains("current_motion"), "missing binding or revoked active tools cannot be resurrected by context");
    assert!(selected_application_tools(&active).is_empty(), "robot presentation needs no all-tool preactivation");
}

#[test]
fn robot_profile_preserves_explicit_reasoning_and_does_not_guess_a_supported_low_tier() {
    assert!(session_reasoning(None).is_none(), "reasoning-capable routes do not advertise which default tier is supported");
    for (selected, expected) in [
        (SessionReasoningEffortDto::None, ReasoningEffort::None),
        (SessionReasoningEffortDto::Minimal, ReasoningEffort::Minimal),
        (SessionReasoningEffortDto::Low, ReasoningEffort::Low),
        (SessionReasoningEffortDto::Medium, ReasoningEffort::Medium),
        (SessionReasoningEffortDto::High, ReasoningEffort::High),
        (SessionReasoningEffortDto::XHigh, ReasoningEffort::XHigh),
        (SessionReasoningEffortDto::Max, ReasoningEffort::Max),
        (SessionReasoningEffortDto::Ultra, ReasoningEffort::Ultra),
    ] {
        let request = session_reasoning(Some(selected)).unwrap();
        assert_eq!(request.effort, Some(expected));
        assert_eq!(request.summary, ReasoningSummary::None);
        assert!(request.max_reasoning_tokens.is_none());
    }
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
            hidden:false,origin:Some("agent_execution".into()),channel_platform:None,
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
        origin:Some("agent_execution".into()),channel_platform:None,
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
async fn system_permission_result_sequence_uses_the_real_host_ack_after_buffer_flush() {
    use nomifun_agent_runtime::AgentToolResult;
    use nomifun_chat_model_broker::ChatToolCall;
    let fixture = Fixture::new("permission-result-sequence").await;
    let host = fixture.host.clone();
    let mut message = fixture.message.clone();
    message.msg_id = message.source_message_id.clone().unwrap();
    host.admit_preparation(&message, CancellationToken::new()).await.unwrap();
    assert!(host.record_event(&message, &AgentEngineEvent::TurnStarted {
        binding: host.engine_binding.clone(), turn_operation_id: "cleanup-retry-turn".into(),
    }).await.unwrap().is_none());
    for (index, is_error) in [true, false].into_iter().enumerate() {
        let call_id = format!("permission-sequence-{index}");
        host.record_event(&message, &AgentEngineEvent::ToolCallCompleted { step: 1, call: ChatToolCall {
            call_id: call_id.clone().into(), name: "read_file".into(),
            arguments: StrictJsonValue(json!({"path":"README.md"})), provider_metadata: None,
        }}).await.unwrap();
        assert!(host.admit_tool(&message, &AgentEngineEvent::ToolStarted {
            step: 1, call_id: call_id.clone().into(), capability_id: "workspace.files".into(),
            action_id: "workspace.files/read".into(),
        }).await.unwrap());
        // Short text stays buffered until the result commits both records.
        assert!(host.record_event(&message, &AgentEngineEvent::OutputTextDelta {
            step: 1, text: format!("buffered before {call_id}"),
        }).await.unwrap().is_none());
        let seq = host.record_event(&message, &AgentEngineEvent::ToolCompleted { runtime_hold: None, step: 1,
            result: AgentToolResult::text(call_id.clone().into(), "owner observation", is_error),
        }).await.unwrap().expect("actual canonical ToolCompleted acknowledgement");
        let persisted: i64 = sqlx::query_scalar("SELECT seq FROM agent_events WHERE session_id=? \
            AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_completed' \
            AND json_extract(inline_json,'$.event.result.call_id')=?")
            .bind(&host.options.conversation_id).bind(&call_id).fetch_one(fixture.pool()).await.unwrap();
        assert_eq!(seq, persisted as u64);
        let text_seq: i64 = sqlx::query_scalar("SELECT MAX(seq) FROM agent_events WHERE session_id=? \
            AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='output_text_delta'")
            .bind(&host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
        assert!(text_seq < persisted, "result identity must not borrow the flushed text acknowledgement");
        assert!(host.record_event(&message, &AgentEngineEvent::ReasoningDelta {
            step: index as u16 + 2, text: "a later canonical observation".into(),
        }).await.unwrap().is_none());
        let head = fixture.owner.canonical().store().head(&host.options.conversation_id.clone().into()).await.unwrap();
        assert!(head.last_seq > seq, "an unrelated later head is distinct from the result receipt");
    }
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
async fn parked_host_receives_input_before_and_during_notification_registration_without_closing_the_turn() {
    for before in [true,false] {
        let fixture=Fixture::new(if before {"wait-input-before"} else {"wait-input-race"}).await;
        let cancellation=CancellationToken::new();
        let request=fixture.host.prepare_turn(&fixture.message,cancellation.clone()).await.unwrap();
        let causality=request.model_request.causality.clone();
        fixture.host.record_event(&fixture.message,&AgentEngineEvent::TurnStarted {
            binding:fixture.host.engine_binding.clone(),turn_operation_id:causality.turn_operation_id.clone(),
        }).await.unwrap();
        fixture.host.record_event(&fixture.message,&AgentEngineEvent::TurnWaiting {model_steps:0,request_id:None,
            reason:AgentWaitReason::InterventionRequired}).await.unwrap();
        let generation=fixture.host.active.lock().await.as_ref().unwrap().epoch as u64;
        let store=fixture.host.session_host.canonical_store().unwrap();
        let queue=async {
            tokio::task::yield_now().await;
            let (_,receipt)=store.steer_active_turn(&fixture.host.options.conversation_id.clone().into(),
                "owner-wait-answer".into(),"session_api".into(),StrictJsonValue(json!({"content":"Authorization completed"}))).await.unwrap();
            assert!(fixture.host.queue_steer(nomifun_ai_agent::RuntimeSteerDelivery {
                receipt_operation_id:receipt.ack.unwrap().event_id.as_ref().to_owned(),wire_turn_id:fixture.message.msg_id.clone(),
                turn_generation:generation,text:"Authorization completed".into(),files:vec![],inject_skills:vec![],
            }).await.unwrap());
        };
        let wait_port=request.wait_port.unwrap();let wait_request=AgentWaitRequest {processes:vec![]};
        let wake=if before {
            queue.await;
            tokio::time::timeout(Duration::from_secs(2),wait_port.wait(&causality,&wait_request,cancellation.clone())).await.unwrap().unwrap()
        } else {
            let (wake,())=tokio::join!(tokio::time::timeout(Duration::from_secs(2),wait_port.wait(&causality,&wait_request,cancellation.clone())),queue);
            wake.unwrap().unwrap()
        };
        assert!(matches!(wake,AgentWaitWake::InputsAvailable));
        let input=request.input_port.unwrap().take(&causality,false).await.unwrap();
        assert_eq!(input.len(),1);assert_eq!(input[0].text,"Authorization completed");
        assert_eq!(fixture.state().await,"running");
        assert!(!fixture.host.active.lock().await.as_ref().unwrap().cleanup_started);
        if before {
            fixture.host.record_event(&fixture.message,&AgentEngineEvent::TurnWaiting {model_steps:0,request_id:None,
                reason:AgentWaitReason::ExternalResource}).await.unwrap();
            let missing=AgentWaitRequest {processes:vec![AgentWaitProcess {process_id:"another-turn-process".into(),output_cursor:0}]};
            let lost=tokio::time::timeout(Duration::from_secs(2),wait_port.wait(&causality,&missing,cancellation.clone())).await.unwrap();
            assert!(matches!(lost,Err(AgentEngineError::WaitOwnerUnavailable)));
            fixture.host.cleanup_turn(&fixture.message).await.unwrap();
            fixture.host.record_event(&fixture.message,&AgentEngineEvent::TurnPaused {model_steps:0,
                reason:"EXECUTION_WAIT_OWNER_UNAVAILABLE".into()}).await.unwrap();
            assert_eq!(fixture.state().await,"running");
            let head=store.head(&fixture.host.options.conversation_id.clone().into()).await.unwrap();
            assert_eq!(head.status,"paused");
            let failures:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/failed'")
                .bind(&fixture.host.options.conversation_id).fetch_one(fixture.pool()).await.unwrap();
            assert_eq!(failures,0,"lost owner observation is reconciliation, not task failure");
            store.cancel_active_turn(&fixture.host.options.conversation_id.clone().into(),"end-owner-loss".into(),"session_api".into()).await.unwrap();
            fixture.assert_settled().await;fixture.finish().await;continue;
        }
        fixture.host.record_event(&fixture.message,&AgentEngineEvent::TurnWaitResolved {model_steps:0}).await.unwrap();
        fixture.host.cleanup_turn(&fixture.message).await.unwrap();
        fixture.host.record_event(&fixture.message,&AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
        fixture.assert_settled().await;fixture.finish().await;
    }
}

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
mod unused_browser_admission {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use nomifun_browser_platform::{
        run_guard::{NativeInputGate, RunAdmissionError},
        runtime::{BrowserNativeSurfacePort, BrowserRuntime, BrowserRuntimeFactory, BrowserRuntimeSnapshot, BrowserTabCommand, CreateBrowserRuntime, WorkspaceError},
    };

    #[derive(Default)]
    struct GateCalls { locks: AtomicUsize, releases: AtomicUsize, unlocks: AtomicUsize }
    #[derive(Default)]
    struct FaultBrowserFactory { creates: AtomicUsize, closes: Arc<AtomicUsize>, gate: Arc<GateCalls> }
    struct FaultBrowser { generation: u64, closed: AtomicBool, closes: Arc<AtomicUsize>, gate: Arc<GateCalls> }
    #[async_trait]
    impl BrowserRuntimeFactory for FaultBrowserFactory {
        async fn create(&self, request: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
            assert!(request.user_input_enabled, "a user-created browser starts with user input");
            self.creates.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(FaultBrowser { generation: request.runtime_generation, closed: AtomicBool::new(false),
                closes: self.closes.clone(), gate: self.gate.clone() }))
        }
    }
    #[async_trait]
    impl NativeInputGate for FaultBrowser {
        async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
            self.gate.locks.fetch_add(1, Ordering::SeqCst);
            Err(RunAdmissionError::InputGateFailed)
        }
        async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
            self.gate.releases.fetch_add(1, Ordering::SeqCst);
            Err(RunAdmissionError::InputGateFailed)
        }
        async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
            self.gate.unlocks.fetch_add(1, Ordering::SeqCst);
            Err(RunAdmissionError::InputGateFailed)
        }
    }
    #[async_trait]
    impl BrowserRuntime for FaultBrowser {
        fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { None }
        async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
            Ok(BrowserRuntimeSnapshot { runtime_generation: self.generation, revision: 1, active_tab_id: None, tabs: vec![], downloads: vec![] })
        }
        async fn execute(&self, _: BrowserTabCommand, _: CancellationToken) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
            self.snapshot().await
        }
        async fn close(&self) -> Result<(), WorkspaceError> {
            if !self.closed.swap(true, Ordering::SeqCst) { self.closes.fetch_add(1, Ordering::SeqCst); }
            Ok(())
        }
    }

    struct PausedModel {
        url: String,
        started: Arc<tokio::sync::Notify>,
        resume: Arc<tokio::sync::Notify>,
        task: tokio::task::JoinHandle<()>,
    }

    impl PausedModel {
        async fn new() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/v1", listener.local_addr().unwrap());
            let started = Arc::new(tokio::sync::Notify::new());
            let resume = Arc::new(tokio::sync::Notify::new());
            let router = axum::Router::new()
                .route("/v1/models", axum::routing::get(|| async {
                    axum::Json(json!({"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}))
                }))
                .route("/v1/chat/completions", axum::routing::post(|
                    axum::extract::State((started, resume)):
                        axum::extract::State<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
                    axum::Json(_): axum::Json<Value>,
                | async move {
                    started.notify_one();
                    resume.notified().await;
                    let chunk = json!({"id":"unused-browser","object":"chat.completion.chunk","created":1,
                        "model":"cleanup-fixture","choices":[{"index":0,"delta":{"role":"assistant",
                            "content":"The file task completed."},"finish_reason":null}]});
                    let end = json!({"id":"unused-browser","object":"chat.completion.chunk","created":1,
                        "model":"cleanup-fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                    ([ (axum::http::header::CONTENT_TYPE, "text/event-stream") ],
                        format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
                }))
                .with_state((started.clone(), resume.clone()));
            let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
            Self { url, started, resume, task }
        }
    }

    #[tokio::test]
    async fn a_running_file_turn_keeps_the_unused_browser_available_despite_native_gate_failures() {
        let factory = Arc::new(FaultBrowserFactory::default());
        let model = PausedModel::new().await;
        let fixture = Fixture::build_delivery("unused-browser-native-gate-failure", vec![], vec![], None, false, None, false,
            Some((factory.clone(), model.url.clone()))).await;
        let session: AgentSessionId = fixture.host.options.conversation_id.clone().into();
        assert!(fixture.host.resources.compiled().resolved_capability(&CapabilityId::from("browser")).is_none(),
            "the file-only turn has no Browser Tool authority");
        let events = fixture.runtime.subscribe();
        fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(8), model.started.notified()).await
            .expect("an unused browser's native gate must not prevent model dispatch");
        assert_eq!(fixture.state().await, "running");
        assert_eq!(factory.gate.locks.load(Ordering::SeqCst), 0);
        assert_eq!(factory.gate.releases.load(Ordering::SeqCst), 0);
        assert_eq!(factory.gate.unlocks.load(Ordering::SeqCst), 0);
        let resources = fixture.services.browser_resources.as_ref().unwrap();
        let user = resources.get_for_agent_session(fixture.services.authoritative_user_id.as_ref(), session.as_ref())
            .await.unwrap().unwrap();
        api(&fixture._router, &format!("/api/agent-sessions/{}/browser/commands", session.as_ref()),
            json!({"command":"create","url":"http://127.0.0.1/user-page"})).await;
        let response = fixture._router.clone().oneshot(Request::builder().method("DELETE")
            .uri(format!("/api/agent-sessions/{}/browser", session.as_ref()))
            .header("x-nomi-local-trust", "cleanup-fixture-local-trust").header("content-type", "application/json")
            .body(Body::from(json!({"runtime_generation":user.runtime_generation()}).to_string())).unwrap()).await.unwrap();
        assert!(response.status().is_success(), "the running file turn must allow closing its unused browser");
        assert_eq!(factory.closes.load(Ordering::SeqCst), 1);
        assert_eq!(factory.gate.locks.load(Ordering::SeqCst), 1,
            "only the explicit user close attempts the native destruction gate");
        api(&fixture._router, &format!("/api/agent-sessions/{}/browser", session.as_ref()), json!({})).await;
        api(&fixture._router, &format!("/api/agent-sessions/{}/browser/commands", session.as_ref()),
            json!({"command":"create","url":"http://127.0.0.1/reopened-user-page"})).await;
        assert_eq!(factory.creates.load(Ordering::SeqCst), 2);
        assert_eq!(fixture.state().await, "running");
        model.resume.notify_one();
        receive_input_handoff_terminal(events).await;
        assert_eq!(fixture.state().await, "completed");
        assert!(fixture.runtime.is_transport_healthy());
        assert!(fixture.host.active.lock().await.is_none());
        assert_eq!(factory.gate.locks.load(Ordering::SeqCst), 1);
        assert_eq!(factory.gate.releases.load(Ordering::SeqCst), 0,
            "a file turn's cleanup does not drain the browser");
        assert_eq!(factory.gate.unlocks.load(Ordering::SeqCst), 0,
            "a file turn's terminal does not unlock the browser");
        fixture.finish().await;
        model.task.abort();
        let _ = model.task.await;
    }
}

struct InputHandoffModelServer {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl InputHandoffModelServer {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let router = axum::Router::new()
            .route("/v1/models", axum::routing::get(|| async {
                axum::Json(json!({"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}))
            }))
            .route("/v1/chat/completions", axum::routing::post(|
                axum::extract::State(requests): axum::extract::State<Arc<Mutex<Vec<Value>>>>,
                axum::Json(body): axum::Json<Value>,
            | async move {
                let call = {
                    let mut recorded = requests.lock().unwrap();
                    recorded.push(body.clone());
                    recorded.len()
                };
                let (delta, reason) = if call == 1 {
                    assert!(body["tools"].as_array().unwrap().iter().any(|tool|
                        tool.pointer("/function/name").and_then(Value::as_str) == Some("request_user_input")));
                    (json!({"role":"assistant","tool_calls":[{"index":0,"id":"ask-report-path","type":"function",
                        "function":{"name":"request_user_input","arguments":json!({
                            "question":"Where should the report be saved?", "remaining_work":"Save and verify the report."
                        }).to_string()}}]}), "tool_calls")
                } else {
                    (json!({"role":"assistant","content":"The follow-up was accepted in the same Session."}), "stop")
                };
                let chunk = json!({"id":format!("input-handoff-{call}"),"object":"chat.completion.chunk","created":1,
                    "model":"cleanup-fixture","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                let end = json!({"id":format!("input-handoff-{call}"),"object":"chat.completion.chunk","created":1,
                    "model":"cleanup-fixture","choices":[{"index":0,"delta":{},"finish_reason":reason}]});
                ([ (axum::http::header::CONTENT_TYPE, "text/event-stream") ],
                    format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
            }))
            .with_state(requests.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        Self { url, requests, task }
    }

    async fn close(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

async fn receive_input_handoff_terminal(mut events: tokio::sync::broadcast::Receiver<nomifun_ai_agent::AgentStreamEvent>)
    -> Vec<nomifun_ai_agent::AgentStreamEvent> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut received = Vec::new();
        loop {
            let event = events.recv().await.unwrap();
            if let nomifun_ai_agent::AgentStreamEvent::Error(error) = &event {
                panic!("installed Host input handoff failed: {error:?}");
            }
            let finished = matches!(event, nomifun_ai_agent::AgentStreamEvent::Finish(_));
            received.push(event);
            if finished { return received; }
        }
    }).await.expect("the input handoff must end without a second model request")
}

#[tokio::test]
async fn installed_host_input_handoff_reopens_as_normal_history_and_the_human_starts_a_new_turn() {
    use nomifun_ai_agent::{AgentStreamEvent, protocol::events::TurnStopReason};
    let server = InputHandoffModelServer::new().await;
    let fixture = Fixture::with_input_model("input-handoff-sdk-history", &server.url).await;
    let events = fixture.runtime.subscribe();
    fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
    let received = receive_input_handoff_terminal(events).await;
    let tip = received.iter().position(|event| matches!(event, AgentStreamEvent::Tips(data) if data.user_input_request.is_some())).unwrap();
    let finish = received.iter().position(|event| matches!(event, AgentStreamEvent::Finish(data) if data.stop_reason == Some(TurnStopReason::InputRequired))).unwrap();
    assert!(tip < finish);
    assert_eq!(server.requests.lock().unwrap().len(), 1, "asking does not auto-answer or continue the model");
    assert_eq!(fixture.runtime.status(), Some(nomifun_common::ConversationStatus::Finished));
    assert!(fixture.host.active.lock().await.is_none());
    let session = AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let store = fixture.owner.canonical().store();
    let original = store.read_turn_receipt(&session, &"cleanup-retry-turn".into()).await.unwrap();
    let terminal = original.terminal_event.unwrap();
    assert_eq!(terminal.kind.0, "turn/completed");
    let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) = &terminal.payload else { panic!("inline terminal expected"); };
    let expected = json!({"request_id":"ask-report-path","question":"Where should the report be saved?", "remaining_work":"Save and verify the report."});
    assert_eq!(payload.0["user_input_request"], expected);
    assert_eq!(payload.0["model_steps"], 1);
    let (status, projection) = file_api_get(&fixture._router, &format!("/api/agent-sessions/{}/projection", session.as_ref()), true).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(projection["data"]["runtime"]["can_send_message"], true);
    assert_eq!(projection["data"]["runtime"]["is_processing"], false);
    assert!(projection["data"]["runtime"]["active_turn_id"].is_null());
    let history_path = format!("/api/agent-sessions/{}/message-history", session.as_ref());
    let (status, history) = file_api_get(&fixture._router, &history_path, true).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let note = history["data"]["items"].as_array().expect("history items").iter()
        .find(|message| message["content"].get("user_input_request").is_some()).expect("typed question survives reopening");
    assert_eq!(note["type"], "tips");
    assert_eq!(note["content"]["type"], "warning");
    assert_eq!(note["content"]["user_input_request"], expected);
    assert_eq!(note["content"]["turn_outcome"],json!({"kind":"input_required","request_id":"ask-report-path"}));
    assert_ne!(note["msg_id"],super::super::engine_journal::canonical_assistant_message_id(
        fixture.message.source_message_id.as_ref().unwrap()).unwrap());
    assert!(!history["data"]["items"].as_array().unwrap().iter().any(|message|
        message["type"]=="text" && message["position"]=="left"
            && message["content"]["content"].as_str().is_none_or(|text|text.trim().is_empty())));
    assert_eq!(note["content"]["turn_id"], fixture.message.source_message_id.as_ref().unwrap().as_str());
    assert!(note["content"].get("error").is_none());
    let note_id = note["message_id"].clone();
    let events = fixture.runtime.subscribe();
    let next = api(&fixture._router, &format!("/api/agent-sessions/{}/turns", session.as_ref()),
        json!({"idempotency_key":"human-after-input-handoff", "input":{"content":"Save it in reports/; continue the remaining report."}})).await;
    let follow_up = receive_input_handoff_terminal(events).await;
    assert!(matches!(follow_up.last(), Some(AgentStreamEvent::Finish(data)) if data.stop_reason == Some(TurnStopReason::EndTurn)));
    let operation = next["operation_id"].as_str().unwrap();
    assert_ne!(operation, "cleanup-retry-turn");
    assert_eq!(store.read_turn_receipt(&session, &operation.into()).await.unwrap().status,
        nomifun_agent_session::TurnReceiptStatus::Completed);
    assert_eq!(store.read_turn_receipt(&session, &"cleanup-retry-turn".into()).await.unwrap().terminal_event.unwrap().event_id, terminal.event_id);
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    let (status, reopened) = file_api_get(&fixture._router, &history_path, true).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let original_note = reopened["data"]["items"].as_array().unwrap().iter()
        .find(|message| message["message_id"] == note_id).unwrap();
    assert_eq!(original_note["content"]["user_input_request"], expected);
    fixture.finish().await;
    server.close().await;
}

async fn file_api_get(router: &axum::Router, path: &str, trusted: bool) -> (axum::http::StatusCode, Value) {
    let mut request = Request::builder().method("GET").uri(path);
    if trusted { request = request.header("x-nomi-local-trust", "cleanup-fixture-local-trust"); }
    let response = router.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[cfg(unix)]
#[tokio::test]
async fn turn_file_api_discovers_command_binary_outputs_and_preserves_exact_turn_history() {
    let fixture = Fixture::new("turn-file-command-observation").await;
    fixture.host.prepare_turn(&fixture.message, CancellationToken::new()).await.unwrap();
    let session = AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let root = fixture.message.source_message_id.as_ref().unwrap();
    let path = format!("/api/agent-sessions/{}/turn-files?turn_ids={root}", session.as_ref());
    let (status, running) = file_api_get(&fixture._router, &path, true).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(running["data"], json!([]));
    // Use the real installed host's resource boundaries and a real process,
    // without dispatching a model or manufacturing a workspace file receipt.
    // The binary fixture exercises detection, not Office document validation.
    let workspace = fixture.host.options.workspace.clone();
    let command = tokio::process::Command::new("/bin/sh")
        .current_dir(&workspace).arg("-c")
        .arg("printf '%s\\n' '# fixture generator' > \"$1\"; printf 'PK\\003\\004\\000\\377binary fixture' > \"$2\"")
        .arg("turn-file-fixture").arg("build_docx.py").arg("骆驼祥子读后感.docx")
        .status().await.unwrap();
    assert!(command.success());
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnCancelled { model_steps:0 }).await.unwrap();
    fixture.assert_settled().await;
    let (status, first) = file_api_get(&fixture._router, &path, true).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{first}");
    let summary: Vec<nomifun_api_types::TurnFileSummary> = serde_json::from_value(first["data"].clone()).unwrap();
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].turn_id, *root);
    assert!(summary[0].complete);
    assert_eq!(summary[0].files.iter().map(|file| file.file_name.as_str()).collect::<Vec<_>>(),
        vec!["build_docx.py", "骆驼祥子读后感.docx"]);
    assert!(summary[0].files.iter().all(|file| file.operation == nomifun_common::FileChangeOperation::Create
        && file.size_bytes.is_some() && file.sha256.is_some()));
    let facts = fixture.owner.canonical().store().turn_output_facts(&session, &"cleanup-retry-turn".into()).await.unwrap();
    assert!(facts.events.iter().any(|event| event.kind.0 == nomifun_agent_contracts::TURN_WORKSPACE_FILES_OBSERVED));
    // A newly opened store reads the persisted summary even while a later
    // input is active and the original files have since changed or disappeared.
    let store = fixture.owner.canonical().store();
    let (input, _) = store.start_turn(&session, "session_api".into(), "file-successor".into(),
        "file-successor".into(), StrictJsonValue(json!({"content":"later turn","admission":{
            "route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref,
        }}))).await.unwrap();
    std::fs::remove_file(std::path::Path::new(&workspace).join("骆驼祥子读后感.docx")).unwrap();
    std::fs::write(std::path::Path::new(&workspace).join("build_docx.py"), "changed after the original Turn").unwrap();
    let second = file_api_get(&fixture._router, &path, true).await;
    assert_eq!(second.0, axum::http::StatusCode::OK);
    assert_eq!(first["data"], second.1["data"]);
    let reopened_pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(&fixture.database_path)).await.unwrap();
    let reopened = nomifun_agent_session::AgentSessionStore::from_pool(reopened_pool.clone()).await.unwrap();
    let reloaded = super::super::turn_files::query(&reopened, &session, &[root.clone()], None,
        std::path::Path::new(&workspace).parent().unwrap()).await.unwrap();
    assert_eq!(summary, reloaded);
    reopened_pool.close().await;
    let next = SendMessageData { source_message_id:Some(input.record.unwrap().event_id.as_ref().into()),
        msg_id:"file-successor-wire".into(), content:"later turn".into(), files:vec![], inject_skills:vec![], origin:None };
    fixture.host.cleanup_turn(&next).await.unwrap();
    fixture.host.record_event(&next, &AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    fixture.finish().await;
}

#[tokio::test]
async fn turn_file_api_requires_owner_and_returns_no_final_list_for_paused_turns() {
    let fixture = Fixture::new("turn-file-api-authority-and-pause").await;
    fixture.host.prepare_turn(&fixture.message, CancellationToken::new()).await.unwrap();
    let session = AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let root = fixture.message.source_message_id.as_ref().unwrap();
    let path = format!("/api/agent-sessions/{}/turn-files?turn_ids={root}", session.as_ref());
    let denied = file_api_get(&fixture._router, &path, false).await;
    assert!(matches!(denied.0, axum::http::StatusCode::UNAUTHORIZED | axum::http::StatusCode::FORBIDDEN));
    let foreign = nomifun_auth::CurrentUser {
        id:nomifun_common::UserId::parse(nomifun_common::generate_id()).unwrap(), username:"foreign fixture".into(),
    };
    let foreign_router = super::super::nomi_core_session::build_nomi_core_agent_router(fixture.api_state.clone())
        .layer(axum::Extension(foreign));
    let denied = file_api_get(&foreign_router, &path, false).await;
    assert_eq!(denied.0, axum::http::StatusCode::FORBIDDEN);
    assert!(denied.1.get("data").is_none(), "a foreign principal received file metadata");
    let binding = fixture.owner.canonical().store().get_live_session(&session).await.unwrap().agent_binding;
    let other = api(&fixture._router, "/api/agent-sessions", json!({
        "preset_id":binding.preset_revision_ref.preset_id,"model":fixture.host.options.model,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}],
    })).await;
    let cross_session = format!("/api/agent-sessions/{}/turn-files?turn_ids={root}", other["agent_session_id"].as_str().unwrap());
    assert_eq!(file_api_get(&fixture._router, &cross_session, true).await.0, axum::http::StatusCode::NOT_FOUND);
    let checkpoint = AgentExecutionCheckpoint {
        version:1, binding:fixture.host.engine_binding.clone(), turn_operation_id:"cleanup-retry-turn".into(),
        active_set_generation:fixture.host.capability_state.snapshot().unwrap().generation,
        model_steps:0, tool_call_count:0, accepted_input_count:1, applied_steering_receipts:vec![],
        plan:Default::default(), work:Default::default(), patch_recovery:Default::default(), segments:None,
        control_rejections:Default::default(), pending_user_input: None, completion_review:Default::default(),
    };
    fixture.host.save_execution_checkpoint(&fixture.message, &checkpoint).await.unwrap().unwrap();
    fixture.host.cleanup_turn(&fixture.message).await.unwrap();
    fixture.host.record_event(&fixture.message, &AgentEngineEvent::TurnPaused {
        model_steps:0, reason:"EXECUTION_USER_REQUESTED".into(),
    }).await.unwrap();
    let (status, paused) = file_api_get(&fixture._router, &path, true).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{paused}");
    assert_eq!(paused["data"], json!([]));
    fixture.finish().await;
}
