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
        input_constraints: None,
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

#[tokio::test]
async fn model_scoped_digest_ignores_unrelated_named_connections_but_checks_selected_connection() {
    let (database, record, route) = fixture().await;
    let pool = database.pool();
    let view = TurnModelConfiguration::default();
    view.capture(pool, "turn-default", &record).await.unwrap();
    let identity = ChatRouteIdentity::new("preset@1", nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT, "turn-model-route".into(), 1);
    sqlx::query("INSERT INTO provider_connections (connection_id,provider_id,role,base_url,auth_scheme,credentials_encrypted,extra,created_at,updated_at) VALUES ('0190f5fe-7c00-7a00-8000-000000000081',?,'anthropic','https://gateway.example','header_key:x-api-key','other-ciphertext','{}',0,0)")
        .bind(PROVIDER).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO provider_models (provider_id,model,enabled,sort_order,created_at,updated_at) VALUES (?,'new-anthropic',1,0,0,0)")
        .bind(PROVIDER).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO provider_model_capabilities (provider_id,model,task,traits,protocol,connection_role,provider_params,created_at,updated_at) VALUES (?,'new-anthropic','chat','[]','anthropic.messages','anthropic','{}',0,0)")
        .bind(PROVIDER).execute(pool).await.unwrap();
    assert_eq!(provider_model_config_digest(pool, &PROVIDER.into(), "current").await.unwrap(), route.config_revision_digest);
    assert_eq!(view.digest_for_route(pool, &route).await.unwrap(), route.config_revision_digest);
    assert!(attempt_wire(pool, &view, &identity).await.is_ok());
    sqlx::query("UPDATE provider_connections SET credentials_encrypted='unrelated-rotation' WHERE provider_id=? AND role='anthropic'")
        .bind(PROVIDER).execute(pool).await.unwrap();
    assert_eq!(view.digest_for_route(pool, &route).await.unwrap(), route.config_revision_digest);

    sqlx::query("INSERT INTO provider_connections (connection_id,provider_id,role,base_url,auth_scheme,credentials_encrypted,extra,created_at,updated_at) VALUES ('0190f5fe-7c00-7a00-8000-000000000082',?,'selected','https://selected.example/v1','bearer','selected-ciphertext','{}',0,0)")
        .bind(PROVIDER).execute(pool).await.unwrap();
    sqlx::query("UPDATE provider_model_capabilities SET connection_role='selected' WHERE provider_id=? AND model='current'")
        .bind(PROVIDER).execute(pool).await.unwrap();
    assert_ne!(view.digest_for_route(pool, &route).await.unwrap(), route.config_revision_digest);
    view.clear("turn-default").unwrap();
    let mut selected_record = record;
    selected_record.primary.connection_config_ref = "0190f5fe-7c00-7a00-8000-000000000082".into();
    selected_record.primary.config_revision_digest = provider_model_config_digest(pool, &PROVIDER.into(), "current").await.unwrap();
    let selected_route = convert_chat_route_candidate(&selected_record.primary).unwrap();
    view.capture(pool, "turn-selected", &selected_record).await.unwrap();
    for (mutation, restore) in [
        ("UPDATE provider_connections SET credentials_encrypted='rotated' WHERE provider_id=? AND role='selected'", "UPDATE provider_connections SET credentials_encrypted='selected-ciphertext' WHERE provider_id=? AND role='selected'"),
        ("UPDATE provider_connections SET base_url='https://changed.example/v1' WHERE provider_id=? AND role='selected'", "UPDATE provider_connections SET base_url='https://selected.example/v1' WHERE provider_id=? AND role='selected'"),
        ("UPDATE provider_connections SET auth_scheme='header_key:x-api-key' WHERE provider_id=? AND role='selected'", "UPDATE provider_connections SET auth_scheme='bearer' WHERE provider_id=? AND role='selected'"),
    ] {
        sqlx::query(mutation).bind(PROVIDER).execute(pool).await.unwrap();
        assert_ne!(view.digest_for_route(pool, &selected_route).await.unwrap(), selected_route.config_revision_digest, "{mutation}");
        assert!(!route_config_matches(pool, &selected_route).await.unwrap());
        sqlx::query(restore).bind(PROVIDER).execute(pool).await.unwrap();
        assert_eq!(view.digest_for_route(pool, &selected_route).await.unwrap(), selected_route.config_revision_digest);
    }
}

