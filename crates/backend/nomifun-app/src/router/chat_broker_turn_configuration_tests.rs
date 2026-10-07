//! Focused native-host configuration boundaries; no provider/model calls.
use super::*;
use nomifun_agent_contracts::{ChatRouteCandidate, ChatRouteFeature, ChatRouteIdentity, ChatRouteProtocol, ChatRouteRecordSchema, ChatRouteTask};
use nomifun_agent_contracts::AgentPresetRevisionPayload;
use nomifun_chat_model_broker::{ChatRouteResolver, ProductionRepositorySet, ProductionRouteResolver};

const PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000098";

async fn persist_record(pool: &SqlitePool, revision: &str, number: i64, record: &CanonicalChatRouteRecord) {
    let payload = AgentPresetRevisionPayload {
        context_order: Vec::new(), middleware_order: Vec::new(), schema_version: "1.0.0".into(),
        model_route_refs: BTreeMap::from([(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT.into(), record.primary.model_route_id.clone())]),
        chat_route_records: BTreeMap::from([(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT.into(), record.clone())]),
        enabled_capabilities: Vec::new(), skill_bindings: Vec::new(), system_role_provider_overrides: BTreeMap::new(),
        persona: "Test".into(), instructions: "Test".into(), starter_prompts: Vec::new(), runtime_policy: Default::default(),
    };
    sqlx::query("INSERT INTO agent_preset_revisions (revision_id,preset_id,revision_no,schema_version,payload_json,revision_digest,created_by,created_at,reason) VALUES (?,'preset',?,'1.0.0',?,?,'owner',0,'')")
        .bind(revision).bind(number).bind(serde_json::to_string(&payload).unwrap()).bind("a".repeat(64))
        .execute(pool).await.unwrap();
}

async fn attempt_wire(pool: &SqlitePool, view: &TurnModelConfiguration, identity: &ChatRouteSelection) -> Result<Value, ProductionRepositoryError> {
    let mut repository = ProductionModelRepository::new(pool.clone());
    repository.turn_configuration = Some(view.clone());
    let repository = Arc::new(repository);
    let mut provider = ProductionProviderRepository::new(pool.clone());
    provider.turn_configuration = Some(view.clone());
    let credentials = ConnectionCredentialLeaseRegistry::default();
    let connection = Arc::new(ProductionConnectionRepository::new(pool.clone(), credentials.clone()));
    let resolver = ProductionRouteResolver::new(ProductionRepositorySet::new(
        Arc::new(provider), connection, repository.clone(),
    ));
    // Exercise the production provider exact-digest AND fresh connection
    // identity path. Resolution registers ciphertext only; it never leases,
    // decrypts credentials, creates an HTTP executor or calls the network.
    let route = resolver.resolve(identity).await.map_err(|_| ProductionRepositoryError::InvalidData)?.primary;
    assert!(credentials.leases.read().unwrap().is_empty());
    assert_eq!(credentials.credentials.read().unwrap().len(), 1);
    let request = ProviderWireRequest {
        protocol: route.protocol, provider_id: route.provider_id.clone(), model: route.model.clone(), route_identity: identity.clone(),
        connection_config_ref: route.connection_config_ref.clone(), config_revision_digest: route.config_revision_digest.clone(),
        credential_ref: route.credential_ref.clone(), route_features: route.features.clone(),
        body: serde_json::json!({"model":"current","messages":[{"role":"user","content":"test"}],"max_tokens":8192}),
    };
    let target = repository.resolve_attempt_target(&request).await?;
    merge_chat_provider_params(request.body, &target.provider_params, request.protocol, target.output_limit)
        .map_err(|_| ProductionRepositoryError::InvalidData)
}

