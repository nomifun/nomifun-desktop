//! Real Session/Runtime/Broker requests across the repository-authenticated
//! auto-group guest ceiling. No model transport or permission projection bypass.
use super::*;
use nomifun_agent_contracts::{
    SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef,
};
use nomifun_agent_session::{EffectEventRequest, EffectStrategy, EffectTerminalState};
use nomifun_channel::ChannelSessionPort;
use nomifun_db::models::ChannelSessionRow;
use sha2::Digest as _;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, path},
};

const TRUST: &str = "channel-guest-contract-fixture";
const PRIVATE_PERSONA: &str = "PRIVATE_OWNER_GUEST_FIXTURE_PERSONA";
const PRIVATE_INSTRUCTIONS: &str = "PRIVATE_OWNER_GUEST_FIXTURE_INSTRUCTIONS";
const PRIVATE_MEMORY: &str = "PRIVATE_PAIRED_COMPANION_MEMORY_MUST_NOT_REACH_GUEST";
const PRIVATE_HISTORY: &str = "PRIVATE_APPROVED_HISTORY_BEFORE_GUEST_ADOPTION";

struct Fixture {
    _root: tempfile::TempDir,
    services: crate::services::AppServices,
    router: Router,
    owner: Arc<NomiCoreSessionOwner>,
    api: NomiCoreAgentApiState,
    principal: PrincipalRef,
    model: nomifun_common::ProviderWithModel,
    private_model: nomifun_common::ProviderWithModel,
    wide_preset: String,
    companion_id: String,
    upstream: MockServer,
}

