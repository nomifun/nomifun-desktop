use super::*;

// The acceptance path runs without a chat runtime. Provider execution itself is
// covered in nomifun-creation; this fixture intentionally has no provider driver.
struct CreationPresetResolver {
    revision: std::sync::atomic::AtomicUsize,
    calls: std::sync::atomic::AtomicUsize,
    runtime_extra: serde_json::Value,
}

#[async_trait::async_trait]
impl nomifun_conversation::ProductAgentSnapshotResolver for CreationPresetResolver {
    async fn resolve_preset(
        &self,
        _: &str,
        preset_id: &str,
        model: Option<&nomifun_common::ProviderWithModel>,
        _: Option<&nomifun_api_types::AgentBindingValueDto>,
    ) -> Result<nomifun_conversation::ProductAgentResolution, AppError> {
        assert!(
            model.is_none(),
            "professional generation needs no chat model"
        );
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut snapshot = make_preset_snapshot("unused");
        snapshot.preset_id = preset_id.to_owned();
        snapshot.preset_revision = self.revision.load(std::sync::atomic::Ordering::SeqCst) as i64;
        snapshot.resolved_model = None;
        snapshot.enabled_capabilities = vec!["creation.media".into()];
        snapshot.enabled_capability_actions = std::collections::BTreeMap::from([(
            "creation.media".into(),
            std::collections::BTreeSet::from([
                "creation.media/image".into(),
                "creation.media/video".into(),
                "creation.media/music".into(),
            ]),
        )]);
        Ok(nomifun_conversation::ProductAgentResolution {
            snapshot,
            runtime_extra: self.runtime_extra.clone(),
        })
    }
    async fn resolve(
        &self,
        _: &str,
        _: &nomifun_conversation::ProductAgentTarget,
        _: Option<&nomifun_common::ProviderWithModel>,
    ) -> Result<nomifun_conversation::ProductAgentResolution, AppError> {
        Err(AppError::BadRequest("not a product target".into()))
    }
}

async fn creation_fixture() -> (
    ConversationService,
    nomifun_db::Database,
    String,
    String,
    Arc<CreationPresetResolver>,
    Arc<nomifun_creation::CreationService>,
) {
    use nomifun_db::sqlx;
    let db = init_database_memory().await.unwrap();
    let provider = nomifun_common::ProviderId::new().into_string();
    sqlx::query("INSERT INTO providers (provider_id,platform,name,base_url,auth_scheme,credentials_encrypted,enabled,created_at,updated_at) VALUES (?,'openai','Test','https://example.invalid','bearer','',1,0,0)")
        .bind(&provider).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO provider_models (provider_id,model,enabled,sort_order,created_at,updated_at) VALUES (?,'media',1,0,0,0)")
        .bind(&provider).execute(db.pool()).await.unwrap();
    for (task, protocol) in [
        ("image_generation", "openai.images"),
        ("video_generation", "openai.videos"),
        ("music_generation", "minimax.music"),
    ] {
        sqlx::query("INSERT INTO provider_model_capabilities (provider_id,model,task,traits,protocol,connection_role,provider_params,created_at,updated_at) VALUES (?,'media',?,'[]',?,'default','{}',0,0)")
            .bind(&provider).bind(task).bind(protocol).execute(db.pool()).await.unwrap();
    }
    let id = nomifun_common::ConversationId::new().into_string();
    sqlx::query("INSERT INTO conversations (conversation_id,user_id,name,type,extra,created_at,updated_at) VALUES (?,?,'Creation','nomi','{}',0,0)")
        .bind(&id).bind(USER_ID).execute(db.pool()).await.unwrap();
    let svc = ConversationService::new(
        Arc::<str>::from(USER_ID),
        PathBuf::from("."),
        Arc::new(TestBroadcaster::new()),
        Arc::new(EmptySkillResolver),
        Arc::new(NoopAgentRuntimeRegistry),
        Arc::new(SqliteConversationRepository::new(db.pool().clone())),
        Arc::new(nomifun_db::SqliteAgentMetadataRepository::new(
            db.pool().clone(),
        )),
        Arc::new(nomifun_conversation::NoExecutionConversationBoundary),
    );
    let resolver = Arc::new(CreationPresetResolver {
        revision: 1.into(),
        calls: 0.into(),
        runtime_extra: json!({}),
    });
    svc.with_product_agent_snapshot_resolver(resolver.clone());
    let engine = nomifun_creation::CreationService::new(Arc::new(
        nomifun_db::SqliteCreationTaskRepository::new(db.pool().clone()),
    ));
    svc.with_creation_service(engine.clone());
    (svc, db, id, provider, resolver, engine)
}