async fn fixture() -> (nomifun_db::Database, CanonicalChatRouteRecord, ResolvedChatRoute) {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let pool = database.pool();
    sqlx::query("INSERT INTO providers (provider_id,platform,name,base_url,auth_scheme,credentials_encrypted,enabled,config_revision,created_at,updated_at) VALUES (?,'custom','Test','https://models.example/v1','bearer','test-ciphertext',1,0,0,0)")
        .bind(PROVIDER).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO provider_models (provider_id,model,enabled,sort_order,created_at,updated_at) VALUES (?,'current',1,0,0,0)")
        .bind(PROVIDER).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO provider_model_capabilities (provider_id,model,task,traits,protocol,connection_role,provider_params,context_limit,output_limit,compaction_threshold_pct,created_at,updated_at) VALUES (?,'current','chat','[\"reasoning\"]','openai.chat_text','default','{\"reasoning_effort\":\"low\"}',32768,2048,75,0,0)")
        .bind(PROVIDER).execute(pool).await.unwrap();
    let primary = ChatRouteCandidate {
        model_route_id: "turn-model-route".into(), model_route_revision: 1,
        provider_id: PROVIDER.into(), model: "current".into(), protocol: ChatRouteProtocol::OpenaiChat,
        connection_config_ref: "default".into(),
        config_revision_digest: provider_model_config_digest(pool, &PROVIDER.into(), "current").await.unwrap(),
        credential_ref: "opaque-test-reference".into(),
        features: BTreeSet::from([ChatRouteFeature::TextInput, ChatRouteFeature::TextOutput, ChatRouteFeature::Reasoning]),
        activation_features: BTreeSet::new(),
    };
    let route = convert_chat_route_candidate(&primary).unwrap();
    let record = CanonicalChatRouteRecord { schema: ChatRouteRecordSchema::V1, task: ChatRouteTask::AgentChat, primary, failovers: Vec::new() };
    sqlx::query("INSERT INTO agent_presets (preset_id,owner_ref_json,source_json,display_json,current_stable_revision,created_at) VALUES ('preset','{}','{}','{}',1,0)")
        .execute(pool).await.unwrap();
    persist_record(pool, "preset@1", 1, &record).await;
    (database, record, route)
}

#[tokio::test]
async fn turn_configuration_preserves_current_inference_then_refreshes_next_turn() {
    let (database, record, route) = fixture().await;
    let pool = database.pool();
    let view = TurnModelConfiguration::default();
    view.capture(pool, "turn-one", &record).await.unwrap();
    sqlx::query("UPDATE provider_model_capabilities SET traits='[\"vision_input\",\"reasoning\"]',provider_params='{\"reasoning_effort\":\"high\"}',context_limit=1000000,output_limit=4096,compaction_threshold_pct=80 WHERE provider_id=?")
        .bind(PROVIDER).execute(pool).await.unwrap();
    assert_ne!(provider_model_config_digest(pool, &PROVIDER.into(), "current").await.unwrap(), route.config_revision_digest);
    assert_eq!(view.digest_for_route(pool, &route).await.unwrap(), route.config_revision_digest);
    let saved = view.candidate(&route).unwrap();
    let chat = saved.capabilities.iter().find(|capability| capability.task == "chat").unwrap();
    assert_eq!(serde_json::from_str::<Value>(&chat.provider_params).unwrap()["reasoning_effort"], "low");
    let identity = ChatRouteIdentity::new("preset@1", nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT, "turn-model-route".into(), 1);
    let wire = attempt_wire(pool, &view, &identity).await.unwrap();
    assert_eq!(wire["reasoning_effort"], "low");
    assert_eq!(wire["max_tokens"], 2048);
    let facts = view.model_facts(&identity, &record).unwrap();
    assert_eq!(facts.candidates()[0].limits.context_tokens, Some(32768));
    assert_eq!(facts.candidates()[0].limits.output_tokens, Some(2048));
    assert_eq!(facts.compaction_threshold_pct(), 75);
    assert!(view.clear("another-turn").is_err());
    assert!(view.belongs_to("turn-one"));
    view.clear("turn-one").unwrap();
    assert!(view.candidate(&route).is_err());
    let mut next = record.clone();
    next.primary.config_revision_digest = provider_model_config_digest(pool, &PROVIDER.into(), "current").await.unwrap();
    view.capture(pool, "turn-two", &next).await.unwrap();
    persist_record(pool, "preset@2", 2, &next).await;
    let next_identity = ChatRouteIdentity::new("preset@2", nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT, "turn-model-route".into(), 1);
    let wire = attempt_wire(pool, &view, &next_identity).await.unwrap();
    assert_eq!(wire["reasoning_effort"], "high");
    assert_eq!(wire["max_tokens"], 4096);
    let facts = view.model_facts(&next_identity, &next).unwrap();
    assert_eq!(facts.candidates()[0].limits.context_tokens, Some(1_000_000));
    assert_eq!(facts.candidates()[0].limits.output_tokens, Some(4096));
    assert_eq!(facts.compaction_threshold_pct(), 80);
    let next_route = convert_chat_route_candidate(&next.primary).unwrap();
    let next = view.candidate(&next_route).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&next.capabilities[0].provider_params).unwrap()["reasoning_effort"], "high");
}

