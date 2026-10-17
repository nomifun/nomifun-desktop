//! One canonical context boundary for user commands and product-owned resets.
use super::*;
use nomifun_agent_session::{ClearSessionContext, ContextClearPreparation};

impl NomiCoreSessionOwner {
    pub(crate) async fn clear_session_context(
        &self, owner_id: &str, session_id: &AgentSessionId, idempotency_key: &str, reason: &str,
    ) -> Result<(), AppError> {
        let _operation_fence = self.session_operation_lock(session_id.as_ref()).write_owned().await;
        if self.is_companion_associated_task(owner_id, session_id).await? {
            return Err(AppError::Conflict("Associated task Sessions keep their exact execution context".into()));
        }
        self.clear_session_context_locked(owner_id, session_id, idempotency_key, reason, None).await.map(|_| ())
    }

    /// The caller holds the existing Session operation write lock, including
    /// when a Cron run must clear and admit its first input without a gap.
    pub(super) async fn clear_session_context_locked(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        reason: &str,
        expected: Option<&nomifun_agent_session::ContextClearBoundary>,
    ) -> Result<nomifun_agent_session::SessionEventAppendResult, AppError> {
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() };
        let command = ClearSessionContext {
            idempotency_key: idempotency_key.to_owned().into(), reason: reason.to_owned(),
        };
        let preparation = match expected {
            Some(expected) => self.canonical.store().validate_context_clear_boundary(&principal, session_id, &command, expected).await,
            None => self.canonical.store().preflight_context_clear(&principal, session_id, &command).await,
        }.map_err(context_clear_error)?;
        let boundary = match preparation {
            ContextClearPreparation::AlreadyCleared(receipt) => return Ok(receipt),
            ContextClearPreparation::Ready(boundary) => boundary,
        };
        if let Some(engines) = self.native_engines.get().and_then(std::sync::Weak::upgrade) {
            engines.cancel_and_join_knowledge_writebacks(session_id).await?;
        }
        // Teardown includes cold builds and quarantined owners. An absent live
        // Runtime handle alone is never proof that work has stopped.
        self.settle_session_binding_runtime(owner_id, session_id).await?;
        let receipt = self.canonical.store().clear_context(&principal, session_id, &command, &boundary)
            .await.map_err(context_clear_error)?;
        self.user_events.send_to_user(owner_id, WebSocketMessage::new("conversation.listChanged", json!({
            "conversation_id": session_id, "action": "updated",
        })));
        Ok(receipt)
    }
}