#[tokio::test]
async fn conversation_creation_music_without_chat_model_keeps_music_parameters_and_turn_ownership() {
    let (svc, _db, id, provider, _, _) = creation_fixture().await;
    let key = nomifun_common::CreationTaskId::new().into_string();
    let request = json!({"preset_id":nomifun_common::generate_id(),"provider_id":provider,"model":"media","capability":"music",
        "params":{"prompt":"Gentle piano","instrumental":true}});
    let accepted = svc.submit_conversation_creation(USER_ID, &id, &key, serde_json::from_value(request).unwrap()).await.unwrap();
    assert_eq!(accepted.message_id, key);
    assert_eq!(accepted.tasks.len(), 1);
    let task = &accepted.tasks[0];
    assert_eq!(task.capability, "music");
    assert_eq!(task.params["instrumental"], true);
    assert_eq!(task.params["prompt"], "Gentle piano");
    assert!(task.params.get("voice").is_none());
    assert!(task.params.get("seconds").is_none());
    assert_eq!(serde_json::to_value(&task.owner).unwrap(), json!({"kind":"conversation_turn","conversation_id":id,"message_id":key}));
    assert_eq!(svc.list_conversation_creations(USER_ID, &id).await.unwrap().items.len(), 1);
}

#[tokio::test]
async fn ordinary_preset_refresh_preserves_exact_runtime_binding_and_rejects_engine_changes() {
    use nomifun_db::sqlx;
    for variant in ["omitted", "same_reordered", "different_profile"] {
        let (svc, db, _, _, _, _) = creation_fixture().await;
        let id = nomifun_common::ConversationId::new().into_string();
        let preset_id = nomifun_common::generate_id();
        let binding = format!(r#"{{"family_id":"nomifun.nomi","build_id":"test-build","build_digest":"{}","host_contract_version":1,"profile":"default"}}"#, "a".repeat(64));
        let mut old_snapshot = make_preset_snapshot("unused");
        old_snapshot.preset_id = preset_id.clone();
        old_snapshot.resolved_model = None;
        let old_snapshot = serde_json::to_string(&old_snapshot).unwrap();
        let extra = format!(r#"{{"runtime_engine_binding":{binding},"workspace":"kept"}}"#);
        sqlx::query("INSERT INTO conversations (conversation_id,user_id,name,type,preset_id,preset_revision,agent_snapshot,extra,created_at,updated_at) VALUES (?,?,'Runtime identity','nomi',?,1,?,?,0,0)")
            .bind(&id).bind(USER_ID).bind(&preset_id).bind(&old_snapshot).bind(&extra).execute(db.pool()).await.unwrap();
        let mut incoming = json!({"system_prompt":"Refreshed Agent instructions"});
        if variant != "omitted" {
            // Equal fields deliberately arrive in the opposite JSON order.
            incoming["runtime_engine_binding"] = serde_json::from_str(&format!(r#"{{"profile":"{}","host_contract_version":1,"build_digest":"{}","build_id":"test-build","family_id":"nomifun.nomi"}}"#,
                if variant == "different_profile" { "other" } else { "default" }, "a".repeat(64))).unwrap();
        }
        svc.with_product_agent_snapshot_resolver(Arc::new(CreationPresetResolver {
            revision: 2.into(), calls: 0.into(), runtime_extra: incoming,
        }));
        let runtime: Arc<dyn AgentRuntimeRegistry> = Arc::new(NoopAgentRuntimeRegistry);
        let result = svc.send_message_with_idempotency_key(USER_ID, &id, &format!("refresh-{variant}"), serde_json::from_value(json!({
            "content":"Generate another image", "preset_id":preset_id,
        })).unwrap(), &runtime).await;
        let (stored_binding, revision, snapshot, stored_extra): (String, i64, String, String) = sqlx::query_as(
            "SELECT json_extract(extra,'$.runtime_engine_binding'),preset_revision,agent_snapshot,extra FROM conversations WHERE conversation_id=?")
            .bind(&id).fetch_one(db.pool()).await.unwrap();
        assert_eq!(stored_binding, binding, "{variant}: immutable binding JSON must remain byte-identical");
        if variant == "different_profile" {
            assert!(matches!(result, Err(AppError::Conflict(ref message)) if message.contains("different runtime engine")), "{result:?}");
            assert_eq!(revision, 1); assert_eq!(snapshot, old_snapshot); assert_eq!(stored_extra, extra);
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE conversation_id=?").bind(&id).fetch_one(db.pool()).await.unwrap();
            assert_eq!(count, 0, "cross-engine selection is rejected before turn admission");
        } else {
            assert_eq!(revision, 2, "{variant}: refresh must pass the real SQLite immutable trigger: {result:?}");
            let stored: serde_json::Value = serde_json::from_str(&stored_extra).unwrap();
            assert_eq!(stored["workspace"], "kept");
            assert_eq!(stored["system_prompt"], "Refreshed Agent instructions");
        }
    }
}

#[tokio::test]
async fn conversation_creation_without_chat_model_freezes_batch_and_replays_after_preset_changes() {
    use nomifun_db::sqlx;
    let (svc, db, id, provider, resolver, _) = creation_fixture().await;
    let key = nomifun_common::CreationTaskId::new().into_string();
    let request = json!({"preset_id":nomifun_common::generate_id(),"provider_id":provider,"model":"media","capability":"t2v",
        "params":{"prompt":"Sunlit coast","seconds":5,"resolution":"720p","count":2}});
    let accepted = svc
        .submit_conversation_creation(
            USER_ID,
            &id,
            &key,
            serde_json::from_value(request.clone()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.message_id, key);
    assert_eq!(accepted.tasks.len(), 2);
    let first_ids: Vec<_> = accepted
        .tasks
        .iter()
        .map(|task| task.creation_task_id.clone())
        .collect();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE conversation_id=?")
        .bind(&id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1, "a batch is one user turn");
    let stored: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT conversation_id,message_id,params FROM creation_tasks ORDER BY creation_task_id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert_eq!(stored.len(), 2);
    for (conversation, message, params) in stored {
        assert_eq!(conversation, id);
        assert_eq!(message, key);
        let params: serde_json::Value = serde_json::from_str(&params).unwrap();
        assert_eq!(params["count"], 1);
        assert_eq!(params["seconds"], 5);
        assert_eq!(params["resolution"], "720p");
        assert_eq!(params["_nomifun_creation_agent"]["preset_revision"], 1);
    }
    resolver
        .revision
        .store(2, std::sync::atomic::Ordering::SeqCst);
    let replay = svc
        .submit_conversation_creation(
            USER_ID,
            &id,
            &key,
            serde_json::from_value(request.clone()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        replay
            .tasks
            .iter()
            .map(|task| task.creation_task_id.clone())
            .collect::<Vec<_>>(),
        first_ids
    );
    assert_eq!(resolver.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let mut changed = request;
    changed["params"]["seconds"] = json!(10);
    assert!(matches!(
        svc.submit_conversation_creation(
            USER_ID,
            &id,
            &key,
            serde_json::from_value(changed).unwrap()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let stranger = nomifun_common::UserId::new().into_string();
    assert!(matches!(
        svc.list_conversation_creations(&stranger, &id).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn conversation_creation_tool_attaches_to_original_user_message_without_rewriting_it() {
    use nomifun_db::sqlx;
    let (_svc, db, id, provider, _, engine) = creation_fixture().await;
    let message = nomifun_common::MessageId::new().into_string();
    let original =
        json!({"content":"帮我把上面的想法画出来","agent_snapshot":{"preset_name":"通用助理"}})
            .to_string();
    sqlx::query("INSERT INTO messages (message_id,conversation_id,msg_id,type,content,position,status,created_at) VALUES (?,?,?,'text',?,'right','finish',1)")
        .bind(&message).bind(&id).bind(&message).bind(&original).execute(db.pool()).await.unwrap();
    let task_id = nomifun_common::CreationTaskId::new().into_string();
    let task = engine
        .create_creative_task(
            nomifun_creation::CreativeTaskOwner::ConversationTurn {
                conversation_id: id.clone(),
                message_id: message.clone(),
            },
            task_id,
            nomifun_creation::NewCreationTask {
                provider_id: provider,
                model: "media".into(),
                capability: "t2i".into(),
                params: json!({"prompt":"A refined visual interpretation"}),
                inputs: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(task.message_id.as_deref(), Some(message.as_str()));
    let stored: String = sqlx::query_scalar("SELECT content FROM messages WHERE message_id=?")
        .bind(&message)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(stored, original);
}

#[tokio::test]
async fn conversation_creation_agent_selection_cannot_replace_a_running_turn() {
    use nomifun_db::{IConversationRepository, sqlx};
    let (svc, db, id, _, resolver, _) = creation_fixture().await;
    let repository = SqliteConversationRepository::new(db.pool().clone());
    let admission = repository
        .claim_turn_delivery_receipt_and_admit(
            USER_ID,
            &id,
            "existing-running-turn",
            "{}",
            0,
            nomifun_common::now_ms(),
        )
        .await
        .unwrap();
    assert!(admission.claimed_new);
    let request = serde_json::from_value(json!({
        "content": "Continue with this Agent next",
        "preset_id": nomifun_common::generate_id(),
    }))
    .unwrap();
    let runtime: Arc<dyn AgentRuntimeRegistry> = Arc::new(NoopAgentRuntimeRegistry);
    let result = svc
        .send_message_with_idempotency_key(
            USER_ID,
            &id,
            "next-agent-during-running",
            request,
            &runtime,
        )
        .await;
    assert!(
        matches!(result, Err(AppError::Conflict(_))),
        "running turn must retain its authority: {result:?}"
    );
    assert_eq!(resolver.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT status,agent_snapshot FROM conversations WHERE conversation_id=?")
            .bind(&id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(row, ("running".to_owned(), None));
    let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE conversation_id=?")
        .bind(&id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(messages, 0);
}

#[tokio::test]
async fn conversation_creation_delete_cancels_queued_work_before_removing_messages() {
    use nomifun_db::{ICreationTaskRepository, sqlx};
    let (svc, db, id, provider, _, _) = creation_fixture().await;
    let task = nomifun_common::CreationTaskId::new().into_string();
    let repo = nomifun_db::SqliteCreationTaskRepository::new(db.pool().clone());
    repo.get_or_create_creative_task(nomifun_db::CreateCreativeTaskParams {
        creation_task_id: &task,
        owner: nomifun_db::CreativeTaskOwnerRef::ConversationTurn {
            conversation_id: &id,
            message_id: &task,
        },
        provider_id: &provider,
        model: "media",
        capability: "t2i",
        params: r#"{"prompt":"Pending work"}"#,
        input_bindings: "[]",
        request_fingerprint: "{}",
        status: "queued",
        submitted_at: 1,
    })
    .await
    .unwrap();
    svc.delete(USER_ID, &id).await.unwrap();
    assert_eq!(
        repo.get_task(&task).await.unwrap().unwrap().status,
        "canceled"
    );
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE conversation_id=?")
            .bind(&id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 0);
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE conversation_id=?")
            .bind(&id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}