#[tokio::test]
async fn turn_configuration_keeps_credentials_enabled_transport_and_task_existence_live() {
    let (database, record, route) = fixture().await;
    let pool = database.pool();
    let view = TurnModelConfiguration::default();
    view.capture(pool, "turn-one", &record).await.unwrap();
    let identity = ChatRouteIdentity::new("preset@1", nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT, "turn-model-route".into(), 1);
    for (mutation, restore) in [
        ("UPDATE providers SET credentials_encrypted='rotated' WHERE provider_id=?", "UPDATE providers SET credentials_encrypted='test-ciphertext' WHERE provider_id=?"),
        ("UPDATE providers SET auth_scheme='query_key:token' WHERE provider_id=?", "UPDATE providers SET auth_scheme='bearer' WHERE provider_id=?"),
        ("UPDATE providers SET enabled=0 WHERE provider_id=?", "UPDATE providers SET enabled=1 WHERE provider_id=?"),
        ("UPDATE provider_models SET enabled=0 WHERE provider_id=?", "UPDATE provider_models SET enabled=1 WHERE provider_id=?"),
        ("UPDATE providers SET base_url='https://different.example/v1' WHERE provider_id=?", "UPDATE providers SET base_url='https://models.example/v1' WHERE provider_id=?"),
        ("UPDATE provider_model_capabilities SET endpoint='/different' WHERE provider_id=?", "UPDATE provider_model_capabilities SET endpoint=NULL WHERE provider_id=?"),
        ("UPDATE provider_model_capabilities SET protocol='openai.responses' WHERE provider_id=?", "UPDATE provider_model_capabilities SET protocol='openai.chat_text' WHERE provider_id=?"),
        ("UPDATE provider_model_capabilities SET connection_role='alternate' WHERE provider_id=?", "UPDATE provider_model_capabilities SET connection_role='default' WHERE provider_id=?"),
    ] {
        sqlx::query(mutation).bind(PROVIDER).execute(pool).await.unwrap();
        assert_ne!(view.digest_for_route(pool, &route).await.unwrap(), route.config_revision_digest, "{mutation}");
        assert!(attempt_wire(pool, &view, &identity).await.is_err(), "{mutation}");
        sqlx::query(restore).bind(PROVIDER).execute(pool).await.unwrap();
        assert_eq!(view.digest_for_route(pool, &route).await.unwrap(), route.config_revision_digest, "{restore}");
    }
    sqlx::query("DELETE FROM provider_model_capabilities WHERE provider_id=?").bind(PROVIDER).execute(pool).await.unwrap();
    assert_ne!(view.digest_for_route(pool, &route).await.unwrap(), route.config_revision_digest);
}

#[tokio::test]
async fn turn_configuration_capture_race_fails_closed_and_publishes_no_partial_view() {
    let (database, record, route) = fixture().await;
    let pool = database.pool();
    sqlx::query("UPDATE provider_model_capabilities SET context_limit=1000000 WHERE provider_id=?")
        .bind(PROVIDER).execute(pool).await.unwrap();
    let view = TurnModelConfiguration::default();
    assert!(view.capture(pool, "turn-one", &record).await.is_err());
    assert!(!view.belongs_to("turn-one"));
    assert!(view.candidate(&route).is_err());
}