impl Fixture {
    async fn new() -> Self {
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"id\":\"guest-fixture\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Fixture reply\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"id\":\"guest-fixture\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n")))
            .mount(&upstream).await;
        let root = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(root.path()).unwrap();
        let config = crate::AppConfig {
            data_dir: base.join("data"),
            work_dir: base.join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(TRUST.into()),
            ..Default::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        let database = nomifun_db::init_database(&config.database_path())
            .await
            .unwrap();
        let services = crate::services::AppServices::from_config(database, &config)
            .await
            .unwrap();
        let corpus_version =
            nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
        nomifun_skill_library::materialize_if_needed(
            &services.data_dir,
            nomifun_skill_library::builtin_skills_corpus(),
            &corpus_version,
        )
        .await
        .unwrap();
        let (states, _components) = super::super::super::state::try_build_module_states(&services)
            .await
            .unwrap();
        let owner = states.nomi_core_agent_api.session_owner.clone();
        let api = states.nomi_core_agent_api.clone();
        let router = super::super::super::create_router_with_states(&services, states);
        let principal = PrincipalRef {
            principal_kind: "user".into(),
            principal_id: services.authoritative_user_id.as_ref().into(),
        };
        let model = provider(&router, &upstream, "guest-fixture").await;
        let private_model = provider(&router, &upstream, "private-fixture").await;
        let companion = call(
            &router,
            "POST",
            "/api/companion/companions",
            json!({"name":"PRIVATE_PAIRED_COMPANION_PERSONA","character":"mochi"}),
        )
        .await;
        let companion_id = companion["companion_id"].as_str().unwrap().to_owned();
        call(
            &router,
            "PATCH",
            &format!("/api/companion/companions/{companion_id}"),
            json!({"model":private_model}),
        )
        .await;
        services
            .companion_service
            .add_memory("preference", PRIVATE_MEMORY, &[], Some(&companion_id))
            .await
            .unwrap();
        let launch = call(
            &router,
            "POST",
            "/api/agent-presets/from-template/chat.minimal",
            json!({"reuse_existing":true,"display_name":"chat.minimal","model":private_model}),
        )
        .await;
        let library = call(&router, "GET", "/api/agent-preset-templates", json!({})).await;
        let wide_preset = library["official_presets"]["chat.minimal"]["preset_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let editor = call(
            &router,
            "GET",
            &format!("/api/agent-presets/{wide_preset}/editor"),
            json!({}),
        )
        .await;
        let mut draft = editor["draft"].clone();
        draft["document"]["persona"] = json!(PRIVATE_PERSONA);
        draft["document"]["instructions"] = json!(PRIVATE_INSTRUCTIONS);
        draft["document"]["enabled_capabilities"] = json!([
            {"capability":{"id":"companion"},"action_allowlist":["companion/evolve","companion/learn"]},
            {"capability":{"id":"companion.memory"},"action_allowlist":["companion.memory/recall","companion.memory/write"]},
            {"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read","workspace.files/write"]},
        ]);
        for field in ["model_route_refs", "chat_route_records"] {
            draft["document"][field] = launch["draft"]["document"][field].clone();
        }
        call(
            &router,
            "POST",
            &format!("/api/agent-presets/{wide_preset}/revisions"),
            json!({
                "expected_current_revision":editor["draft"]["current_revision"],"draft":draft,
            }),
        )
        .await;
        Self {
            _root: root,
            services,
            router,
            owner,
            api,
            principal,
            model,
            private_model,
            wide_preset,
            companion_id,
            upstream,
        }
    }

    fn request(
        &self,
        model: &nomifun_common::ProviderWithModel,
        extra: Value,
    ) -> CreateConversationRequest {
        CreateConversationRequest {
            r#type: nomifun_common::AgentType::Nomi,
            name: Some("Channel fixture".into()),
            model: Some(model.clone()),
            source: None,
            channel_chat_id: Some("fixture-group".into()),
            preset_id: None,
            delegation_policy: Default::default(),
            execution_model_pool: None,
            decision_policy: Default::default(),
            execution_template_id: None,
            extra,
        }
    }

    async fn relationship(&self, authorization: &str) -> ChannelSessionRow {
        let user = Uuid::now_v7().to_string();
        let id = Uuid::now_v7().to_string();
        let chat = Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO channel_users(channel_user_id,platform_user_id,platform_type,authorization_kind,authorized_at) VALUES(?,?,'telegram',?,1)")
            .bind(&user).bind(format!("external-{user}")).bind(authorization).execute(&self.owner.pool).await.unwrap();
        sqlx::query("INSERT INTO channel_sessions(channel_session_id,channel_user_id,agent_type,chat_id,chat_kind,created_at,last_activity) VALUES(?,?,'nomi',?,'group',1,1)")
            .bind(&id).bind(&user).bind(chat).execute(&self.owner.pool).await.unwrap();
        self.row(&id).await
    }

    async fn row(&self, id: &str) -> ChannelSessionRow {
        sqlx::query_as("SELECT * FROM channel_sessions WHERE channel_session_id=?")
            .bind(id)
            .fetch_one(&self.owner.pool)
            .await
            .unwrap()
    }

    async fn bind(&self, row: &ChannelSessionRow, session: &str) {
        sqlx::query("UPDATE channel_sessions SET conversation_id=? WHERE channel_session_id=?")
            .bind(session)
            .bind(&row.channel_session_id)
            .execute(&self.owner.pool)
            .await
            .unwrap();
    }

    fn opening(&self, row: &ChannelSessionRow) -> String {
        nomifun_channel::message_service::group_guest_creation_key(
            &self.principal.principal_id,
            row,
        )
    }

    async fn wide_extra(&self) -> Value {
        let resolver = self
            .owner
            .product_agent_resolver
            .get()
            .and_then(std::sync::Weak::upgrade)
            .unwrap();
        let owner = UserId::from(self.principal.principal_id.clone());
        let selected = AgentChatModelSelectionDto {
            provider_id: self.private_model.provider_id.clone(),
            model: self.private_model.model.clone(),
        };
        let binding = resolver
            .control_plane
            .resolve_agent_session_binding_with_model(&owner, &self.wide_preset, Some(&selected))
            .await
            .unwrap();
        let binding = resolver
            .resource_bindings
            .resolve_for_saved_binding(
                &resolver.control_plane,
                &owner,
                binding,
                &[
                    AgentResourceSelectionDto {
                        resource_kind: "companion".into(),
                        resource_id: self.companion_id.clone(),
                    },
                    AgentResourceSelectionDto {
                        resource_kind: "companion_memory".into(),
                        resource_id: self.companion_id.clone(),
                    },
                    AgentResourceSelectionDto {
                        resource_kind: "workspace".into(),
                        resource_id: "default-workspace".into(),
                    },
                ],
            )
            .await
            .unwrap();
        let binding: AgentBindingValue = serde_json::to_value(binding)
            .and_then(serde_json::from_value)
            .unwrap();
        let workspace = self._root.path().join("owner-workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut extra = json!({"workspace":workspace.to_string_lossy()});
        attach_session_metadata(&mut extra, &binding, None).unwrap();
        extra
    }

    async fn wide(&self, row: &ChannelSessionRow, key: &str) -> ConversationResponse {
        let created = ChannelSessionPort::create_idempotent(
            self.owner.as_ref(),
            &self.principal.principal_id,
            self.request(&self.private_model, self.wide_extra().await),
            key,
        )
        .await
        .unwrap();
        self.bind(row, &created.conversation_id).await;
        created
    }

    async fn counts(&self, session: &AgentSessionId) -> (i64, i64, i64) {
        sqlx::query_as("SELECT SUM(kind='session/agent-binding-changed'),SUM(kind='context/cleared'),SUM(kind='message/user-accepted') FROM agent_events WHERE session_id=?")
            .bind(session.as_ref()).fetch_one(&self.owner.pool).await.unwrap()
    }

    async fn completed(&self, session: &str, key: &str) {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let nomifun_channel::ChannelTurnReceiptState::Completed(receipt) =
                    ChannelSessionPort::turn_outcome(
                        self.owner.as_ref(),
                        &self.principal.principal_id,
                        session,
                        key,
                    )
                    .await
                    .unwrap()
                {
                    assert_eq!(receipt.result_ok, Some(true), "{receipt:?}");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the real guest Runtime/Broker Turn must close");
    }

    async fn accepted_origin(&self, session: &AgentSessionId, key: &str) -> Option<String> {
        let operation = if let Some(input) = self.owner.canonical
            .session_input_by_key(&self.principal, session, key).await.unwrap() {
            self.owner.canonical.store().session_input_claim(session, &input.event.event_id)
                .await.unwrap().expect("the admitted ordinary input must have its exact claim").operation_id
        } else {
            // Model-only guest ingress has its own current direct admission;
            // approved ordinary input uses the SessionInput claim above.
            NomiCoreSessionOwner::turn_operation_id(&self.principal.principal_id, session.as_ref(), key)
        };
        self.owner
            .canonical
            .store()
            .turn_accepted_input(session, &operation)
            .await
            .unwrap()
            .1
            .0
            .get("origin")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    async fn finish(self) {
        let _ = self
            .services
            .cleanup_after_startup_failure(anyhow::anyhow!("channel guest fixture complete"))
            .await;
    }
}

fn message(content: &str) -> SendMessageRequest {
    SendMessageRequest {
        content: content.into(),
        files: vec![],
        inject_skills: vec![],
        hidden: false,
        origin: None,
        channel_platform: Some("telegram".into()),
    }
}

async fn provider(
    router: &Router,
    upstream: &MockServer,
    model: &str,
) -> nomifun_common::ProviderWithModel {
    let provider=call(router,"POST","/api/providers",json!({
        "platform":"custom","name":model,"base_url":format!("{}/v1",upstream.uri()),
        "auth_scheme":"bearer","credentials":{"api_keys":["fixture-only"]},"enabled":true,
        "initial_model":{"model":model,"enabled":true,"capabilities":[{
            "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default",
        }]}
    })).await;
    nomifun_common::ProviderWithModel {
        provider_id: provider["provider_id"].as_str().unwrap().into(),
        model: model.into(),
        use_model: None,
    }
}

async fn response(router: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

async fn call(router: &Router, method: &str, uri: &str, body: Value) -> Value {
    let (status, value) = response(router, method, uri, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {value}");
    value["data"].clone()
}

fn assert_model_only_wire(body: &Value, model: &str) {
    assert_eq!(body["model"], model);
    assert!(
        body.get("tools")
            .is_none_or(|tools| tools.is_null() || tools.as_array().is_some_and(Vec::is_empty)),
        "a guest has no SDK tool definitions: {body}"
    );
    let messages = body["messages"].to_string();
    assert!(!messages.contains("Current historical reader is available"),
        "guest requests cannot advertise an unavailable historical reader: {messages}");
    for private in [
        PRIVATE_PERSONA,
        PRIVATE_INSTRUCTIONS,
        PRIVATE_MEMORY,
        PRIVATE_HISTORY,
        "PRIVATE_PAIRED_COMPANION_PERSONA",
    ] {
        assert!(
            !messages.contains(private),
            "private owner data reached the guest model: {private}: {messages}"
        );
    }
}

async fn assert_guest_control_refused(f: &Fixture, session: &AgentSessionId, opening: &str, tool: &str, args: Value) {
    let marker = format!("PROVIDER_FORGED_GUEST_CONTROL_{tool}");
    let response = json!({"id":"forged-guest-control","choices":[{"index":0,"delta":{"tool_calls":[{
        "index":0,"id":format!("forged-{tool}"),"type":"function","function":{"name":tool,"arguments":args.to_string()}
    }]},"finish_reason":null}]});
    let terminal = json!({"id":"forged-guest-control","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
    Mock::given(method("POST")).and(path("/v1/chat/completions")).and(body_string_contains(marker.clone()))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
            .set_body_string(format!("data: {response}\n\ndata: {terminal}\n\ndata: [DONE]\n\n")))
        .with_priority(1).up_to_n_times(1).mount(&f.upstream).await;
    let before = f.upstream.received_requests().await.unwrap().len();
    let key = format!("forged-guest-{tool}");
    ChannelSessionPort::send_group_guest_turn(f.owner.as_ref(),&f.principal.principal_id,
        session.as_ref(),&key,message(&marker),opening).await.unwrap();
    let delivery = tokio::time::timeout(Duration::from_secs(15),async { loop {
        if let PublicTurnDeliveryState::Completed(delivery) = f.owner.session_turn_delivery_state(
            &f.principal.principal_id,session.as_ref(),&key).await.unwrap() { break delivery; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("a fabricated control must be refused within the original bounded Turn");
    assert!(delivery.user_input_request.is_none(),"an unadvertised control cannot create a guest input handoff");
    assert!(!matches!(delivery.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired { .. })));
    assert_ne!(delivery.result_ok,Some(true),"a provider violating the pure-chat tool surface must not be reported as successful execution");
    let requests = f.upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(),before+1,"pure-chat guests never open a second model step to repair an unsupported tool proposal");
    for request in requests.iter().skip(before) { assert_model_only_wire(&request.body_json::<Value>().unwrap(),"private-fixture"); }
    let operation = NomiCoreSessionOwner::turn_operation_id(&f.principal.principal_id,session.as_ref(),&key);
    let facts = f.owner.canonical.store().turn_output_facts(session,&operation).await.unwrap();
    assert!(facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded").any(|event| {
        let Some(value)=facts.event_payloads.get(event.event_id.as_ref()).and_then(|value|value.get("event")) else { return false; };
        matches!(serde_json::from_value::<nomifun_agent_runtime::AgentEngineEvent>(value.clone()),
            Ok(nomifun_agent_runtime::AgentEngineEvent::ToolCompleted { result, .. })
                if result.call_id.as_ref()==format!("forged-{tool}") && result.is_error && result.output_text().contains("Not executed"))
    }),"canonical ToolCompleted must record the actual dispatch refusal; hiding the schema alone is insufficient");
}

#[tokio::test]
async fn channel_guest_new_session_uses_the_requested_model_and_an_empty_real_sdk_ceiling() {
    let f = Fixture::new().await;
    let row = f.relationship("auto_group").await;
    let opening = f.opening(&row);
    let mut forged = f.wide_extra().await;
    forged["system_prompt"] = json!(PRIVATE_INSTRUCTIONS);
    forged["companion_session"] = json!(true);
    forged["companion_id"] = json!(f.companion_id);
    let guest = ChannelSessionPort::create_group_guest_idempotent(
        f.owner.as_ref(),
        &f.principal.principal_id,
        f.request(&f.model, forged),
        &opening,
    )
    .await
    .unwrap();
    f.bind(&row, &guest.conversation_id).await;
    let session = AgentSessionId::from(guest.conversation_id.clone());
    let control = f
        .owner
        .runtime_control_plane
        .get()
        .and_then(std::sync::Weak::upgrade)
        .unwrap();
    let saved = f
        .owner
        .canonical
        .get(&f.principal, &session)
        .await
        .unwrap()
        .session;
    control
        .validate_channel_guest_model_only_binding(
            &UserId::from(f.principal.principal_id.clone()),
            &agent_binding_dto(&saved.agent_binding).unwrap(),
        )
        .await
        .unwrap();
    assert!(saved.agent_binding.typed_resource_bindings.is_empty());
    let before_memory = f
        .services
        .companion_service
        .companion_status(&f.companion_id)
        .await
        .unwrap();
    let delivery = ChannelSessionPort::send_group_guest_turn(
        f.owner.as_ref(),
        &f.principal.principal_id,
        session.as_ref(),
        "new-guest-input",
        message("Hello from the open group"),
        &opening,
    )
    .await
    .unwrap();
    assert!(!delivery.delivery.replayed);
    f.completed(session.as_ref(), "new-guest-input").await;
    assert_eq!(
        f.accepted_origin(&session, "new-guest-input")
            .await
            .as_deref(),
        Some("channel_guest")
    );
    assert_eq!(f.counts(&session).await, (0, 0, 1));
    // The ordinary idle preparation boundary must not reinterpret this
    // host-owned empty chat.minimal variant as a retired personal Agent.
    // Keep the real guest contract before the next actual SDK/Broker Turn.
    {
        let _idle = f.owner.session_operation_lock(session.as_ref()).write_owned().await;
        f.owner
            .prepare_session_contract_evolution(&f.principal.principal_id, &session)
            .await
            .unwrap();
        let prepared = f.owner.canonical.get(&f.principal, &session).await.unwrap();
        assert_eq!(prepared.head.status, "ready");
        assert_eq!(prepared.session.agent_binding, saved.agent_binding);
        control
            .validate_channel_guest_model_only_binding(
                &UserId::from(f.principal.principal_id.clone()),
                &agent_binding_dto(&prepared.session.agent_binding).unwrap(),
            )
            .await
            .unwrap();
    }
    assert_eq!(f.counts(&session).await, (0, 0, 1), "idle preparation must not add a guest OS contract transition");
    let requests = f.upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_model_only_wire(&requests[0].body_json::<Value>().unwrap(), "guest-fixture");
    assert_eq!(
        ChannelSessionPort::find_created_session(
            f.owner.as_ref(),
            &f.principal.principal_id,
            &opening
        )
        .await
        .unwrap(),
        Some(session.as_ref().to_owned())
    );
    let same = ChannelSessionPort::create_group_guest_idempotent(
        f.owner.as_ref(),
        &f.principal.principal_id,
        f.request(&f.model, json!({})),
        &opening,
    )
    .await
    .unwrap();
    assert_eq!(
        same.conversation_id,
        session.as_ref(),
        "the original tier opening reuses the same guest Session"
    );
    ChannelSessionPort::send_group_guest_turn(
        f.owner.as_ref(),
        &f.principal.principal_id,
        session.as_ref(),
        "new-guest-input-2",
        message("A second input in the same open group"),
        &opening,
    )
    .await
    .unwrap();
    f.completed(session.as_ref(), "new-guest-input-2").await;
    let replay = ChannelSessionPort::send_group_guest_turn(
        f.owner.as_ref(),
        &f.principal.principal_id,
        session.as_ref(),
        "new-guest-input",
        message("Hello from the open group"),
        &opening,
    )
    .await
    .unwrap();
    assert!(
        replay.delivery.replayed && replay.delivery.completed,
        "a fresh same-empty admission replays its original receipt"
    );
    assert_eq!(f.counts(&session).await, (0, 0, 2));
    let requests = f.upstream.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        2,
        "exact replay never sends a third SDK request"
    );
    assert_model_only_wire(&requests[1].body_json::<Value>().unwrap(), "guest-fixture");
    let after_memory = f
        .services
        .companion_service
        .companion_status(&f.companion_id)
        .await
        .unwrap();
    assert_eq!(after_memory.memories_active, before_memory.memories_active);
    assert_eq!(
        after_memory.memories_archived,
        before_memory.memories_archived
    );
    let mut spoof = message("Do not accept a forged guest source");
    spoof.origin = Some("channel_guest".into());
    assert!(
        ChannelSessionPort::send_turn(
            f.owner.as_ref(),
            &f.principal.principal_id,
            session.as_ref(),
            "spoof-source",
            spoof
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.counts(&session).await,
        (0, 0, 2),
        "ordinary APIs cannot author reserved guest input provenance"
    );
    assert_eq!(f.upstream.received_requests().await.unwrap().len(), 2);
    let approved_scope = json!([
        f.principal.principal_id,
        row.channel_plugin_id.as_deref().unwrap_or(""),
        row.channel_session_id,
        row.agent_type,
        "dedicated-group-approved"
    ]);
    let approved_opening = format!(
        "channel-session:v1:{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&approved_scope).unwrap())
    );
    assert_ne!(approved_opening, opening);
    assert!(
        ChannelSessionPort::find_created_session(
            f.owner.as_ref(),
            &f.principal.principal_id,
            &approved_opening
        )
        .await
        .unwrap()
        .is_none(),
        "a guest opening cannot satisfy an approved tier lookup"
    );
    sqlx::query("UPDATE channel_users SET authorization_kind='approved' WHERE channel_user_id=?")
        .bind(&row.channel_user_id)
        .execute(&f.owner.pool)
        .await
        .unwrap();
    let approved = f.wide(&row, &approved_opening).await;
    assert_ne!(
        approved.conversation_id,
        session.as_ref(),
        "a different admission tier gets its own canonical opening"
    );
    assert_eq!(
        ChannelSessionPort::find_created_session(
            f.owner.as_ref(),
            &f.principal.principal_id,
            &approved_opening
        )
        .await
        .unwrap(),
        Some(approved.conversation_id.clone())
    );
    assert_eq!(
        ChannelSessionPort::find_created_session(
            f.owner.as_ref(),
            &f.principal.principal_id,
            &opening
        )
        .await
        .unwrap(),
        Some(session.as_ref().to_owned()),
        "the original guest tier remains separately recoverable"
    );
    assert_eq!(
        f.owner
            .canonical
            .get(&f.principal, &session)
            .await
            .unwrap()
            .session
            .agent_binding,
        saved.agent_binding
    );
    assert_eq!(f.counts(&session).await, (0, 0, 2));
    f.finish().await;
}

#[tokio::test]
async fn channel_guest_old_wide_idle_session_narrows_once_clears_context_and_preserves_history() {
    let f = Fixture::new().await;
    let row = f.relationship("approved").await;
    let opening = f.opening(&row);
    let old = f.wide(&row, &opening).await;
    let session = AgentSessionId::from(old.conversation_id.clone());
    let original = f
        .owner
        .canonical
        .get(&f.principal, &session)
        .await
        .unwrap()
        .session
        .agent_binding;
    ChannelSessionPort::send_turn(
        f.owner.as_ref(),
        &f.principal.principal_id,
        session.as_ref(),
        "approved-history",
        message(PRIVATE_HISTORY),
    )
    .await
    .unwrap();
    f.completed(session.as_ref(), "approved-history").await;
    assert_eq!(
        f.owner
            .canonical
            .get(&f.principal, &session)
            .await
            .unwrap()
            .session
            .agent_binding,
        original,
        "an approved group keeps its existing owner-authorized Agent"
    );
    let requests = f.upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let approved = requests[0].body_json::<Value>().unwrap();
    assert_eq!(approved["model"], "private-fixture");
    assert!(
        approved["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty()),
        "the legacy fixture really has tools"
    );
    let approved_context = approved["messages"].to_string();
    for private in [PRIVATE_PERSONA, PRIVATE_INSTRUCTIONS, PRIVATE_MEMORY] {
        assert!(
            approved_context.contains(private),
            "approved baseline must prove the actual private context exists: {private}"
        );
    }
    assert_eq!(f.accepted_origin(&session, "approved-history").await, None);
    let idmm = f
        .owner
        .idmm
        .get()
        .and_then(std::sync::Weak::upgrade)
        .expect("real IDMM service");
    let idmm_path = format!("/api/agent-sessions/{}/idmm", session.as_ref());
    let enabled = nomifun_api_types::IdmmConfig {
        mode: nomifun_api_types::IdmmMode::RuleOnly,
        ..Default::default()
    };
    let configured = call(
        &f.router,
        "PUT",
        &idmm_path,
        serde_json::to_value(&enabled).unwrap(),
    )
    .await;
    assert_eq!(
        configured["config"]["mode"], "rule_only",
        "normal approved Sessions retain IDMM configuration support"
    );
    assert_eq!(
        idmm.state(session.as_ref()).await.unwrap().config.mode,
        nomifun_api_types::IdmmMode::RuleOnly
    );
    let approved_evaluation = call(
        &f.router,
        "POST",
        &format!("{idmm_path}/evaluate"),
        json!({}),
    )
    .await;
    assert_eq!(
        approved_evaluation["config"]["mode"], "rule_only",
        "ordinary approved supervision still observes its own Session"
    );
    let history = f
        .owner
        .canonical
        .store()
        .read_events(&session, None, 500)
        .await
        .unwrap()
        .events;
    sqlx::query("UPDATE channel_users SET authorization_kind='auto_group' WHERE channel_user_id=?")
        .bind(&row.channel_user_id)
        .execute(&f.owner.pool)
        .await
        .unwrap();
    assert!(
        f.owner
            .validate_registered_channel_guest_runtime(&f.principal.principal_id, &session)
            .await
            .is_err(),
        "warmup/recovery cannot run a registered old broad guest before adoption"
    );
    let adopted = ChannelSessionPort::create_group_guest_idempotent(
        f.owner.as_ref(),
        &f.principal.principal_id,
        f.request(&f.model, json!({})),
        &opening,
    )
    .await
    .unwrap();
    assert_eq!(adopted.conversation_id, old.conversation_id);
    assert_eq!(
        idmm.state(session.as_ref()).await.unwrap().config.mode,
        nomifun_api_types::IdmmMode::Off,
        "guest adoption removes a real mutable IDMM override in addition to narrowing the frozen Agent"
    );
    let guest_idmm = call(&f.router, "GET", &idmm_path, json!({})).await;
    assert_eq!(guest_idmm["config"]["mode"], "off");
    let (status, rejected) = response(
        &f.router,
        "PUT",
        &idmm_path,
        serde_json::to_value(&enabled).unwrap(),
    )
    .await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::CONFLICT),
        "guest IDMM enable must be rejected by its repository role: {status} {rejected}"
    );
    assert_eq!(
        call(&f.router, "GET", &idmm_path, json!({})).await["config"]["mode"],
        "off"
    );
    let (status, rejected) = response(
        &f.router,
        "POST",
        &format!("{idmm_path}/evaluate"),
        json!({}),
    )
    .await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::CONFLICT),
        "public evaluation cannot supervise a guest: {status} {rejected}"
    );
    // The observer cannot read a repository-classified guest's old messages or
    // delete a potentially misbound owner's mutable configuration by guessing.
    assert_eq!(
        idmm.set_config(session.as_ref(), enabled)
            .await
            .unwrap()
            .config
            .mode,
        nomifun_api_types::IdmmMode::RuleOnly
    );
    assert!(idmm.evaluate_now(session.as_ref()).await.is_err());
    assert_eq!(
        idmm.state(session.as_ref()).await.unwrap().config.mode,
        nomifun_api_types::IdmmMode::RuleOnly,
        "observe rejection itself cannot delete a mutable owner override"
    );
    ChannelSessionPort::create_group_guest_idempotent(
        f.owner.as_ref(),
        &f.principal.principal_id,
        f.request(&f.model, json!({})),
        &opening,
    )
    .await
    .unwrap();
    let settled_idmm = idmm.state(session.as_ref()).await.unwrap();
    assert_eq!(
        settled_idmm.config.mode,
        nomifun_api_types::IdmmMode::Off,
        "only the formally verified dedicated guest boundary may remove its stale override"
    );
    assert!(settled_idmm.recent_interventions.is_empty());
    let count_before_idmm = f.counts(&session).await;
    let mut inflight = message("A stale supervisor must not inject this Turn");
    inflight.origin = Some("idmm".into());
    let decision = nomifun_agent_contracts::IdmmDecisionExplanation {
        intervention_id: Uuid::now_v7().to_string(),
        source: nomifun_agent_contracts::IdmmDecisionSource::Recovery,
        reason_code: "provider_failure_recovery".into(),
        rationale: "Retry the known failed request".into(),
        model: None,
        question: None,
    };
    decision.validate().unwrap();
    assert!(
        f.owner
            .send_session_idmm_message_idempotent(
                &f.principal.principal_id,
                session.as_ref(),
                "stale-idmm-delivery",
                inflight,
                decision
            )
            .await
            .is_err(),
        "an already in-flight supervisor callback must recheck the guest role"
    );
    assert_eq!(
        f.counts(&session).await,
        count_before_idmm,
        "rejected IDMM delivery creates no accepted input"
    );
    assert_eq!(
        f.upstream.received_requests().await.unwrap().len(),
        1,
        "IDMM checks never send another guest or bypass SDK request"
    );
    let narrowed = f
        .owner
        .canonical
        .get(&f.principal, &session)
        .await
        .unwrap()
        .session
        .agent_binding;
    assert_eq!(narrowed.binding_version, original.binding_version + 1);
    assert!(narrowed.typed_resource_bindings.is_empty());
    assert_eq!(f.counts(&session).await, (1, 1, 1));
    let retained = f
        .owner
        .canonical
        .store()
        .read_events(&session, None, 500)
        .await
        .unwrap()
        .events;
    assert!(
        history.iter().all(|fact| retained.contains(fact)),
        "normal CAS/clear retains every canonical history fact"
    );
    f.owner
        .validate_registered_channel_guest_runtime(&f.principal.principal_id, &session)
        .await
        .unwrap();
    ChannelSessionPort::send_group_guest_turn(
        f.owner.as_ref(),
        &f.principal.principal_id,
        session.as_ref(),
        "after-guest-adoption",
        message("Hello with the guest ceiling"),
        &opening,
    )
    .await
    .unwrap();
    f.completed(session.as_ref(), "after-guest-adoption").await;
    let requests = f.upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_model_only_wire(
        &requests[1].body_json::<Value>().unwrap(),
        "private-fixture",
    );
    assert_eq!(
        f.accepted_origin(&session, "after-guest-adoption")
            .await
            .as_deref(),
        Some("channel_guest")
    );
    let count = f.counts(&session).await;
    ChannelSessionPort::create_group_guest_idempotent(
        f.owner.as_ref(),
        &f.principal.principal_id,
        f.request(&f.model, json!({})),
        &opening,
    )
    .await
    .unwrap();
    let replay = ChannelSessionPort::send_group_guest_turn(
        f.owner.as_ref(),
        &f.principal.principal_id,
        session.as_ref(),
        "approved-history",
        message(PRIVATE_HISTORY),
        &opening,
    )
    .await;
    assert!(
        replay.is_err(),
        "a broad completed result before the guest clear cannot be delivered to the group"
    );
    let original_input=f.owner.canonical.session_input_by_key(&f.principal,&session,"approved-history").await.unwrap().unwrap();
    assert!(ChannelSessionPort::observe_input(f.owner.as_ref(),&f.principal.principal_id,session.as_ref(),
        original_input.event.event_id.as_ref()).await.is_err(),"input receipt hydration must enforce the same guest history boundary");
    assert!(ChannelSessionPort::turn_outcome(f.owner.as_ref(),&f.principal.principal_id,session.as_ref(),
        "approved-history").await.is_err(),"legacy receipt hydration cannot expose the original broad reply");
    let owner_ack=ChannelSessionPort::send_turn(f.owner.as_ref(),&f.principal.principal_id,session.as_ref(),
        "approved-history",message(PRIVATE_HISTORY)).await.unwrap();
    assert_eq!(owner_ack.delivery.message_id,original_input.event.event_id.as_ref());
    assert!(owner_ack.delivery.replayed&&!owner_ack.delivery.completed,
        "the ordinary owner still receives the original acceptance without replaying work or returning its old result to a guest");
    assert!(matches!(f.owner.session_turn_delivery_state(&f.principal.principal_id,session.as_ref(),"approved-history").await.unwrap(),
        PublicTurnDeliveryState::Completed(_)),"owner-only history remains readable independently of channel delivery authority");
    assert_eq!(
        f.accepted_origin(&session, "approved-history").await,
        None,
        "old accepted roots retain their exact original source on replay"
    );
    assert_eq!(
        f.counts(&session).await,
        count,
        "ensure/replay adds neither a transition, clear nor accepted input"
    );
    assert_eq!(
        f.owner
            .canonical
            .get(&f.principal, &session)
            .await
            .unwrap()
            .session
            .agent_binding,
        narrowed
    );
    assert_eq!(f.upstream.received_requests().await.unwrap().len(), 2);
    assert_guest_control_refused(&f,&session,&opening,"load_tool_history",json!({})).await;
    assert_guest_control_refused(&f,&session,&opening,"request_user_input",json!({
        "question":"An unadvertised provider proposal must not become a real question",
        "remaining_work":"No guest task may acquire an input-handoff control",
    })).await;
    f.finish().await;
}

#[tokio::test]
async fn channel_guest_active_or_unknown_effect_sessions_cannot_adopt_a_narrower_binding() {
    for unknown in [false, true] {
        let f = Fixture::new().await;
        let row = f.relationship("approved").await;
        let opening = f.opening(&row);
        let old = f.wide(&row, &opening).await;
        let session = AgentSessionId::from(old.conversation_id.clone());
        let original = f
            .owner
            .canonical
            .get(&f.principal, &session)
            .await
            .unwrap()
            .session
            .agent_binding;
        let operation = NomiCoreSessionOwner::turn_operation_id(
            &f.principal.principal_id,
            session.as_ref(),
            "unsettled-input",
        );
        let input = f
            .owner
            .canonical_turn_input_with_admission(
                &f.principal.principal_id,
                &session,
                &operation,
                &message("Unsettled legacy work"),
            )
            .await
            .unwrap();
        f.owner
            .canonical
            .start_turn(&f.principal, &session, "unsettled-input", input)
            .await
            .unwrap();
        let store = f.owner.canonical.store();
        let started = store
            .read_turn_receipt(&session, &operation)
            .await
            .unwrap()
            .started_event
            .unwrap()
            .event_id;
        if unknown {
            let tool = Uuid::now_v7().to_string();
            let cause=store.append_event(&SessionEventAppend {agent_session_id:session.clone(),event_id:tool.clone().into(),
                producer_id:"runtime_supervisor".into(),idempotency_key:tool.into(),semantic_event:SemanticSessionEventDraft {
                    kind:SessionEventKind("tool/call-started".into()),kind_version:1,correlation_id:"legacy-write-operation".into(),
                    causation_event_id:Some(started.clone()),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "operation_id":"legacy-write-operation","call_id":"legacy-write-call","capability_id":"workspace.files","action_id":"workspace.files/write",
                    })))}}).await.unwrap().ack.unwrap().event_id;
            let effect = EffectEventRequest {
                agent_session_id: session.clone(),
                effect_id: "unknown-legacy-write".into(),
                turn_id: operation.clone(),
                operation_id: "legacy-write-operation".into(),
                owner_domain: "workspace".into(),
                capability_module: "workspace.files".into(),
                action_id: "workspace.files/write".into(),
                resource_binding_id: None,
                resource_key: None,
                input_digest: "8".repeat(64).into(),
                recorded_at: 2000,
                event_id: "legacy-effect-started".into(),
                producer_id: "capability_host".into(),
                idempotency_key: "legacy-write-lifecycle".into(),
                correlation_id: "unknown-legacy-write".into(),
                strategy: EffectStrategy::ManagedEffect,
                causation_event_id: Some(cause),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
            };
            let effect_started = store
                .record_effect_started(effect.clone())
                .await
                .unwrap()
                .ack
                .unwrap()
                .event_id;
            store
                .record_effect_terminal(
                    EffectEventRequest {
                        event_id: "legacy-write-uncertain".into(),
                        recorded_at: 2010,
                        producer_id: "runtime_supervisor".into(),
                        causation_event_id: Some(effect_started),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                            json!({"recovery":"native_recovery_external_reconciliation_required"}),
                        )),
                        ..effect
                    },
                    EffectTerminalState::Uncertain,
                )
                .await
                .unwrap();
            f.owner
                .canonical
                .cancel_exact_turn(&f.principal, &session, "legacy-turn-cancelled", &operation)
                .await
                .unwrap();
            assert!(store.head(&session).await.unwrap().active_turn_id.is_none());
            assert!(store.has_unsettled_effects(&session).await.unwrap());
        }
        sqlx::query(
            "UPDATE channel_users SET authorization_kind='auto_group' WHERE channel_user_id=?",
        )
        .bind(&row.channel_user_id)
        .execute(&f.owner.pool)
        .await
        .unwrap();
        assert!(
            ChannelSessionPort::create_group_guest_idempotent(
                f.owner.as_ref(),
                &f.principal.principal_id,
                f.request(&f.model, json!({})),
                &opening
            )
            .await
            .is_err()
        );
        assert_eq!(
            f.owner
                .canonical
                .get(&f.principal, &session)
                .await
                .unwrap()
                .session
                .agent_binding,
            original
        );
        assert_eq!(
            f.counts(&session).await,
            (0, 0, 1),
            "blocked adoption never clears history or rewrites the binding"
        );
        assert!(f.upstream.received_requests().await.unwrap().is_empty());
        f.finish().await;
    }
}

