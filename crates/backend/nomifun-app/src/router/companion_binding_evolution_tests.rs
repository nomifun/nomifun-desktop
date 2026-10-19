//! Current-generation retired device bindings advance through a canonical
//! transition. The fixture authors its old opening once; no live binding or
//! immutable Revision/Snapshot is rewritten to manufacture the predecessor.
use super::*;

async fn install_old_device_session(flow: &PhysicalEntryFixture, official: bool) -> AgentSessionId {
    let router = &flow.fixture._router;
    let projection = flow
        .fixture
        .owner
        .get_session(&flow.owner, flow.robot_id.as_ref())
        .await
        .unwrap();
    let legacy_projection = flow
        .fixture
        .owner
        .get_session(&flow.owner, &flow.fixture.host.options.conversation_id)
        .await
        .unwrap();
    let legacy_model = serde_json::to_value(
        legacy_projection
            .agent_snapshot
            .as_ref()
            .unwrap()
            .resolved_model
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let editor = api(router, "/api/agent-presets/from-template/chat.minimal", json!({
        "reuse_existing":false, "display_name":"Retired device seed fixture", "model":legacy_model,
    })).await;
    let preset = editor["preset"]["preset_id"].as_str().unwrap();
    let mut draft = editor["draft"].clone();
    // The pre-unification seed's actual five-module authority. It lacks the
    // ordinary research/files/desktop capabilities selected for the new main.
    draft["document"]["enabled_capabilities"] = json!([
        {"capability":{"id":"agent.collaboration"},"action_allowlist":["agent/delegate"]},
        {"capability":{"id":"companion"},"action_allowlist":["companion/evolve","companion/learn"]},
        {"capability":{"id":"companion.memory"},"action_allowlist":["companion.memory/recall","companion.memory/write"]},
        {"capability":{"id":"robot"},"action_allowlist":["robot/device","robot/display","robot/motion","robot/vision"]},
        {"capability":{"id":"session.management"},"action_allowlist":["session.management/execution_answer_decision","session.management/execution_cancel","session.management/execution_status"]},
    ]);
    draft["document"]["instructions"] = json!("Historical device-only official configuration.");
    let _: nomifun_api_types::AgentPresetDraftDto = serde_json::from_value(draft.clone())
        .expect("the old fixture must use the formal authoring DTO; resource requirements are compiler-derived");
    api(
        router,
        &format!("/api/agent-presets/{preset}/revisions"),
        json!({
            "expected_current_revision":draft["current_revision"], "draft":draft,
            "reason":"bootstrap a readable current-generation retired seed",
        }),
    )
    .await;
    let selections = projection.agent_snapshot.as_ref().unwrap().canonical_binding.as_ref().unwrap()
        .typed_resource_bindings.iter()
        .filter(|resource| matches!(resource.resource_kind.as_str(), "companion"|"companion_memory"|"robot"))
        .map(|resource| json!({"resource_kind":resource.resource_kind,"resource_id":resource.resource_id}))
        .collect::<Vec<_>>();
    let temporary = api(
        router,
        "/api/agent-sessions",
        json!({
            "preset_id":preset,"model":legacy_model,"resource_selections":selections,
        }),
    )
    .await;
    let temporary_id: AgentSessionId = temporary["agent_session_id"]
        .as_str()
        .unwrap()
        .to_owned()
        .into();
    let old_projection = flow
        .fixture
        .owner
        .get_session(&flow.owner, temporary_id.as_ref())
        .await
        .unwrap();
    let old_snapshot = old_projection.agent_snapshot.as_ref().unwrap();
    assert_ne!(
        old_snapshot.resolved_model,
        projection.agent_snapshot.as_ref().unwrap().resolved_model,
        "the predecessor carries an independent device-model override"
    );
    assert!(
        !old_snapshot
            .enabled_capabilities
            .iter()
            .any(|capability| capability == "web.research")
    );
    let store = flow.fixture.owner.canonical().store();
    let mut old_session = store.get_live_session(&temporary_id).await.unwrap();
    let binding = old_session.agent_binding.clone();

    // Fixture bootstrap establishes the host-owned provenance used by old
    // hidden official variants, before the device's canonical opening exists.
    // The saved Revision/Snapshot digests and contents remain exact.
    if official {
        sqlx::query("UPDATE agent_presets SET source_json=?,display_json=? WHERE preset_id=?")
            .bind(json!("official").to_string())
            .bind(json!({"display_name":"companion.robot","session_only":true}).to_string())
            .bind(binding.preset_revision_ref.preset_id.as_ref())
            .execute(flow.fixture.pool())
            .await
            .unwrap();
    }
    let binding_dto = serde_json::from_value(serde_json::to_value(&binding).unwrap()).unwrap();
    assert_eq!(
        flow.fixture
            .voice_control
            .is_retired_companion_device_binding(
                &nomifun_agent_contracts::UserId::from(flow.owner.clone()),
                &binding_dto,
            )
            .await
            .unwrap(),
        official
    );

    let modern = store.get_live_session(&flow.robot_id).await.unwrap();
    let old_id: AgentSessionId = nomifun_common::generate_id().into();
    old_session.agent_session_id = old_id.clone();
    old_session.metadata.companion = modern.metadata.companion;
    old_session.metadata.title = Some("Existing physical-entry conversation".into());
    old_session.next_seq = 1;
    let creation_key = "retired-device-fixture-thread";
    let opening_key = format!("user:{}:open:{creation_key}", flow.owner);
    let mut opening = nomifun_agent_session::CreateSessionRequest::new(
        old_session,
        nomifun_common::now_ms(),
        format!("open:{opening_key}"),
        "session_api".into(),
        opening_key.clone().into(),
        format!("open:{opening_key}").into(),
    );
    opening.opening_event_id = Some(format!("opening:{opening_key}").into());
    opening.activation_event_id = Some(format!("active-set:{opening_key}").into());
    opening.initial_active_capability_ids =
        store.active_capability_ids(&temporary_id).await.unwrap();
    let created = store.create_session(opening).await.unwrap();
    store
        .append_event(&nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: old_id.clone(),
            event_id: format!("ready:{opening_key}").into(),
            producer_id: "runtime_supervisor".into(),
            idempotency_key: format!("{opening_key}:ready").into(),
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind("session/ready".into()),
                kind_version: 1,
                correlation_id: format!("session:{}", old_id.as_ref()).into(),
                causation_event_id: Some(created.opening_ack.event_id),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    StrictJsonValue(json!({
                        "resolved_snapshot_ref":binding.resolved_snapshot_ref,
                    })),
                ),
            },
        })
        .await
        .unwrap();
    let registry = nomifun_companion::CompanionStore::open(
        &flow
            .fixture
            .services
            .data_dir
            .join("companion")
            .join("shared"),
        Some(&flow.owner),
    )
    .await
    .unwrap();
    registry
        .insert_companion_thread_for_role(
            old_id.as_ref(),
            &flow.companion_id,
            "Existing physical-entry conversation",
            nomifun_companion::CompanionRole::Robot,
            Some(&flow.device_id),
            Some(creation_key),
        )
        .await
        .unwrap();
    registry
        .set_companion_state(
            &flow.companion_id,
            &format!(
                "robot_receive_session:{}",
                flow.device_id
                    .bytes()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            old_id.as_ref(),
        )
        .await
        .unwrap();
    drop(registry);
    old_id
}

