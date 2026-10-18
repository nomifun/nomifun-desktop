//! A previous bundled release continues through the production Session owner.
//! All records and transports belong to the isolated in-memory fixture. Old
//! immutable contracts are inserted before Session creation; no user data or
//! projection supplies history, binding authority, or compatibility evidence.

use std::sync::{Arc, Mutex};

use axum::{Router, body::Body, http::{Request, StatusCode}};
use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetRevision, AgentSessionId, AgentSessionLiveRecord,
    CanonicalSchemaRef, CorrelationId, EventId, EventProducerId, IdempotencyKey,
    OperationId, PluginSourceKind, RemoteBindingProvenance, ResolvedSnapshotEnvelope, SemanticSessionEventDraft,
    SessionEventAppend, SessionEventKind, SessionEventPayloadRef, StrictJsonValue, digest_payload,
};
use nomifun_agent_session::{
    AgentSessionStore, CreateSessionRequest, EffectEventRequest, EffectStrategy, EffectTerminalState, NativePauseState,
};
use nomifun_agent_kernel::{AgentPresetCompiler, CompileRequest};
use nomifun_app::compatibility::AppServices;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};

const TRUST: &str = "session-contract-evolution";
const OLD_INPUT: &str = "PREVIOUS_BUNDLED_RELEASE_INPUT";
const REPLY: &str = "FORWARD_EXECUTION_OK";