#[tokio::test]
async fn channel_guest_extra_flags_cannot_repurpose_an_ordinary_owner_session() {
    let f = Fixture::new().await;
    let row = f.relationship("approved").await;
    let mut extra = f.wide_extra().await;
    extra["channel_group_guest"] = json!(true);
    extra["authorization_kind"] = json!("auto_group");
    let ordinary = ChannelSessionPort::create_idempotent(
        f.owner.as_ref(),
        &f.principal.principal_id,
        f.request(&f.private_model, extra),
        &format!("ordinary-owner:{}", Uuid::now_v7()),
    )
    .await
    .unwrap();
    let session = AgentSessionId::from(ordinary.conversation_id.clone());
    let original = f
        .owner
        .canonical
        .get(&f.principal, &session)
        .await
        .unwrap()
        .session
        .agent_binding;
    f.owner
        .validate_registered_channel_guest_runtime(&f.principal.principal_id, &session)
        .await
        .unwrap();
    assert_eq!(
        f.counts(&session).await,
        (0, 0, 0),
        "presentation flags create no guest transition or context clear"
    );
    f.bind(&row, session.as_ref()).await;
    let idmm = f
        .owner
        .idmm
        .get()
        .and_then(std::sync::Weak::upgrade)
        .expect("real IDMM service");
    idmm.set_config(
        session.as_ref(),
        nomifun_api_types::IdmmConfig {
            mode: nomifun_api_types::IdmmMode::RuleOnly,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE channel_users SET authorization_kind='auto_group' WHERE channel_user_id=?")
        .bind(&row.channel_user_id)
        .execute(&f.owner.pool)
        .await
        .unwrap();
    let opening = f.opening(&row);
    assert!(
        f.owner
            .validate_registered_channel_guest_runtime(&f.principal.principal_id, &session)
            .await
            .is_err()
    );
    assert!(
        ChannelSessionPort::send_group_guest_turn(
            f.owner.as_ref(),
            &f.principal.principal_id,
            session.as_ref(),
            "forged-input",
            message("Do not repurpose an owner Session"),
            &opening
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.owner
            .canonical
            .get(&f.principal, &session)
            .await
            .unwrap()
            .session
            .agent_binding,
        original
    );
    assert_eq!(f.counts(&session).await, (0, 0, 0));
    assert!(
        idmm.evaluate_now(session.as_ref()).await.is_err(),
        "a wrong guest repository link cannot authorize private message observation"
    );
    assert_eq!(
        idmm.state(session.as_ref()).await.unwrap().config.mode,
        nomifun_api_types::IdmmMode::RuleOnly,
        "neither the observer nor a rejected guest send may delete the ordinary owner's configuration"
    );
    assert!(f.upstream.received_requests().await.unwrap().is_empty());
    f.finish().await;
}


#[tokio::test]
async fn unavailable_unclaimed_input_ends_owner_channel_robot_and_remote_observation_without_a_turn() {
    use nomifun_conversation::UnavailableInputCause;
    use nomifun_agent_session::SessionInputBlockedReason;
    let f=Fixture::new().await;
    for (key,reason,expected) in [
        ("blocked-before-claim",SessionInputBlockedReason::SkillUnavailable,
            UnavailableInputCause::PreparationBlocked {reason:"SKILL_UNAVAILABLE".into()}),
        ("cancel-before-claim",SessionInputBlockedReason::CancelledByOwner,UnavailableInputCause::CancelledByOwner),
    ] {
        let row=f.relationship("auto_group").await;
        let created=ChannelSessionPort::create_group_guest_idempotent(f.owner.as_ref(),&f.principal.principal_id,
            f.request(&f.model,json!({})),&f.opening(&row)).await.unwrap();
        f.bind(&row,&created.conversation_id).await;
        let session=AgentSessionId::from(created.conversation_id);
        let body=json!({"content":"Original pending input","files":[],"inject_skills":[]});
        let accepted=f.owner.canonical.accept_session_input(&f.principal,&session,key,body.clone()).await.unwrap().ack.unwrap();
        f.owner.canonical.store().mark_session_input_blocked(&session,&accepted.event_id,reason).await.unwrap();
        let cursor=f.owner.canonical.store().current_cursor(&session).await.unwrap();
        assert_eq!(f.owner.session_turn_delivery_state(&f.principal.principal_id,session.as_ref(),key).await.unwrap(),
            PublicTurnDeliveryState::InputUnavailable {message_id:accepted.event_id.as_ref().into(),cause:expected.clone(),event_seq:cursor.seq});
        let observation=ChannelSessionPort::observe_input(f.owner.as_ref(),&f.principal.principal_id,session.as_ref(),accepted.event_id.as_ref()).await.unwrap();
        match (&expected,observation) {
            (UnavailableInputCause::CancelledByOwner,nomifun_channel::ChannelInputObservation::Discarded {reason})=>assert_eq!(reason,"CANCELLED_BY_OWNER"),
            (UnavailableInputCause::PreparationBlocked {reason:expected},nomifun_channel::ChannelInputObservation::Blocked {reason})=>assert_eq!(&reason,expected),
            _=>panic!("the exact unavailable input must finish observation for {expected:?}"),
        }
        assert!(tokio::time::timeout(Duration::from_millis(250),
            f.owner.robot_output_wake(&f.principal.principal_id,session.as_ref(),key)).await.unwrap().unwrap().is_none(),
            "no unclaimed terminal may wait for a nonexistent Runtime");
        let replay=f.owner.canonical.accept_session_input(&f.principal,&session,key,body).await.unwrap();
        assert!(replay.duplicate);assert_eq!(replay.ack.unwrap().event_id,accepted.event_id);
        assert_eq!(f.owner.canonical.store().current_cursor(&session).await.unwrap(),cursor);
        assert!(f.owner.canonical.store().session_input_claim(&session,&accepted.event_id).await.unwrap().is_none());
        let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
            .bind(session.as_ref()).fetch_one(&f.owner.pool).await.unwrap();
        assert_eq!(turns,0,"an input observation is never a fabricated Turn terminal");
        if matches!(expected,UnavailableInputCause::PreparationBlocked {..}) {
            // A Remote cursor must observe the change after its original
            // accepted event, while keeping the same canonical message ID.
            let repository=&f.api.remote_repository;
            let binding=f.owner.canonical.get(&f.principal,&session).await.unwrap().session.agent_binding;
            let binding_id=Uuid::now_v7().to_string();
            let digest=nomifun_agent_contracts::digest_payload(&binding).unwrap().as_ref().to_owned();
            repository.create_binding(nomifun_db::CreateRemoteBindingParams {remote_binding_id:binding_id.clone(),
                owner_user_id:f.principal.principal_id.clone(),name:"Unavailable input projection".into(),
                agent_binding_json:serde_json::to_string(&binding).unwrap(),nomi_snapshot_json:"{}".into(),provenance_json:"{}".into(),
                agent_binding_digest:digest.clone(),binding_version:binding.binding_version as i64}).await.unwrap();
            repository.get_or_create_session(nomifun_db::GetOrCreateRemoteSessionParams {
                owner_user_id:f.principal.principal_id.clone(),remote_binding_id:binding_id,
                expected_binding_version:binding.binding_version as i64,expected_agent_binding_digest:digest,
                open_idempotency_key:format!("open-{key}"),agent_session_id:session.as_ref().into(),initial_input_digest:None,
            }).await.unwrap();
            repository.set_session_state(&f.principal.principal_id,session.as_ref(),"ready").await.unwrap();
            append_remote_event_once(repository,&f.principal.principal_id,session.as_ref(),"turn/accepted",Some(key),
                json!({"message_id":accepted.event_id,"outcome":"accepted"})).await.unwrap();
            let after=repository.read_events(&f.principal.principal_id,session.as_ref(),0,100).await.unwrap().next_cursor;
            schedule_remote_turn_finalizer(&f.api,&f.principal.principal_id,&session,key).unwrap();
            let events=tokio::time::timeout(Duration::from_secs(5),async {
                loop {
                    let page=repository.read_events(&f.principal.principal_id,session.as_ref(),after,100).await.unwrap();
                    if !page.events.is_empty() {break page;}
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }).await.unwrap();
            assert_eq!(events.events.len(),1);
            assert_eq!(events.events[0].event_type,"session/state-changed");
            let payload:Value=serde_json::from_str(&events.events[0].payload_json).unwrap();
            assert_eq!(payload["message_id"],accepted.event_id.as_ref());
            assert_eq!(payload["reason"],"SKILL_UNAVAILABLE");assert_eq!(payload["turn_admitted"],false);
            assert_eq!(payload["source_operation_key_digest"],remote_operation_key_digest(key).unwrap());
            assert_eq!(payload["input_event_seq"],cursor.seq);
            assert_eq!(payload["operation_key_digest"],remote_operation_key_digest(&remote_input_change_key(key,accepted.event_id.as_ref(),cursor.seq).unwrap()).unwrap());
            assert_eq!(repository.get_session(&f.principal.principal_id,session.as_ref()).await.unwrap().unwrap().state,"ready");
            tokio::time::timeout(Duration::from_secs(5),async {
                while !f.api.remote_runtime.is_empty() {tokio::time::sleep(Duration::from_millis(5)).await;}
            }).await.unwrap();
            schedule_remote_turn_finalizer(&f.api,&f.principal.principal_id,&session,key).unwrap();
            tokio::time::timeout(Duration::from_secs(5),async {
                while !f.api.remote_runtime.is_empty() {tokio::time::sleep(Duration::from_millis(5)).await;}
            }).await.unwrap();
            assert_eq!(repository.read_events(&f.principal.principal_id,session.as_ref(),after,100).await.unwrap().events,events.events,
                "re-observing the original input must not add another Remote projection or retry its work");
            assert_eq!(f.owner.canonical.store().current_cursor(&session).await.unwrap(),cursor);
            f.owner.canonical.store().mark_session_input_blocked(&session,&accepted.event_id,SessionInputBlockedReason::CancelledByOwner).await.unwrap();
            let cancelled=f.owner.canonical.store().current_cursor(&session).await.unwrap();
            schedule_remote_turn_finalizer(&f.api,&f.principal.principal_id,&session,key).unwrap();
            let changed=tokio::time::timeout(Duration::from_secs(5),async {
                loop {
                    let page=repository.read_events(&f.principal.principal_id,session.as_ref(),events.next_cursor,100).await.unwrap();
                    if !page.events.is_empty() {break page;}
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }).await.unwrap();
            assert_eq!(changed.events.len(),1);
            assert_eq!(changed.events[0].event_type,"session/state-changed");
            let changed_payload:Value=serde_json::from_str(&changed.events[0].payload_json).unwrap();
            assert_eq!(changed_payload["message_id"],accepted.event_id.as_ref());
            assert_eq!(changed_payload["reason"],"CANCELLED_BY_OWNER");
            assert_eq!(changed_payload["source_operation_key_digest"],payload["source_operation_key_digest"]);
            assert_eq!(changed_payload["input_event_seq"],cancelled.seq);
            assert_ne!(changed_payload["operation_key_digest"],payload["operation_key_digest"]);
            tokio::time::timeout(Duration::from_secs(5),async {
                while !f.api.remote_runtime.is_empty() {tokio::time::sleep(Duration::from_millis(5)).await;}
            }).await.unwrap();
            schedule_remote_turn_finalizer(&f.api,&f.principal.principal_id,&session,key).unwrap();
            tokio::time::timeout(Duration::from_secs(5),async {
                while !f.api.remote_runtime.is_empty() {tokio::time::sleep(Duration::from_millis(5)).await;}
            }).await.unwrap();
            assert_eq!(repository.read_events(&f.principal.principal_id,session.as_ref(),events.next_cursor,100).await.unwrap().events,changed.events);
            assert_eq!(f.owner.canonical.store().current_cursor(&session).await.unwrap(),cancelled);
        }
    }
    f.finish().await;
}