#[tokio::test]
async fn retired_official_device_binding_advances_in_place_then_directly_queries_weather() {
    let server = CompanionFlowModelServer::new().await;
    let mut flow =
        PhysicalEntryFixture::with_research(&server, "retired-device-binding-weather", true).await;
    install_search_provider(&flow.fixture._router, &server).await;
    let original_device_session = flow.robot_id.clone();
    let old_id = install_old_device_session(&flow, true).await;
    flow.robot_id = old_id.clone();
    let store = flow.fixture.owner.canonical().store();
    let before = store.get_live_session(&old_id).await.unwrap();
    let old_binding = before.agent_binding.clone();
    let snapshot_before: (String, String) = sqlx::query_as(
        "SELECT content_json,envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?",
    )
    .bind(old_binding.resolved_snapshot_ref.snapshot_id.as_ref())
    .fetch_one(flow.fixture.pool())
    .await
    .unwrap();
    let revision_before: (String,String) = sqlx::query_as(
        "SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE preset_id=? AND revision_no=?",
    ).bind(old_binding.preset_revision_ref.preset_id.as_ref()).bind(old_binding.preset_revision_ref.revision as i64)
        .fetch_one(flow.fixture.pool()).await.unwrap();
    let opening_before: String =
        sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND seq=1")
            .bind(old_id.as_ref())
            .fetch_one(flow.fixture.pool())
            .await
            .unwrap();
    let session_count_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
        .fetch_one(flow.fixture.pool())
        .await
        .unwrap();
    let original_device_head = store.head(&original_device_session).await.unwrap().last_seq;
    let desktop_head = store.head(&flow.desktop_id).await.unwrap().last_seq;

    let reused = flow
        .fixture
        .services
        .companion_service
        .ensure_session_for_role(
            &flow.companion_id,
            nomifun_companion::CompanionRole::Robot,
            Some(&flow.device_id),
        )
        .await
        .unwrap();
    assert_eq!(
        reused.conversation_id,
        old_id.as_ref(),
        "the canonical device receive Session keeps its identity"
    );
    let after = store.get_live_session(&old_id).await.unwrap();
    assert_eq!(after.metadata.companion, before.metadata.companion);
    assert_eq!(
        after.agent_binding.binding_version,
        old_binding.binding_version + 1
    );
    assert_ne!(
        after.agent_binding.resolved_snapshot_ref,
        old_binding.resolved_snapshot_ref
    );
    let retried_creation = flow
        .fixture
        .services
        .companion_service
        .create_session_for_role(
            &flow.companion_id,
            nomifun_companion::CompanionRole::Robot,
            Some(&flow.device_id),
            "retired-device-fixture-thread",
        )
        .await
        .unwrap();
    assert_eq!(
        retried_creation.conversation_id,
        old_id.as_ref(),
        "a lost old creation response reuses the adopted Session"
    );
    assert_eq!(
        store.get_live_session(&old_id).await.unwrap().agent_binding,
        after.agent_binding
    );
    let current = flow
        .fixture
        .owner
        .get_session(&flow.owner, old_id.as_ref())
        .await
        .unwrap();
    let desktop = flow
        .fixture
        .owner
        .get_session(&flow.owner, flow.desktop_id.as_ref())
        .await
        .unwrap();
    assert_eq!(
        current.agent_snapshot.as_ref().unwrap().resolved_model,
        desktop.agent_snapshot.as_ref().unwrap().resolved_model
    );
    assert_eq!(
        current
            .agent_snapshot
            .as_ref()
            .unwrap()
            .canonical_binding
            .as_ref()
            .unwrap()
            .preset_revision_ref,
        desktop
            .agent_snapshot
            .as_ref()
            .unwrap()
            .canonical_binding
            .as_ref()
            .unwrap()
            .preset_revision_ref
    );
    assert!(
        current
            .agent_snapshot
            .as_ref()
            .unwrap()
            .enabled_capability_actions["web.research"]
            .contains("web.research/search")
    );
    for resource in old_binding
        .typed_resource_bindings
        .iter()
        .filter(|resource| resource.resource_kind.as_ref() == "robot")
    {
        assert!(
            after
                .agent_binding
                .typed_resource_bindings
                .iter()
                .any(|next| next.resource_kind == resource.resource_kind
                    && next.resource_id == resource.resource_id
                    && next.owner_id == resource.owner_id)
        );
    }
    let transitions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed'")
        .bind(old_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(
        transitions, 1,
        "one explicit canonical ContextOnly binding transition advances this Session"
    );
    assert_eq!(
        server.state.requests.lock().unwrap().len(),
        0,
        "contract evolution does not call a model"
    );

    let utterance = flow.request("evolved-weather-request", WEATHER_QUERY);
    let operation = send_and_complete_robot_turn(
        &flow.fixture,
        &flow.owner,
        &utterance,
        "evolved-weather-input",
    )
    .await;
    let receipt = store.read_turn_receipt(&old_id, &operation).await.unwrap();
    let facts = store.turn_output_facts(&old_id, &operation).await.unwrap();
    let public = nomifun_agent_session::canonical_turn_public_result(&facts, &receipt).unwrap();
    assert!(public.result_text.unwrap().contains(WEATHER_RESULT));
    assert_eq!(server.state.searches.lock().unwrap().len(), 1);
    assert_eq!(
        server.state.requests.lock().unwrap().len(),
        2,
        "the evolved device Session performs research directly"
    );
    assert!(
        flow.fixture
            .voice_execution
            .list(&flow.owner, Some(20), Some(0))
            .await
            .unwrap()
            .is_empty()
    );
    let effects = store.list_effects(&old_id).await.unwrap();
    assert!(facts.event_payloads.values().any(|payload| matches!(
        serde_json::from_value::<nomifun_agent_runtime::AgentEngineEvent>(payload["event"].clone()),
        Ok(nomifun_agent_runtime::AgentEngineEvent::ToolCompleted {result,..})
            if result.call_id.as_ref()=="weather-search" && !result.is_error
    )), "read-only research must retain its canonical tool receipt without a mutation Effect");
    assert!(
        effects
            .iter()
            .all(|effect| effect.capability_module.as_ref() != "agent.collaboration")
    );
    let sessions_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
        .fetch_one(flow.fixture.pool())
        .await
        .unwrap();
    assert_eq!(sessions_after, session_count_before);
    let transitions_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed'")
        .bind(old_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(
        transitions_after, 1,
        "re-entry and the real input reuse the committed transition"
    );
    assert_eq!(
        store.head(&original_device_session).await.unwrap().last_seq,
        original_device_head
    );
    assert_eq!(
        store.head(&flow.desktop_id).await.unwrap().last_seq,
        desktop_head
    );
    let snapshot_after: (String, String) = sqlx::query_as(
        "SELECT content_json,envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?",
    )
    .bind(old_binding.resolved_snapshot_ref.snapshot_id.as_ref())
    .fetch_one(flow.fixture.pool())
    .await
    .unwrap();
    let revision_after: (String,String) = sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE preset_id=? AND revision_no=?")
        .bind(old_binding.preset_revision_ref.preset_id.as_ref()).bind(old_binding.preset_revision_ref.revision as i64)
        .fetch_one(flow.fixture.pool()).await.unwrap();
    let opening_after: String =
        sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND seq=1")
            .bind(old_id.as_ref())
            .fetch_one(flow.fixture.pool())
            .await
            .unwrap();
    assert_eq!(snapshot_after, snapshot_before);
    assert_eq!(revision_after, revision_before);
    assert_eq!(opening_after, opening_before);
    flow.finish().await;
    server.close().await;
}

#[tokio::test]
async fn old_personal_device_binding_adopts_once_and_later_main_defaults_do_not_replace_it() {
    let server = CompanionFlowModelServer::new().await;
    let mut flow =
        PhysicalEntryFixture::with_research(&server, "old-personal-device-binding", true).await;
    install_search_provider(&flow.fixture._router, &server).await;
    let old_id = install_old_device_session(&flow, false).await;
    flow.robot_id = old_id.clone();
    let store = flow.fixture.owner.canonical().store();
    let old = store.get_live_session(&old_id).await.unwrap();
    let old_snapshot: (String, String) = sqlx::query_as(
        "SELECT content_json,envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?",
    )
    .bind(old.agent_binding.resolved_snapshot_ref.snapshot_id.as_ref())
    .fetch_one(flow.fixture.pool())
    .await
    .unwrap();
    let old_revision: (String,String) = sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE preset_id=? AND revision_no=?")
        .bind(old.agent_binding.preset_revision_ref.preset_id.as_ref()).bind(old.agent_binding.preset_revision_ref.revision as i64)
        .fetch_one(flow.fixture.pool()).await.unwrap();
    let reused = flow
        .fixture
        .services
        .companion_service
        .ensure_session_for_role(
            &flow.companion_id,
            nomifun_companion::CompanionRole::Robot,
            Some(&flow.device_id),
        )
        .await
        .unwrap();
    assert_eq!(reused.conversation_id, old_id.as_ref());
    let adopted = store.get_live_session(&old_id).await.unwrap();
    assert_ne!(
        adopted.agent_binding.resolved_snapshot_ref,
        old.agent_binding.resolved_snapshot_ref
    );
    let projection = flow
        .fixture
        .owner
        .get_session(&flow.owner, old_id.as_ref())
        .await
        .unwrap();
    assert!(
        projection
            .agent_snapshot
            .as_ref()
            .unwrap()
            .enabled_capabilities
            .iter()
            .any(|capability| capability == "web.research")
    );
    let model = serde_json::to_value(
        projection
            .agent_snapshot
            .as_ref()
            .unwrap()
            .resolved_model
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let changed_main = shared_preset(&flow.fixture._router, &model, false).await;
    request(&flow.fixture._router,"PUT",&format!("/api/product-agent-bindings/companion/{}",flow.companion_id),json!({
        "selection":{"kind":"preset","preset_id":changed_main},"model":model,"resource_selections":[],
    })).await;
    let read_after_defaults = flow
        .fixture
        .owner
        .get_session(&flow.owner, old_id.as_ref())
        .await
        .unwrap();
    assert_eq!(
        read_after_defaults
            .agent_snapshot
            .as_ref()
            .unwrap()
            .canonical_binding
            .as_ref()
            .unwrap()
            .resolved_snapshot_ref,
        projection
            .agent_snapshot
            .as_ref()
            .unwrap()
            .canonical_binding
            .as_ref()
            .unwrap()
            .resolved_snapshot_ref,
        "GET does not silently apply later primary defaults"
    );
    let creation_retry = flow
        .fixture
        .services
        .companion_service
        .create_session_for_role(
            &flow.companion_id,
            nomifun_companion::CompanionRole::Robot,
            Some(&flow.device_id),
            "retired-device-fixture-thread",
        )
        .await
        .unwrap();
    assert_eq!(creation_retry.conversation_id, old_id.as_ref());
    let still_same = flow
        .fixture
        .services
        .companion_service
        .ensure_session_for_role(
            &flow.companion_id,
            nomifun_companion::CompanionRole::Robot,
            Some(&flow.device_id),
        )
        .await
        .unwrap();
    assert_eq!(still_same.conversation_id, old_id.as_ref());
    assert_eq!(
        store.get_live_session(&old_id).await.unwrap().agent_binding,
        adopted.agent_binding,
        "the committed adoption is a one-time transition, not a live defaults follower"
    );
    let transitions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed' AND idempotency_key=?")
        .bind(old_id.as_ref()).bind(format!("companion-unified-agent:{}",old_id.as_ref()))
        .fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(transitions, 1);
    let utterance = flow.request("adopted-personal-weather-request", WEATHER_QUERY);
    let operation = send_and_complete_robot_turn(
        &flow.fixture,
        &flow.owner,
        &utterance,
        "adopted-personal-weather-input",
    )
    .await;
    let receipt = store.read_turn_receipt(&old_id, &operation).await.unwrap();
    let facts = store.turn_output_facts(&old_id, &operation).await.unwrap();
    assert!(
        nomifun_agent_session::canonical_turn_public_result(&facts, &receipt)
            .unwrap()
            .result_text
            .unwrap()
            .contains(WEATHER_RESULT)
    );
    assert_eq!(
        server.state.searches.lock().unwrap().len(),
        1,
        "the frozen adopted research grant remains usable after defaults change"
    );
    assert_eq!(server.state.requests.lock().unwrap().len(), 2);
    assert!(
        flow.fixture
            .voice_execution
            .list(&flow.owner, Some(20), Some(0))
            .await
            .unwrap()
            .is_empty()
    );
    let snapshot_after: (String, String) = sqlx::query_as(
        "SELECT content_json,envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?",
    )
    .bind(old.agent_binding.resolved_snapshot_ref.snapshot_id.as_ref())
    .fetch_one(flow.fixture.pool())
    .await
    .unwrap();
    let revision_after: (String,String) = sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE preset_id=? AND revision_no=?")
        .bind(old.agent_binding.preset_revision_ref.preset_id.as_ref()).bind(old.agent_binding.preset_revision_ref.revision as i64)
        .fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(snapshot_after, old_snapshot);
    assert_eq!(revision_after, old_revision);
    flow.finish().await;
    server.close().await;
}

#[tokio::test]
async fn recording_onset_adopts_retired_binding_before_freezing_the_same_real_input() {
    let server = CompanionFlowModelServer::new().await;
    let mut flow =
        PhysicalEntryFixture::with_research(&server, "onset-adopts-retired-binding", true).await;
    install_search_provider(&flow.fixture._router, &server).await;
    let old_id = install_old_device_session(&flow, true).await;
    flow.robot_id = old_id.clone();
    let store = flow.fixture.owner.canonical().store();
    let before = store.get_live_session(&old_id).await.unwrap().agent_binding;
    let backend = flow
        .fixture
        .services
        .robot
        .as_ref()
        .unwrap()
        .conversation_backend_for_test();
    let context = backend
        .capture_input_context(
            &flow.companion_id,
            &flow.device_id,
            old_id.as_ref(),
            &flow.connection,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    let adopted = store.get_live_session(&old_id).await.unwrap().agent_binding;
    assert_ne!(before.resolved_snapshot_ref, adopted.resolved_snapshot_ref);
    assert_eq!(
        context.binding_version, adopted.binding_version,
        "recording freezes the completed evolution, not the retired predecessor"
    );
    assert_eq!(
        server.state.requests.lock().unwrap().len(),
        0,
        "onset evolution does not call a model"
    );
    let mut utterance = flow.request("onset-adopted-weather", WEATHER_QUERY);
    utterance.captured_context = Some(context.clone());
    assert!(backend.input_context_matches(&utterance).await.unwrap());
    let operation = send_and_complete_robot_turn(
        &flow.fixture,
        &flow.owner,
        &utterance,
        "onset-adopted-weather",
    )
    .await;
    assert!(
        backend.input_context_matches(&utterance).await.unwrap(),
        "admission does not evolve away from its own recording context"
    );
    let receipt = store.read_turn_receipt(&old_id, &operation).await.unwrap();
    let facts = store.turn_output_facts(&old_id, &operation).await.unwrap();
    assert!(
        nomifun_agent_session::canonical_turn_public_result(&facts, &receipt)
            .unwrap()
            .result_text
            .unwrap()
            .contains(WEATHER_RESULT)
    );
    assert_eq!(server.state.requests.lock().unwrap().len(), 2);
    assert_eq!(server.state.searches.lock().unwrap().len(), 1);
    assert_eq!(
        store.get_live_session(&old_id).await.unwrap().agent_binding,
        adopted
    );
    flow.finish().await;
    server.close().await;
}