async fn response(router: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = router.clone().oneshot(Request::builder().method(method).uri(uri)
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024).await.unwrap();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

async fn call(router: &Router, method: &str, uri: &str, body: Value) -> Value {
    let (status, value) = response(router, method, uri, body).await;
    assert!(matches!(status, StatusCode::OK | StatusCode::CREATED), "{method} {uri}: {status} {value}");
    value["data"].clone()
}

struct Fixture {
    router: Router,
    services: AppServices,
    store: AgentSessionStore,
    seed: AgentSessionLiveRecord,
    previous: ResolvedSnapshotEnvelope,
    old_binding: AgentBindingValue,
    second_model: Value,
    requests: Arc<Mutex<Vec<Value>>>,
    _upstream: MockServer,
}

impl Fixture {
    async fn new(managed_source: bool) -> Self {
        Self::with_previous_contract(managed_source,false).await
    }

    async fn with_previous_contract(managed_source: bool, start_only_defaults: bool) -> Self {
        let upstream = MockServer::start().await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                captured.lock().unwrap().push(request.body_json::<Value>().unwrap());
                ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                    .set_body_string(format!(
                        "data: {{\"id\":\"forward-contract\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{REPLY}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"forward-contract\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
                    ))
            }).mount(&upstream).await;
        let (router, services) = super::common::build_local_trust_app(TRUST).await;
        let mut models = Vec::new();
        for model in ["contract-one", "contract-two"] {
            let provider = call(&router, "POST", "/api/providers", json!({
                "platform": "stepfun-plan", "name": model, "base_url": format!("{}/v1", upstream.uri()),
                "auth_scheme": "bearer", "credentials": { "api_keys": ["fixture-only"] }, "enabled": true,
                "initial_model": { "model": model, "enabled": true, "capabilities": [{
                    "task": "chat", "traits": [], "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {}
                }] }
            })).await;
            models.push(json!({ "provider_id": provider["provider_id"], "model": model }));
        }
        let preset = call(&router, "POST", "/api/agent-presets/from-template/coding.codex", json!({
            "display_name": "Contract evolution fixture", "reuse_existing": false,
            "model_route_refs": {}, "chat_route_records": {}, "model": models[0]
        })).await;
        let seed = call(&router, "POST", "/api/agent-sessions", json!({
            "preset_id": preset["preset"]["preset_id"], "model": models[0],
            "resource_selections": [
                { "resource_kind": "workspace", "resource_id": "default-workspace" },
                { "resource_kind": "process_session", "resource_id": "managed-process-session" },
                { "resource_kind": "project_memory", "resource_id": "default-project-memory" }
            ]
        })).await;
        let pool = services.database.pool();
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let seed = store.get_live_session(&AgentSessionId::from(seed["agent_session_id"].as_str().unwrap())).await.unwrap();
        let current_snapshot: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
            .bind(seed.agent_binding.resolved_snapshot_ref.snapshot_id.as_ref()).fetch_one(pool).await.unwrap();
        let mut previous: ResolvedSnapshotEnvelope = serde_json::from_str(&current_snapshot).unwrap();
        let reference = &seed.agent_binding.preset_revision_ref;
        let (payload, created_by, created_at): (String, String, i64) = sqlx::query_as(
            "SELECT payload_json,created_by,created_at FROM agent_preset_revisions WHERE revision_id=?"
        ).bind(reference.revision_id()).fetch_one(pool).await.unwrap();
        let locks: Vec<String> = sqlx::query_scalar("SELECT lock_json FROM agent_preset_contribution_locks WHERE revision_id=? ORDER BY contribution_id")
            .bind(reference.revision_id()).fetch_all(pool).await.unwrap();
        let mut revision = AgentPresetRevision {
            reference: reference.clone(), payload: serde_json::from_str(&payload).unwrap(),
            contribution_locks: locks.iter().map(|lock| serde_json::from_str(lock).unwrap()).collect(),
            created_by: created_by.into(), created_at_ms: created_at, reason: Some("Fixture previous bundled release".into()),
        };
        // workspace.files is present on every supported host. The mechanism
        // deliberately has no Browser, application version, or digest exception.
        if !start_only_defaults {
        let capability = previous.content.enabled_capabilities.iter_mut()
            .find(|capability| capability.capability.id.as_ref() == "workspace.files").unwrap();
        assert_eq!(capability.resolved_source.source_kind, PluginSourceKind::Bundled);
        let old_lock = capability.contribution_lock.clone();
        capability.schema_digest = digest_payload(&"previous bundled schema").unwrap();
        capability.target_artifact_digest = digest_payload(&"previous bundled artifact").unwrap();
        capability.contribution_lock.contract_digest = capability.schema_digest.clone();
        capability.actions[0].input_schema = CanonicalSchemaRef::from(format!(
            "schema://fixture.previous/input#{}", digest_payload(&"previous input").unwrap().as_ref()
        ));
        if managed_source { capability.resolved_source.source_kind = PluginSourceKind::ManagedLocal; }
        for lock in &mut revision.contribution_locks {
            if *lock == old_lock { *lock = capability.contribution_lock.clone(); }
        }
        let capability_id = capability.capability.id.clone();
        for provider in previous.content.resolved_role_providers.values_mut()
            .filter(|provider| provider.supported_members.contains(&capability_id))
        {
            provider.provider.role.contract_digest = digest_payload(&"previous bundled role").unwrap();
            provider.provider.contribution_digest = digest_payload(&"previous bundled provider").unwrap();
        }
        }
        // Preserve a real old grant ceiling: this Session selected only the
        // retired synchronous launch, not poll or stdin controls. The forward
        // launch transition must not silently add those unselected Actions.
        let process = previous.content.enabled_capabilities.iter_mut()
            .find(|capability|capability.capability.id.as_ref()=="workspace.process").unwrap();
        if start_only_defaults {
            let start=process.actions.iter().find(|action|action.action_id.as_ref()=="workspace.process/start").unwrap();
            let mut prior_schema=nomifun_agent_domain_wave2::resolve_action_schema("workspace.process",&start.input_schema).unwrap();
            prior_schema.0["properties"]["timeout_ms"]["default"]=json!(30000);
            prior_schema.0["properties"]["timeout_ms"]["maximum"]=json!(600000);
            prior_schema.0["properties"]["wait_ms"]["default"]=json!(0);
            let lock=process.contribution_lock.clone();
            process.actions.iter_mut().find(|action|action.action_id.as_ref()=="workspace.process/start").unwrap().input_schema=
                format!("schema://fixture.previous/process-start/input#{}",digest_payload(&prior_schema).unwrap().as_ref()).into();
            process.schema_digest=digest_payload(&("previous process defaults",&prior_schema)).unwrap();
            process.contribution_lock.contract_digest=process.schema_digest.clone();
            process.target_artifact_digest=digest_payload(&"previous bounded launch artifact").unwrap();
            for saved in &mut revision.contribution_locks {if *saved==lock {*saved=process.contribution_lock.clone();}}
            process.action_allowlist=std::collections::BTreeSet::from(["workspace.process/start".into()]);
        } else {
            let mut retired_launch = process.actions.iter().find(|action|action.action_id.as_ref()=="workspace.process/start").unwrap().clone();
            retired_launch.action_id = "workspace.process/exec".into();
            process.actions.insert(0,retired_launch);
            process.action_allowlist = std::collections::BTreeSet::from(["workspace.process/exec".into()]);
        }
        revision.payload.enabled_capabilities.iter_mut().find(|selection|selection.capability.id.as_ref()=="workspace.process")
            .unwrap().action_allowlist = process.action_allowlist.clone();
        // The immediately preceding current-generation publisher did not
        // encode per-Action resource operations. Its exact execution ceiling
        // was the complete owning module resource-kind set, including the
        // empty set for hidden tool discovery. Exercise the real forward
        // transition for every Action, rather than merely changing an input
        // schema on one already-normalized capability.
        if !start_only_defaults { for capability in &mut previous.content.enabled_capabilities {
            for action in &mut capability.actions {
                action.required_resource_operations = None;
            }
        }}
        let latest: i64 = sqlx::query_scalar("SELECT MAX(revision_no) FROM agent_preset_revisions WHERE preset_id=?")
            .bind(reference.preset_id.as_ref()).fetch_one(pool).await.unwrap();
        revision.reference.revision = u64::try_from(latest).unwrap() + 1;
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        revision.validate().unwrap();
        previous.content.preset_revision_ref = revision.reference.clone();
        previous.content.chat_route_identity = revision.chat_route_identity().unwrap();
        previous.snapshot_ref.snapshot_digest = digest_payload(&previous.content).unwrap();
        previous.snapshot_ref.snapshot_id = format!("fixture.previous:{}", uuid::Uuid::now_v7()).into();
        previous.validate().unwrap();
        // Reconstruct the profile identity from these frozen contracts through
        // the canonical algorithm, without consulting the current Registry.
        previous = AgentPresetCompiler::derive_model_snapshot(&revision, &previous, CompileRequest {
            revision: revision.clone(), principal: previous.actor.clone(), scene: previous.scene.clone(),
            surface: previous.surface.clone(), audience: previous.audience.clone(),
            created_at_ms: previous.created_at_ms, resolver_run_id: previous.resolver_run_id.clone(),
        }).unwrap();
        // Insert new immutable fixture artifacts; the real current artifacts
        // remain intact, just as two installations publish separate releases.
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO agent_preset_revisions (revision_id,preset_id,revision_no,schema_version,payload_json,revision_digest,created_by,created_at,reason) VALUES (?,?,?,?,?,?,?,?,?)")
            .bind(revision.reference.revision_id()).bind(revision.reference.preset_id.as_ref())
            .bind(i64::try_from(revision.reference.revision).unwrap()).bind(revision.payload.schema_version.as_ref())
            .bind(serde_json::to_string(&revision.payload).unwrap()).bind(revision.reference.revision_digest.as_ref())
            .bind(revision.created_by.as_ref()).bind(revision.created_at_ms).bind(revision.reason.as_deref())
            .execute(&mut *tx).await.unwrap();
        for lock in &revision.contribution_locks {
            sqlx::query("INSERT INTO agent_preset_contribution_locks (revision_id,contribution_id,lock_json) VALUES (?,?,?)")
                .bind(revision.reference.revision_id()).bind(lock.contribution_id.as_ref()).bind(serde_json::to_string(lock).unwrap())
                .execute(&mut *tx).await.unwrap();
        }
        sqlx::query("INSERT INTO agent_runtime_snapshots (snapshot_id,snapshot_digest,content_json,envelope_json) VALUES (?,?,?,?)")
            .bind(previous.snapshot_ref.snapshot_id.as_ref()).bind(previous.snapshot_ref.snapshot_digest.as_ref())
            .bind(serde_json::to_string(&previous.content).unwrap()).bind(serde_json::to_string(&previous).unwrap())
            .execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let old_binding = AgentBindingValue { preset_revision_ref: revision.reference, resolved_snapshot_ref: previous.snapshot_ref.clone(),
            typed_resource_bindings: seed.agent_binding.typed_resource_bindings.clone(), binding_version: 1 };
        Self { router, services, store, seed, previous, old_binding, second_model: models.remove(1), requests, _upstream: upstream }
    }

    async fn old_session(&self, unknown_effect: bool) -> AgentSessionId {
        self.old_session_with_remote(unknown_effect, None).await
    }

    async fn old_session_with_remote(&self, unknown_effect: bool, remote: Option<RemoteBindingProvenance>) -> AgentSessionId {
        let id = AgentSessionId::from(uuid::Uuid::now_v7().to_string());
        let session_id = id.as_ref();
        let mut session = self.seed.clone();
        session.agent_session_id = id.clone();
        session.agent_binding = self.old_binding.clone();
        session.remote_binding_provenance = remote;
        session.next_seq = 1;
        let mut create = CreateSessionRequest::new(session, nomifun_common::now_ms(), OperationId::from(format!("open:{session_id}")),
            EventProducerId::from("session_api"), IdempotencyKey::from(format!("open:{session_id}")), CorrelationId::from(format!("open:{session_id}")));
        create.initial_active_capability_ids = self.previous.content.capability_allowlist.iter().map(|id| id.as_ref().to_owned()).collect();
        let created = self.store.create_session(create).await.unwrap();
        self.store.append_event(&self.event(&id, "session/ready", "ready", Some(created.opening_ack.event_id), json!({}))).await.unwrap();
        let key = format!("old-turn:{session_id}");
        let scoped_key = format!("user:{}:{session_id}:{key}", self.services.authoritative_user_id);
        let operation = OperationId::from(format!("turn:{scoped_key}"));
        let (_, started) = self.store.start_turn(&id, EventProducerId::from("session_api"), IdempotencyKey::from(scoped_key),
            operation.clone(), StrictJsonValue(json!({ "content": OLD_INPUT, "files": [], "inject_skills": [],
                "hidden": false, "origin": null, "channel_platform": null, "admission": {
                "route_identity": self.previous.content.chat_route_identity, "resolved_snapshot_ref": self.previous.snapshot_ref
            } }))).await.unwrap();
        let mut cause = started.ack.unwrap().event_id;
        let mut unknown_terminal = None;
        if unknown_effect {
            let tool_operation = format!("push:{session_id}");
            let mut tool = self.event(&id, "tool/call-started", "old-push", Some(cause), json!({
                "operation_id": tool_operation, "capability_id": "workspace.vcs", "action_id": "workspace.vcs/push"
            }));
            tool.producer_id = EventProducerId::from("capability_host");
            tool.semantic_event.correlation_id = CorrelationId::from(tool_operation.as_str());
            cause = self.store.append_event(&tool).await.unwrap().ack.unwrap().event_id;
            let effect = EffectEventRequest {
                agent_session_id: id.clone(), effect_id: format!("unknown:{session_id}"), turn_id: operation.clone(),
                operation_id: OperationId::from(format!("push:{session_id}")), owner_domain: "workspace".into(),
                capability_module: "workspace.vcs".into(), action_id: "workspace.vcs/push".into(),
                resource_binding_id: None, resource_key: Some("workspace.vcs:origin:main".into()),
                input_digest: digest_payload(&"push input").unwrap(), recorded_at: nomifun_common::now_ms(),
                event_id: EventId::from(format!("effect-started:{session_id}")), producer_id: EventProducerId::from("capability_host"),
                idempotency_key: IdempotencyKey::from(format!("effect-started:{session_id}")), correlation_id: CorrelationId::from(format!("unknown:{session_id}")),
                strategy: EffectStrategy::ExternalUncertainEffect, causation_event_id: Some(cause),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
            };
            self.store.record_effect_started(effect.clone()).await.unwrap();
            let mut terminal = effect;
            terminal.causation_event_id = Some(terminal.event_id.clone());
            terminal.event_id = EventId::from(format!("effect-unknown:{session_id}"));
            terminal.producer_id = EventProducerId::from("workspace-effect-owner");
            cause = terminal.causation_event_id.clone().unwrap();
            unknown_terminal = Some(terminal);
        }
        // A genuine accepted input that never began Runtime execution is valid
        // closed history. Do not fabricate native events for the old artifact.
        let mut terminal = self.event(&id, "turn/failed", "old-terminal", Some(cause), json!({ "message": "Previous fixture host stopped before Runtime startup" }));
        terminal.semantic_event.correlation_id = CorrelationId::from(operation.as_ref());
        self.store.append_turn_terminal(&terminal, &operation).await.unwrap();
        if let Some(terminal) = unknown_terminal {
            // The external receipt can arrive after the local Turn closes.
            // Canonical settlement records uncertainty without reopening it.
            self.store.record_effect_terminal(terminal, EffectTerminalState::Uncertain).await.unwrap();
        }
        id
    }

    fn event(&self, id: &AgentSessionId, kind: &str, key: &str, cause: Option<EventId>, payload: Value) -> SessionEventAppend {
        let session_id = id.as_ref();
        SessionEventAppend {
            agent_session_id: id.clone(), event_id: EventId::from(format!("{key}:{session_id}")),
            producer_id: EventProducerId::from("runtime_supervisor"), idempotency_key: IdempotencyKey::from(format!("{key}:{session_id}")),
            semantic_event: SemanticSessionEventDraft { kind: SessionEventKind(kind.into()), kind_version: 1,
                correlation_id: CorrelationId::from(format!("{key}:{session_id}")), causation_event_id: cause,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)) },
        }
    }

    async fn transitions(&self, id: &AgentSessionId) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed'")
            .bind(id.as_ref()).fetch_one(self.services.database.pool()).await.unwrap()
    }

    async fn start(&self, id: &AgentSessionId) -> OperationId {
        let session_id = id.as_ref();
        let accepted = call(&self.router, "POST", &format!("/api/agent-sessions/{session_id}/turns"), json!({
            "idempotency_key": uuid::Uuid::now_v7().to_string(), "input": { "content": "continue this conversation" }
        })).await;
        OperationId::from(accepted["operation_id"].as_str().unwrap())
    }

    async fn wait_for_reply(&self, id: &AgentSessionId, operation: &OperationId) -> Value {
        let session_id = id.as_ref();
        for _ in 0..600 {
            let history = call(&self.router, "GET", &format!("/api/agent-sessions/{session_id}/message-history?page_size=100"), Value::Null).await;
            let head = self.store.head(id).await.unwrap();
            if history.to_string().contains(REPLY) && head.active_turn_id.is_none() && head.status != "running" {
                assert_eq!(self.store.read_turn_receipt(id, operation).await.unwrap().status,
                    nomifun_agent_session::TurnReceiptStatus::Completed,
                    "the new native execution must complete successfully");
                return history;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let events = call(&self.router, "GET", &format!("/api/agent-sessions/{session_id}/events?after_seq=0&limit=500"), Value::Null).await;
        panic!("new native Turn did not finish: {events}");
    }

    async fn finish(self) {
        self.services.agent_runtime_sessions.terminate_all();
        self.services.shutdown_browser_platform().await.unwrap();
        self.services.database.close().await;
    }
}

#[tokio::test]
async fn bundled_contract_evolution_prepares_warmup_send_and_model_with_canonical_history() {
    let fixture = Fixture::new(false).await;
    assert!(fixture.previous.content.enabled_capabilities.iter().flat_map(|capability| &capability.actions)
        .all(|action| action.required_resource_operations.is_none()));
    for trigger in ["warmup", "send", "model"] {
        let id = fixture.old_session(false).await;
        let session_id = id.as_ref();
        call(&fixture.router, "POST", &format!("/api/agent-sessions/{session_id}/turns"), json!({
            "idempotency_key": format!("old-turn:{session_id}"), "input": { "content": OLD_INPUT }
        })).await;
        assert_eq!(fixture.store.get_live_session(&id).await.unwrap().agent_binding, fixture.old_binding,
            "exact-key redelivery retains its original admission without evolving the binding");
        assert_eq!(fixture.transitions(&id).await, 0);
        match trigger {
            "warmup" => { call(&fixture.router, "POST", &format!("/api/agent-sessions/{session_id}/warmup"), json!({})).await; }
            "model" => { call(&fixture.router, "PUT", &format!("/api/agent-sessions/{session_id}/model"), fixture.second_model.clone()).await; }
            _ => {}
        }
        let operation = fixture.start(&id).await;
        let history = fixture.wait_for_reply(&id, &operation).await;
        assert!(history.to_string().contains(OLD_INPUT), "the original accepted input remains visible");
        assert_eq!(fixture.transitions(&id).await, 1, "{trigger} commits one forward execution boundary");
        let transitioned = fixture.store.get_live_session(&id).await.unwrap();
        assert_eq!(transitioned.agent_binding.typed_resource_bindings, fixture.old_binding.typed_resource_bindings);
        assert!(transitioned.agent_binding.binding_version > fixture.old_binding.binding_version);
        assert_ne!(transitioned.agent_binding.resolved_snapshot_ref, fixture.old_binding.resolved_snapshot_ref);
        let current_json: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
            .bind(transitioned.agent_binding.resolved_snapshot_ref.snapshot_id.as_ref()).fetch_one(fixture.services.database.pool()).await.unwrap();
        let current: ResolvedSnapshotEnvelope = serde_json::from_str(&current_json).unwrap();
        let process = current.content.enabled_capabilities.iter().find(|capability|capability.capability.id.as_ref()=="workspace.process").unwrap();
        assert_eq!(process.action_allowlist,std::collections::BTreeSet::from(["workspace.process/start".into()]),
            "the exact old launch grant becomes bounded launch without new control authority");
        assert!(!process.actions.iter().any(|action|action.action_id.as_ref()=="workspace.process/exec"));
        let requests = fixture.requests.lock().unwrap();
        assert!(requests.last().unwrap()["messages"].to_string().contains(OLD_INPUT), "old input enters the actual next model request as canonical context");
        drop(requests);
        call(&fixture.router, "POST", &format!("/api/agent-sessions/{session_id}/warmup"), json!({})).await;
        call(&fixture.router, "PUT", &format!("/api/agent-sessions/{session_id}/model"), fixture.second_model.clone()).await;
        assert_eq!(fixture.transitions(&id).await, 1, "repeat preparation/model derivation does not evolve twice");
        let old_json: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
            .bind(fixture.previous.snapshot_ref.snapshot_id.as_ref()).fetch_one(fixture.services.database.pool()).await.unwrap();
        assert_eq!(serde_json::from_str::<ResolvedSnapshotEnvelope>(&old_json).unwrap(), fixture.previous, "normal preparation preserves immutable old Snapshot");
    }
    fixture.finish().await;
}

#[tokio::test]
async fn prior_start_schema_defaults_evolve_only_at_idle_and_leave_authority_and_history_exact() {
    let fixture=Fixture::with_previous_contract(false,true).await;
    let old_start=fixture.previous.content.enabled_capabilities.iter().find(|capability|capability.capability.id.as_ref()=="workspace.process").unwrap()
        .actions.iter().find(|action|action.action_id.as_ref()=="workspace.process/start").unwrap().clone();
    for trigger in ["warmup","send","model"] {
        let id=fixture.old_session(false).await;let sid=id.as_ref();
        match trigger {
            "warmup"=>{call(&fixture.router,"POST",&format!("/api/agent-sessions/{sid}/warmup"),json!({})).await;},
            "model"=>{call(&fixture.router,"PUT",&format!("/api/agent-sessions/{sid}/model"),fixture.second_model.clone()).await;},
            _=>{},
        }
        let before=fixture.requests.lock().unwrap().len();
        let operation=fixture.start(&id).await;
        fixture.wait_for_reply(&id,&operation).await;
        assert_eq!(fixture.transitions(&id).await,1);
        let binding=fixture.store.get_live_session(&id).await.unwrap().agent_binding;
        assert_eq!(binding.typed_resource_bindings,fixture.old_binding.typed_resource_bindings);
        let current_json:String=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
            .bind(binding.resolved_snapshot_ref.snapshot_id.as_ref()).fetch_one(fixture.services.database.pool()).await.unwrap();
        let current:ResolvedSnapshotEnvelope=serde_json::from_str(&current_json).unwrap();
        let process=current.content.enabled_capabilities.iter().find(|capability|capability.capability.id.as_ref()=="workspace.process").unwrap();
        assert_eq!(process.action_allowlist,std::collections::BTreeSet::from(["workspace.process/start".into()]));
        let start=process.actions.iter().find(|action|action.action_id.as_ref()=="workspace.process/start").unwrap();
        assert_ne!(start.input_schema,old_start.input_schema);
        assert_eq!(start.effect_class,old_start.effect_class);
        assert_eq!(start.required_resource_operations,old_start.required_resource_operations);
        let requests=fixture.requests.lock().unwrap();
        let starts=requests[before..].iter().flat_map(|request|request["tools"].as_array().into_iter().flatten())
            .filter(|tool|tool["function"]["name"]=="start_process").collect::<Vec<_>>();
        assert!(!starts.is_empty(),"the evolved launch definition must reach the actual model boundary");
        for tool in starts {
            let parameters=&tool["function"]["parameters"];
            assert!(parameters["properties"]["timeout_ms"].get("default").is_none());
            assert!(parameters["properties"]["timeout_ms"].get("maximum").is_none());
            assert_eq!(parameters["properties"]["wait_ms"]["default"],1000);
            let text=tool.to_string();assert!(!text.contains("Default 30000") && !text.contains("default 30000"));
        }
        drop(requests);
        let old_json:String=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
            .bind(fixture.previous.snapshot_ref.snapshot_id.as_ref()).fetch_one(fixture.services.database.pool()).await.unwrap();
        assert_eq!(serde_json::from_str::<ResolvedSnapshotEnvelope>(&old_json).unwrap(),fixture.previous);
    }
    fixture.finish().await;
}

#[tokio::test]
async fn bundled_contract_evolution_cannot_rebind_unknown_effects() {
    let fixture = Fixture::new(false).await;
    let id = fixture.old_session(true).await;
    let session_id = id.as_ref();
    for (method, endpoint, body) in [
        ("POST", "warmup", json!({})),
        ("POST", "turns", json!({ "idempotency_key": uuid::Uuid::now_v7().to_string(), "input": { "content": "must remain blocked" } })),
        ("PUT", "model", fixture.second_model.clone()),
    ] {
        let (status, error) = response(&fixture.router, method, &format!("/api/agent-sessions/{session_id}/{endpoint}"), body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{endpoint}: {error}");
        assert_eq!(fixture.store.get_live_session(&id).await.unwrap().agent_binding, fixture.old_binding);
        assert_eq!(fixture.transitions(&id).await, 0);
    }
    assert!(fixture.requests.lock().unwrap().is_empty(), "unknown effects never permit model execution");
    fixture.finish().await;
}

#[tokio::test]
async fn bundled_contract_evolution_cannot_adopt_managed_source_drift() {
    let fixture = Fixture::new(true).await;
    let id = fixture.old_session(false).await;
    let session_id = id.as_ref();
    for (method, endpoint, body) in [
        ("POST", "warmup", json!({})),
        ("POST", "turns", json!({ "idempotency_key": uuid::Uuid::now_v7().to_string(), "input": { "content": "do not adopt publisher changes" } })),
        ("PUT", "model", fixture.second_model.clone()),
    ] {
        let (status, error) = response(&fixture.router, method, &format!("/api/agent-sessions/{session_id}/{endpoint}"), body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{endpoint}: {error}");
        assert_eq!(error["code"], "AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED", "{endpoint}: {error}");
        assert_eq!(fixture.store.get_live_session(&id).await.unwrap().agent_binding, fixture.old_binding);
        assert_eq!(fixture.transitions(&id).await, 0);
    }
    assert!(fixture.requests.lock().unwrap().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn bundled_contract_evolution_preserves_active_paused_automation_and_remote_bindings() {
    let fixture = Fixture::new(false).await;
    for boundary in ["active", "paused", "automation", "remote"] {
        let remote = if boundary == "remote" {
            let binding = call(&fixture.router, "POST", "/api/remote-bindings", json!({
                "name": "Frozen previous release", "agent_binding": fixture.old_binding
            })).await;
            Some(RemoteBindingProvenance {
                remote_binding_id: binding["remote_binding_id"].as_str().unwrap().into(), binding_version: 1,
            })
        } else { None };
        let id = fixture.old_session_with_remote(false, remote).await;
        let session_id = id.as_ref();
        if matches!(boundary, "active" | "paused") {
            let operation = OperationId::from(format!("frozen-active:{session_id}"));
            let (_, started) = fixture.store.start_turn(&id, EventProducerId::from("session_api"),
                IdempotencyKey::from(operation.as_ref()), operation.clone(), StrictJsonValue(json!({ "content": "Existing operation" }))).await.unwrap();
            if boundary == "paused" {
                let pause = NativePauseState {
                    revision: 1, reason: "EXECUTION_USER_REQUESTED".into(), checkpoint_revision: 0,
                    checkpoint_digest: None, execution_fence: 1, cleanup_proven: true, paused_at_ms: nomifun_common::now_ms(),
                };
                let mut event = fixture.event(&id, "turn/paused", "native-pause", Some(started.ack.unwrap().event_id), json!({ "pause": pause }));
                event.semantic_event.correlation_id = CorrelationId::from(operation.as_ref());
                fixture.store.append_event(&event).await.unwrap();
            }
        } else if boundary == "automation" {
            // AgentExecution fixtures use this persisted relation for an
            // automation Attempt; its own immutable binding is audit evidence.
            sqlx::query("INSERT INTO conversation_execution_links (conversation_id,execution_id,relation,step_id,attempt_id,created_at,updated_at) VALUES (?,?,?,?,?,1,1)")
                .bind(session_id).bind(uuid::Uuid::now_v7().to_string()).bind("automation")
                .bind(uuid::Uuid::now_v7().to_string()).bind(uuid::Uuid::now_v7().to_string())
                .execute(fixture.services.database.pool()).await.unwrap();
        }
        for (method, endpoint, body) in [
            ("POST", "warmup", json!({})),
            ("POST", "turns", json!({ "idempotency_key": uuid::Uuid::now_v7().to_string(), "input": { "content": "Respect frozen execution authority" } })),
            ("PUT", "model", fixture.second_model.clone()),
        ] {
            let (status, value) = response(&fixture.router, method, &format!("/api/agent-sessions/{session_id}/{endpoint}"), body).await;
            if endpoint == "turns" && matches!(boundary, "automation" | "remote") && status == StatusCode::OK {
                // Admission may return before exact old-contract compilation
                // fails. The canonical failure must not become an upgrade.
                let operation = OperationId::from(value["data"]["operation_id"].as_str().unwrap());
                let receipt = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                    loop {
                        let receipt = fixture.store.read_turn_receipt(&id, &operation).await.unwrap();
                        if receipt.terminal_event.is_some() { break receipt; }
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }).await.unwrap();
                assert_eq!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Failed, "{boundary}");
            } else {
                assert_eq!(status, StatusCode::CONFLICT, "{boundary}/{endpoint}: {value}");
            }
            assert_eq!(fixture.store.get_live_session(&id).await.unwrap().agent_binding, fixture.old_binding, "{boundary}/{endpoint}");
            assert_eq!(fixture.transitions(&id).await, 0, "{boundary}/{endpoint}");
        }
    }
    assert!(fixture.requests.lock().unwrap().is_empty(), "frozen execution authority never invokes the new model");
    fixture.finish().await;
}