#[tokio::test]
async fn existing_model_v1_digest_requires_exact_graph_during_frozen_turn() {
    let (database, mut record, _) = fixture().await;
    let pool = database.pool();
    record.primary.config_revision_digest = provider_legacy_model_config_digest(pool, &PROVIDER.into(), "current").await.unwrap();
    let route = convert_chat_route_candidate(&record.primary).unwrap();
    let view = TurnModelConfiguration::default();
    view.capture(pool, "existing-turn", &record).await.unwrap();
    assert_eq!(view.candidate(&route).unwrap().digest_scope, ProviderConfigurationScope::ModelV1);
    assert!(route_config_matches(pool, &route).await.unwrap());
    let repository = ProductionProviderRepository::new(pool.clone());
    assert_eq!(repository.find_provider_for_route(&route).await.unwrap().unwrap().config_revision_digest, route.config_revision_digest);
    sqlx::query("INSERT INTO provider_connections (connection_id,provider_id,role,base_url,auth_scheme,credentials_encrypted,extra,created_at,updated_at) VALUES ('0190f5fe-7c00-7a00-8000-000000000081',?,'anthropic','https://gateway.example','header_key:x-api-key','other-ciphertext','{}',0,0)")
        .bind(PROVIDER).execute(pool).await.unwrap();
    assert_ne!(view.digest_for_route(pool, &route).await.unwrap(), route.config_revision_digest);
    assert!(!route_config_matches(pool, &route).await.unwrap(), "an old digest without old connection facts must not be relaxed");
    let new_view = TurnModelConfiguration::default();
    assert!(new_view.capture(pool, "later-turn", &record).await.is_err());
}