fn context_clear_error(error: nomifun_agent_session::SessionStoreError) -> AppError {
    use nomifun_agent_session::SessionStoreError;
    match error {
        SessionStoreError::CheckpointNotQuiescent | SessionStoreError::ExecutionFenced
        | SessionStoreError::ExecutionLeaseActive | SessionStoreError::RecoveryRequiresReconciliation =>
            AppError::Conflict(error.to_string()),
        other => agent_session_store_error(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_cron::{CronSessionPort, CronRuntimePreparationRequest, CronTurnMessage,
        CronTurnRequest, CronTurnRuntimeOverlay, CronTurnRuntimePreparation};
    use tower::ServiceExt;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};

    const TRUST: &str = "canonical-context-clear";

    struct Fixture {
        _root: tempfile::TempDir,
        _services: crate::services::AppServices,
        router: Router,
        owner: Arc<NomiCoreSessionOwner>,
        principal: PrincipalRef,
        session: AgentSessionId,
        upstream: MockServer,
    }

    impl Fixture {
        async fn new() -> Self {
            let upstream = MockServer::start().await;
            Mock::given(method("POST")).and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                    .set_body_string(concat!(
                        "data: {\"id\":\"clear-fixture\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Fixture reply\"},\"finish_reason\":null}]}\n\n",
                        "data: {\"id\":\"clear-fixture\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: [DONE]\n\n")))
                .mount(&upstream).await;
            let root = tempfile::tempdir().unwrap();
            let fixture_root = std::fs::canonicalize(root.path()).unwrap();
            let config = crate::AppConfig {
                data_dir: fixture_root.join("data"), work_dir: fixture_root.join("work"),
                auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
                local_trust_secret: Some(TRUST.into()), ..Default::default()
            };
            std::fs::create_dir_all(&config.data_dir).unwrap();
            let db = nomifun_db::init_database(&config.database_path()).await.unwrap();
            let services = crate::services::AppServices::from_config(db, &config).await.unwrap();
            let (states, _components) = super::super::super::state::try_build_module_states(&services).await.unwrap();
            let owner = states.nomi_core_agent_api.session_owner.clone();
            let router = super::super::super::create_router_with_states(&services, states);
            let principal = PrincipalRef { principal_kind: "user".into(), principal_id: services.authoritative_user_id.as_ref().into() };
            let provider = call(&router, "/api/providers", json!({
                "platform":"custom", "name":"isolated clear fixture", "base_url":format!("{}/v1",upstream.uri()),
                "auth_scheme":"bearer", "credentials":{"api_keys":["fixture-only"]}, "enabled":true,
                "initial_model":{"model":"clear-fixture", "enabled":true, "capabilities":[{
                    "task":"chat", "traits":[], "protocol":"openai.chat_text", "connection_role":"default"
                }]}
            })).await;
            let model = json!({"provider_id":provider["provider_id"],"model":"clear-fixture"});
            let preset = call(&router, "/api/agent-presets/from-template/chat.minimal",
                json!({"reuse_existing":false,"display_name":"Clear fixture","model":model})).await;
            let session = call(&router, "/api/agent-sessions",
                json!({"preset_id":preset["preset"]["preset_id"],"model":model})).await;
            Self { _root:root, _services:services, router, owner, principal,
                session:session["agent_session_id"].as_str().unwrap().into(), upstream }
        }

        async fn seed_prior_turn(&self, key: &str) {
            let request: SendMessageRequest = serde_json::from_value(json!({"content":"PRIVATE_PRE_CLEAR_CONTEXT"})).unwrap();
            let operation = NomiCoreSessionOwner::turn_operation_id(&self.principal.principal_id, self.session.as_ref(), key);
            let input = self.owner.canonical_turn_input_with_admission(&self.principal.principal_id, &self.session, &operation, &request).await.unwrap();
            self.owner.canonical.start_turn(&self.principal, &self.session, key, input).await.unwrap();
            self.owner.canonical.cancel_exact_turn(&self.principal, &self.session, &format!("cancel:{key}"), &operation).await.unwrap();
        }

        async fn clear_count(&self) -> i64 {
            sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
                .bind(self.session.as_ref()).fetch_one(&self.owner.pool).await.unwrap()
        }

        async fn warm(&self) {
            call(&self.router, &format!("/api/agent-sessions/{}/warmup", self.session.as_ref()), json!({})).await;
            assert!(self.owner.runtime_sessions.get_runtime(self.session.as_ref()).is_some());
        }

        async fn finish(self) {
            self.owner.runtime_sessions.terminate_and_wait_result(self.session.as_ref(), None).await.unwrap();
        }
    }

    async fn response(router: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
        let response = router.clone().oneshot(axum::http::Request::builder().method("POST").uri(uri)
            .header("x-nomi-local-trust", TRUST).header("content-type","application/json")
            .body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4*1024*1024).await.unwrap();
        let value=serde_json::from_slice(&bytes).unwrap_or_else(|_|Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, value)
    }

    async fn call(router: &Router, uri: &str, body: Value) -> Value {
        let (status, value) = response(router, uri, body).await;
        assert!(status.is_success(), "{uri}: {status} {value}");
        value["data"].clone()
    }

    #[tokio::test]
    async fn user_and_archive_clear_share_canonical_history_and_replay_never_tears_down_successor() {
        let f = Fixture::new().await;
        f.seed_prior_turn("prior").await;
        let before = f.owner.canonical.get(&f.principal, &f.session).await.unwrap().session;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_messages WHERE session_id=?")
            .bind(f.session.as_ref()).fetch_one(&f.owner.pool).await.unwrap();
        f.warm().await;
        let uri = format!("/api/agent-sessions/{}/clear-context",f.session.as_ref());
        call(&f.router, &uri, json!({"idempotency_key":"user-action"})).await;
        assert_eq!(f.clear_count().await,1);
        assert!(!f.owner.runtime_sessions.has_owned_runtime(f.session.as_ref()));
        let after=f.owner.canonical.get(&f.principal, &f.session).await.unwrap().session;
        assert_eq!(after.agent_session_id,before.agent_session_id);
        assert_eq!(after.owner_ref,before.owner_ref);
        assert_eq!(after.agent_binding,before.agent_binding);
        assert_eq!(after.metadata,before.metadata);
        let retained: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_messages WHERE session_id=?")
            .bind(f.session.as_ref()).fetch_one(&f.owner.pool).await.unwrap();
        assert_eq!(retained,count);
        f.warm().await;
        call(&f.router, &uri, json!({"idempotency_key":"user-action"})).await;
        assert!(f.owner.runtime_sessions.has_owned_runtime(f.session.as_ref()));
        assert_eq!(f.clear_count().await,1);
        let archive=nomifun_companion::CompanionArchiveSessionPort::window_messages(f.owner.as_ref(),
            &f.principal.principal_id,f.session.as_ref(),0,400,"archive-window").await.unwrap();
        nomifun_companion::CompanionArchiveSessionPort::reset_context(f.owner.as_ref(),
            &f.principal.principal_id, f.session.as_ref(), "archive-window", &archive.context_boundary).await.unwrap();
        assert_eq!(f.clear_count().await,2);
        assert!(!f.owner.runtime_sessions.has_owned_runtime(f.session.as_ref()));
        assert!(f.upstream.received_requests().await.unwrap().is_empty(),"clear and warmup never infer");
        let (status,_) = response(&f.router,&uri,json!({"idempotency_key":"","reason":"forged"})).await;
        assert!(status.is_client_error());
        f.finish().await;
    }

    #[tokio::test]
    async fn cron_run_clears_once_before_input_and_redelivery_preserves_later_runtime() {
        let f = Fixture::new().await;
        f.seed_prior_turn("prior").await;
        let job = Uuid::now_v7().to_string();
        let run = Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO cron_jobs(cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,agent_type,created_by,created_at,updated_at,conversation_id) VALUES(?,?,'fixture','every','60000','scheduled','nomi','user',1,1,?)")
            .bind(&job).bind(&f.principal.principal_id).bind(f.session.as_ref()).execute(&f.owner.pool).await.unwrap();
        sqlx::query("INSERT INTO cron_run_reservations(cron_job_run_id,cron_job_id,trigger_kind,operation_key,request_fingerprint,conversation_id,created_at_ms,updated_at_ms) VALUES(?,?,'run_now',?,'fixture',?,1,1)")
            .bind(&run).bind(&job).bind(&run).bind(f.session.as_ref()).execute(&f.owner.pool).await.unwrap();
        let request = CronRuntimePreparationRequest {
            owner_id:f.principal.principal_id.clone(),agent_session_id:f.session.clone(),idempotency_key:"cron-turn".into(),
            turn:CronTurnRequest { message:CronTurnMessage { content:"Scheduled fixture request".into(),files:vec![],inject_skills:vec![],hidden:false,origin:Some("cron".into()),channel_platform:None },
                runtime:CronTurnRuntimePreparation { overlay:CronTurnRuntimeOverlay {cron_job_id:job,cron_job_run_id:run},clear_context:true } },
        };
        let mut reserved_origin=request.clone();
        reserved_origin.turn.message.origin=Some("idmm".into());
        assert!(f.owner.prepare_runtime_and_send(reserved_origin).await.is_err());
        assert_eq!(f.clear_count().await,0,"origin validation must precede a context reset");
        f.warm().await;
        let delivered=f.owner.prepare_runtime_and_send(request.clone()).await.unwrap();
        assert!(!delivered.delivery.replayed);
        tokio::time::timeout(Duration::from_secs(10),async { loop {
            if matches!(f.owner.session_turn_delivery_state(&f.principal.principal_id,f.session.as_ref(),"cron-turn").await.unwrap(),PublicTurnDeliveryState::Completed(_)) { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }}).await.unwrap();
        assert_eq!(f.clear_count().await,1);
        let requests=f.upstream.received_requests().await.unwrap();
        assert!(!requests.is_empty());
        for request in requests { assert!(!String::from_utf8_lossy(&request.body).contains("PRIVATE_PRE_CLEAR_CONTEXT")); }
        f.warm().await;
        let replay=f.owner.prepare_runtime_and_send(request.clone()).await.unwrap();
        assert!(replay.delivery.replayed);
        assert!(f.owner.runtime_sessions.has_owned_runtime(f.session.as_ref()));
        assert_eq!(f.clear_count().await,1);
        // Simulate a crash after clear but before admission, followed by a
        // different user request. Retrying that unaccepted run must preserve it.
        let mut interrupted=request;
        let run=Uuid::now_v7().to_string();
        interrupted.idempotency_key="interrupted-run".into();
        interrupted.turn.runtime.overlay.cron_job_run_id=run.clone();
        sqlx::query("INSERT INTO cron_run_reservations(cron_job_run_id,cron_job_id,trigger_kind,operation_key,request_fingerprint,conversation_id,created_at_ms,updated_at_ms) VALUES(?,?,'run_now',?,'fixture',?,1,1)")
            .bind(&run).bind(&interrupted.turn.runtime.overlay.cron_job_id).bind(&run)
            .bind(f.session.as_ref()).execute(&f.owner.pool).await.unwrap();
        f.owner.clear_session_context(&f.principal.principal_id,&f.session,&format!("cron-context-clear:{run}"),"cron_run").await.unwrap();
        f.seed_prior_turn("intervening-input").await;
        f.warm().await;
        assert!(f.owner.prepare_runtime_and_send(interrupted).await.is_err());
        assert_eq!(f.clear_count().await,2);
        assert!(f.owner.runtime_sessions.has_owned_runtime(f.session.as_ref()));
        f.finish().await;
    }

    struct ArchivePort {
        owner: Arc<NomiCoreSessionOwner>,
        owner_id: String,
    }

    #[async_trait]
    impl nomifun_companion::archiver::ArchiveConversationPort for ArchivePort {
        async fn window_messages(&self, session:&str, since:i64, key:&str)
            -> Result<nomifun_companion::archiver::ArchiveWindow,AppError> {
            let mut window=nomifun_companion::CompanionArchiveSessionPort::window_messages(
                self.owner.as_ref(),&self.owner_id,session,since,400,key).await?;
            // Age only the scheduler's display observations. The immutable
            // proof still comes from the real canonical owner and current input.
            for message in &mut window.messages { message.created_at-=120_000; }
            Ok(window)
        }
        async fn reset_context(&self, session:&str,key:&str,expected:&nomifun_agent_session::ContextClearBoundary)
            -> Result<(),AppError> {
            nomifun_companion::CompanionArchiveSessionPort::reset_context(self.owner.as_ref(),
                &self.owner_id,session,key,expected).await
        }
    }

    struct DelayedDigest {
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl nomifun_companion::learner::CompanionCompleter for DelayedDigest {
        async fn complete(&self,_provider:&str,_model:&str,_system:&str,input:&str,_tokens:Option<u32>)->Result<String,AppError> {
            assert!(input.contains("PRIVATE_PRE_CLEAR_CONTEXT"));
            self.started.notify_one();
            self.release.notified().await;
            Ok(r#"{"summary":"The original archive window was summarized.","topics":["fixture"],"mood":"content"}"#.into())
        }
    }

    #[tokio::test]
    async fn delayed_archive_digest_preserves_a_successor_turn_and_runtime() {
        use nomifun_companion::{CompanionRegistry,CompanionStore,SharedCompanionConfig};
        let f=Fixture::new().await;
        f.seed_prior_turn("original-window").await;
        let registry=Arc::new(CompanionRegistry::scan(f._root.path().join("companions"),f._root.path().join("shared")).unwrap());
        let companion=registry.create("Archive fixture","ink").await.unwrap();
        let store=CompanionStore::open_memory().await.unwrap();
        store.insert_companion_thread(f.session.as_ref(),&companion.companion_id,"Archive fixture").await.unwrap();
        let started=Arc::new(tokio::sync::Notify::new());
        let release=Arc::new(tokio::sync::Notify::new());
        let archiver=Arc::new(nomifun_companion::archiver::Archiver {
            store:store.clone(),config:Arc::new(tokio::sync::RwLock::new(SharedCompanionConfig::default())),registry,
            completer:Arc::new(DelayedDigest {started:started.clone(),release:release.clone()}),
            port:Arc::new(ArchivePort {owner:f.owner.clone(),owner_id:f.principal.principal_id.clone()}),
            run_lock:Arc::new(tokio::sync::Mutex::new(())),
        });
        let companion_id=companion.companion_id.clone();
        let task=tokio::spawn(async move {
            archiver.sweep_companion(&companion_id,&nomifun_companion::archiver::ArchiveThresholds {idle_minutes:1,min_chars:1},
                &nomifun_common::ProviderWithModel {provider_id:nomifun_common::generate_id(),model:"archive-fixture".into(),use_model:None}).await
        });
        tokio::time::timeout(Duration::from_secs(5),started.notified()).await.unwrap();
        f.seed_prior_turn("successor-during-digest").await;
        f.warm().await;
        let before=f.owner.canonical.store().head(&f.session).await.unwrap().last_seq;
        release.notify_one();
        assert_eq!(task.await.unwrap().unwrap(),nomifun_companion::archiver::SweepAction::Archive);
        assert_eq!(store.list_digests(&companion.companion_id,10).await.unwrap().len(),1,"keep the successful digest");
        assert_eq!(f.clear_count().await,0,"the digest cannot clear a newer accepted input");
        assert_eq!(f.owner.canonical.store().head(&f.session).await.unwrap().last_seq,before);
        assert!(f.owner.runtime_sessions.has_owned_runtime(f.session.as_ref()),"stale archive proof must fail before teardown");
        f.finish().await;
    }
}