#[tokio::test]
async fn production_host_route_expresses_signed_anthropic_continuation_and_cache() {
    use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatProviderReasoning,
        ChatRole, PromptCachePolicy, protocol_features, recorded_conformance_fixtures};
    let (database, mut record, _) = fixture().await;
    let pool = database.pool();
    sqlx::query("UPDATE providers SET auth_scheme='header_key:x-api-key' WHERE provider_id=?")
        .bind(PROVIDER).execute(pool).await.unwrap();
    sqlx::query("UPDATE provider_model_capabilities SET protocol='anthropic.messages',traits='[]',provider_params='{}' WHERE provider_id=? AND model='current'")
        .bind(PROVIDER).execute(pool).await.unwrap();
    record.primary.protocol = ChatRouteProtocol::Anthropic;
    record.primary.config_revision_digest = provider_model_config_digest(pool, &PROVIDER.into(), "current").await.unwrap();
    let stored_features = record.primary.features.clone();
    assert!(!stored_features.contains(&ChatRouteFeature::ReasoningSignature));
    persist_record(pool, "preset@2", 2, &record).await;
    let registry = ConnectionCredentialLeaseRegistry::default();
    let resolver = ProductionRouteResolver::new(ProductionRepositorySet::new(
        Arc::new(ProductionProviderRepository::new(pool.clone())),
        Arc::new(ProductionConnectionRepository::new(pool.clone(), registry.clone())),
        Arc::new(ProductionModelRepository::new(pool.clone())),
    ));
    let identity = ChatRouteIdentity::new("preset@2", nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
        "turn-model-route".into(), 1);
    let resolved = resolver.resolve(&identity).await.unwrap().primary;
    let mut input = recorded_conformance_fixtures().into_iter()
        .find(|fixture| fixture.protocol == ChatProtocol::Anthropic).unwrap().request.input;
    input.messages = vec![ChatMessage { role: ChatRole::Assistant, provider_round_id: None,
        content: vec![ChatContentPart::ProviderReasoning {
            block: ChatProviderReasoning::AnthropicThinking {
                text: "native thought".into(), signature: "opaque-signature".into(),
                route_digest: Some(digest_payload(&resolved).unwrap()),
            }
        }] }];
    input.prompt_cache = PromptCachePolicy::Ephemeral;
    input.tools.clear();
    input.tool_choice = nomifun_chat_model_broker::ChatToolChoice::None;
    input.validate().unwrap();
    let required = input.required_features();
    assert!(required.contains(&ChatModelFeature::ReasoningSignature));
    assert!(required.contains(&ChatModelFeature::PromptCache));
    assert!(resolved.features.is_superset(&required), "persisted host route must express its next native step");
    assert!(protocol_features(resolved.protocol).is_superset(&required));
    assert_eq!(record.primary.features, stored_features, "immutable route facts are not rewritten");
    assert!(registry.leases.read().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_input_constraints_are_frozen_and_enter_every_configuration_digest_scope() {
    let (database, mut record, route)=fixture().await; let pool=database.pool();
    let provider=ProviderIdRef::from(PROVIDER);
    let scopes=[ProviderConfigurationScope::ModelV2,ProviderConfigurationScope::ModelV1,ProviderConfigurationScope::ProviderV1];
    let mut original=Vec::new();
    for scope in scopes { original.push(provider_config_digest_inner(pool,&provider,
        if scope==ProviderConfigurationScope::ProviderV1 {None} else {Some("current")},scope).await.unwrap()); }
    let view=TurnModelConfiguration::default(); view.capture(pool,"original-turn",&record).await.unwrap();
    sqlx::query("UPDATE provider_model_capabilities SET input_constraints='{}' WHERE provider_id=?")
        .bind(PROVIDER).execute(pool).await.unwrap();
    for (scope,expected) in scopes.into_iter().zip(&original) { assert_eq!(provider_config_digest_inner(pool,&provider,
        if scope==ProviderConfigurationScope::ProviderV1 {None} else {Some("current")},scope).await.unwrap(),*expected); }
    sqlx::query("UPDATE provider_model_capabilities SET input_constraints=? WHERE provider_id=?")
        .bind(r#"{"image_input":false}"#).bind(PROVIDER).execute(pool).await.unwrap();
    for (scope,expected) in scopes.into_iter().zip(&original) { assert_ne!(provider_config_digest_inner(pool,&provider,
        if scope==ProviderConfigurationScope::ProviderV1 {None} else {Some("current")},scope).await.unwrap(),*expected); }
    assert_eq!(view.digest_for_route(pool,&route).await.unwrap(),route.config_revision_digest);
    assert!(!route_config_matches(pool,&route).await.unwrap());
    assert!(TurnModelConfiguration::default().capture(pool,"stale-wide-route",&record).await.is_err());
    view.clear("original-turn").unwrap();
    record.primary.input_constraints=Some(nomifun_agent_contracts::ModelInputConstraints {image_input:Some(false)});
    record.primary.features.remove(&ChatRouteFeature::ImageInput);
    record.primary.config_revision_digest=provider_model_config_digest(pool,&provider,"current").await.unwrap();
    let constrained=convert_chat_route_candidate(&record.primary).unwrap();
    assert!(!constrained.features.contains(&ChatModelFeature::ImageInput));
    view.capture(pool,"constrained-turn",&record).await.unwrap();
    let saved=view.candidate(&constrained).unwrap();
    assert_eq!(saved.capabilities.iter().find(|capability|capability.task=="chat").unwrap().input_constraints.as_deref(),Some(r#"{"image_input":false}"#));
    sqlx::query("UPDATE provider_model_capabilities SET input_constraints=NULL WHERE provider_id=?")
        .bind(PROVIDER).execute(pool).await.unwrap();
    assert_eq!(view.digest_for_route(pool,&constrained).await.unwrap(),constrained.config_revision_digest);
    assert!(!route_config_matches(pool,&constrained).await.unwrap());
}
