use super::*;
use crate::{ClearSessionContext, ContextClearBoundary, ContextClearPreparation, NativeExecutionClaim};

fn command(key: &str) -> ClearSessionContext {
    ClearSessionContext { idempotency_key: key.into(), reason: "user_requested".into() }
}

async fn prepare(store: &AgentSessionStore, session: &AgentSessionId, request: &ClearSessionContext) -> ContextClearBoundary {
    match store.preflight_context_clear(&owner(), session, request).await.unwrap() {
        ContextClearPreparation::Ready(boundary) => boundary,
        ContextClearPreparation::AlreadyCleared(_) => panic!("first clear must prepare"),
    }
}

#[tokio::test]
async fn context_clear_preserves_session_history_and_exact_replay_does_not_clear_a_successor() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "clear-preserves").await;
    let session_before = store.get_live_session(&session.agent_session_id).await.unwrap();
    let before = store.read_events(&session.agent_session_id, None, 500).await.unwrap().events;
    let request = command("clear-preserves-command");
    let boundary = prepare(&store, &session.agent_session_id, &request).await;
    let cleared = store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.unwrap();
    assert!(!cleared.duplicate);
    assert_eq!(cleared.record.as_ref().unwrap().kind.0, "context/cleared");
    let after = store.read_events(&session.agent_session_id, None, 500).await.unwrap().events;
    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(after.len(), before.len() + 1);
    let session_after = store.get_live_session(&session.agent_session_id).await.unwrap();
    assert_eq!(session_after.agent_binding, session_before.agent_binding);
    assert_eq!(session_after.metadata, session_before.metadata);
    assert_eq!(session_after.owner_ref, session_before.owner_ref);
    store.start_turn(&session.agent_session_id, "session_api".into(), "successor-input".into(), "successor".into(),
        StrictJsonValue(json!({"content":"keep the next context"}))).await.unwrap();
    let head = store.head(&session.agent_session_id).await.unwrap();
    let replay = store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.ack, cleared.ack);
    assert!(matches!(store.preflight_context_clear(&owner(), &session.agent_session_id, &request).await.unwrap(),
        ContextClearPreparation::AlreadyCleared(result) if result.ack == cleared.ack));
    assert_eq!(store.head(&session.agent_session_id).await.unwrap(), head);
    let mut changed = command("clear-preserves-command"); changed.reason = "different".into();
    assert!(matches!(store.preflight_context_clear(&owner(), &session.agent_session_id, &changed).await,
        Err(SessionStoreError::IdempotencyConflict(_))));
    let foreign = PrincipalRef { principal_id: "foreign".into(), ..owner() };
    assert!(store.preflight_context_clear(&foreign, &session.agent_session_id, &request).await.is_err());
}

#[tokio::test]
async fn context_clear_rejects_generic_append_and_cross_session_key_reuse() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (first, ready) = create_ready(&store, "clear-generic").await;
    let raw = append(&first.agent_session_id, "raw-clear", "session_api", "raw-clear", "context/cleared",
        first.agent_session_id.as_ref(), Some(ready), json!({"reason":"user_requested"}));
    assert!(matches!(store.append_event(&raw).await, Err(SessionStoreError::ExecutionFenced)));
    clear_context(&store, &first.agent_session_id, "shared-clear-key").await;
    let (second, _) = create_ready(&store, "clear-key-other").await;
    assert!(matches!(store.preflight_context_clear(&owner(), &second.agent_session_id, &command("shared-clear-key")).await,
        Err(SessionStoreError::IdempotencyConflict(_))));
}

#[tokio::test]
async fn context_clear_rejects_running_paused_and_reconciliation_without_append() {
    for state in ["running", "paused", "reconciliation"] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let (session, _) = create_turn(&store, state, "clear-owned-turn").await;
        if state == "paused" {
            let lease = store.claim_native_execution(NativeExecutionClaim {
                owner: owner(), agent_session_id: session.agent_session_id.clone(), operation_id: "clear-owned-turn".into(),
                snapshot: snapshot_ref(), active_set_generation: 0, holder: "clear-test".into(), expected_fence: 0, checkpoint: None,
            }).await.unwrap();
            store.pause_native_execution(&lease, "EXECUTION_USER_REQUESTED", true).await.unwrap();
        } else if state == "reconciliation" {
            // Fault-inject the exact recovery disposition; the canonical Turn
            // remains active, so neither this head nor a damaged ready head
            // may grant context mutation authority.
            sqlx::query("UPDATE agent_session_heads SET status='reconciliation' WHERE session_id=?")
                .bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
        }
        assert_eq!(store.head(&session.agent_session_id).await.unwrap().status, state);
        let before = store.current_cursor(&session.agent_session_id).await.unwrap();
        assert!(store.preflight_context_clear(&owner(), &session.agent_session_id, &command("blocked")).await.is_err());
        sqlx::query("UPDATE agent_session_heads SET status='ready',active_turn_id=NULL WHERE session_id=?")
            .bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
        assert!(matches!(store.preflight_context_clear(&owner(), &session.agent_session_id, &command("blocked")).await,
            Err(SessionStoreError::RecoveryRequiresReconciliation)));
        assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(), before);
    }
}

#[tokio::test]
async fn context_clear_rejects_unsettled_effects_even_with_a_terminal_turn() {
    for state in ["pending", "unknown", "cancelled"] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let (session, effect, started) = create_pending_effect(&store, state, EffectStrategy::ExternalUncertainEffect).await;
        if state != "pending" {
            store.record_effect_terminal(EffectEventRequest {
                event_id: format!("effect-terminal-{state}").into(),
                producer_id: "runtime_supervisor".into(), causation_event_id: Some(started.clone()),
                recorded_at: effect.recorded_at + 1, ..effect.clone()
            }, EffectTerminalState::Uncertain).await.unwrap();
            if state == "cancelled" {
                // Cancellation without proven external settlement must retain
                // the same blocker as the original unknown receipt.
                sqlx::query("UPDATE agent_effects SET state='cancelled' WHERE effect_id=?")
                    .bind(&effect.effect_id).execute(store.test_pool()).await.unwrap();
            }
        }
        let terminal = append(&session.agent_session_id, &format!("terminal-{state}"), "runtime_supervisor", &format!("terminal-{state}"),
            "turn/failed", effect.turn_id.as_ref(), Some(started), json!({"message":"execution stopped"}));
        store.append_event(&terminal).await.unwrap();
        assert_eq!(store.head(&session.agent_session_id).await.unwrap().status, "ready");
        assert!(store.preflight_context_clear(&owner(), &session.agent_session_id, &command("unsettled-clear")).await.is_err());
    }
}

#[tokio::test]
async fn context_clear_rejects_remote_attempt_automation_and_retired_purpose() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let mut remote = live_session(session_id());
    remote.remote_binding_provenance = Some(RemoteBindingProvenance { remote_binding_id: RemoteBindingId::from("remote-clear"), binding_version: 1 });
    let remote_id = remote.agent_session_id.clone();
    store.create_session(create_request(remote, "remote-clear")).await.unwrap();
    assert!(store.preflight_context_clear(&owner(), &remote_id, &command("remote-clear-command")).await.is_err());
    for relation in ["attempt", "automation"] {
        let (session, _) = create_ready(&store, relation).await;
        sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES(?,?,?,?,?,0,1,1)")
            .bind(session.agent_session_id.as_ref()).bind(Uuid::now_v7().to_string()).bind(relation)
            .bind(Uuid::now_v7().to_string()).bind(Uuid::now_v7().to_string()).execute(store.test_pool()).await.unwrap();
        assert!(store.preflight_context_clear(&owner(), &session.agent_session_id, &command("execution-clear")).await.is_err());
    }
    let (retired, _) = create_ready(&store, "retired-clear").await;
    sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.metadata.purpose','plugin_authoring') WHERE session_id=? AND seq=1")
        .bind(retired.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.preflight_context_clear(&owner(), &retired.agent_session_id, &command("retired-clear-command")).await.is_err());
}

#[tokio::test]
async fn context_clear_commit_rechecks_new_turn_binding_and_context_floor() {
    for change in ["input", "binding", "clear"] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let (session, _) = create_ready(&store, change).await;
        let request = command("stale-clear");
        let boundary = prepare(&store, &session.agent_session_id, &request).await;
        match change {
            "input" => {
                let (_, turn) = store.start_turn(&session.agent_session_id, "session_api".into(), "racing-input".into(), "racing-turn".into(),
                    StrictJsonValue(json!({"content":"must survive"}))).await.unwrap();
                store.append_event(&append(&session.agent_session_id, "racing-completed", "runtime_supervisor", "racing-completed",
                    "turn/completed", "racing-turn", Some(turn.ack.unwrap().event_id), json!({}))).await.unwrap();
            }
            "binding" => {
                let expected = session.agent_binding.clone();
                store.replace_session_agent_binding(&owner(), &session.agent_session_id,
                    agent_switch(expected.clone(), agent_replacement(&expected, "clear-race"), "clear-race")).await.unwrap();
            }
            "clear" => { clear_context(&store, &session.agent_session_id, "racing-clear").await; }
            _ => unreachable!(),
        }
        let before = store.current_cursor(&session.agent_session_id).await.unwrap();
        assert!(store.validate_context_clear_boundary(&owner(), &session.agent_session_id, &request, &boundary).await.is_err());
        assert!(store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.is_err());
        assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(), before);
    }
}

#[tokio::test]
async fn context_clear_rechecks_effects_after_preflight() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, effect, started) = create_pending_effect(&store, "clear-effect-race", EffectStrategy::ExternalUncertainEffect).await;
    store.record_effect_terminal(EffectEventRequest {
        event_id: "effect-race-returned".into(),
        producer_id: "runtime_supervisor".into(), causation_event_id: Some(started.clone()),
        recorded_at: effect.recorded_at + 1, ..effect.clone()
    }, EffectTerminalState::Succeeded).await.unwrap();
    let terminal = append(&session.agent_session_id, "effect-race-terminal", "runtime_supervisor", "effect-race-terminal",
        "turn/failed", effect.turn_id.as_ref(), Some(started), json!({"message":"stopped"}));
    store.append_event(&terminal).await.unwrap();
    let request = command("effect-race-clear");
    let boundary = prepare(&store, &session.agent_session_id, &request).await;
    // Inject a late owner uncertainty into this isolated database to verify
    // that commit repeats settlement checks rather than trusting preflight.
    sqlx::query("UPDATE agent_effects SET state='unknown' WHERE effect_id=?")
        .bind(effect.effect_id).execute(store.test_pool()).await.unwrap();
    assert!(store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.is_err());
}

#[tokio::test]
async fn context_clear_waits_for_knowledge_writeback_settlement_without_losing_its_boundary() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "clear-writeback").await;
    let operation = OperationId::from("clear-writeback-turn");
    let (_, started) = store.start_turn(&session.agent_session_id, "session_api".into(), "clear-writeback-input".into(), operation.clone(),
        StrictJsonValue(json!({"content":"remember the result", "admission":{"resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();
    store.append_event(&append(&session.agent_session_id, "clear-writeback-terminal", "runtime_supervisor", "clear-writeback-terminal",
        "turn/completed", operation.as_ref(), Some(started.ack.unwrap().event_id), json!({}))).await.unwrap();
    let writeback = store.start_knowledge_writeback(crate::KnowledgeWritebackStartRequest {
        owner: owner(), agent_session_id: session.agent_session_id.clone(), operation_id: operation.clone(),
        resolved_snapshot_ref: session.agent_binding.resolved_snapshot_ref.clone(), binding_version: session.agent_binding.binding_version,
        execution_instance_id: "clear-test-instance".into(),
    }).await.unwrap();
    let request = command("clear-after-writeback");
    let boundary = prepare(&store, &session.agent_session_id, &request).await;
    assert!(store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.is_err());
    store.settle_knowledge_writeback(&owner(), &session.agent_session_id, &writeback.start_event_id,
        nomifun_agent_contracts::KnowledgeWritebackSettledV1 { operation_id: operation,
            status: nomifun_agent_contracts::KnowledgeWritebackNoticeStatus::Cancelled,
            written: vec![], detail: None, diagnostic: None }).await.unwrap();
    assert!(!store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.unwrap().duplicate);
}

#[tokio::test]
async fn context_clear_requires_native_cleanup_proof_only_for_the_exact_session() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    for settled in [false, true] {
        let key = if settled { "cleanup-proven" } else { "cleanup-pending" };
        let (session, started) = create_turn(&store, key, key).await;
        store.append_event(&append(&session.agent_session_id, &format!("dispatch-{key}"), "runtime_supervisor", &format!("dispatch-{key}"),
            "runtime/progress-recorded", key, Some(started.clone()), json!({"event":{"event":"host_tool_dispatch","operation_id":"native-tool"}}))).await.unwrap();
        if settled {
            store.append_event(&append(&session.agent_session_id, &format!("settled-{key}"), "runtime_supervisor", &format!("settled-{key}"),
                "runtime/progress-recorded", key, Some(started.clone()), json!({"event":{"event":"host_tool_settled","operation_id":"native-tool"}}))).await.unwrap();
        }
        store.append_event(&append(&session.agent_session_id, &format!("terminal-{key}"), "runtime_supervisor", &format!("terminal-{key}"),
            "turn/completed", key, Some(started), json!({}))).await.unwrap();
        let request = command(&format!("clear-{key}"));
        let boundary = prepare(&store, &session.agent_session_id, &request).await;
        assert_eq!(store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.is_ok(), settled);
    }
}

#[tokio::test]
async fn context_clear_same_key_concurrent_commits_have_one_fact_and_one_replay() {
    let store = AgentSessionStore::open_in_memory_with_connections(4).await.unwrap();
    let (session, _) = create_ready(&store, "clear-concurrent").await;
    let request = command("clear-concurrent-command");
    let boundary = prepare(&store, &session.agent_session_id, &request).await;
    let expected_owner = owner();
    let (first, second) = tokio::join!(
        store.clear_context(&expected_owner, &session.agent_session_id, &request, &boundary),
        store.clear_context(&expected_owner, &session.agent_session_id, &request, &boundary),
    );
    let first = first.unwrap(); let second = second.unwrap();
    assert_ne!(first.duplicate, second.duplicate);
    assert_eq!(first.ack, second.ack);
    assert_eq!(store.read_events(&session.agent_session_id, None, 500).await.unwrap().events.iter()
        .filter(|event| event.kind.0 == "context/cleared").count(), 1);
}

#[tokio::test]
async fn context_clear_rechecks_the_active_lead_owner_and_allows_terminal_or_inactive_leads() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let expected_owner = PrincipalRef { principal_id: Uuid::now_v7().to_string(), ..owner() };
    let mut session = live_session(session_id()); session.owner_ref = expected_owner.clone();
    let created = store.create_session(create_request(session, "clear-lead-owner")).await.unwrap();
    let session = created.session.agent_session_id;
    store.append_event(&append(&session, "clear-lead-ready", "runtime_supervisor", "clear-lead-ready", "session/ready",
        "session-clear-lead-owner", Some(created.opening_ack.event_id), json!({}))).await.unwrap();
    let execution = Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,created_at,updated_at) VALUES(?,?,'lead context fixture','completed','fixed','automatic','automatic','{}',1,1)")
        .bind(&execution).bind(&expected_owner.principal_id).execute(store.test_pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,active,created_at,updated_at) VALUES(?,?,'lead',1,1,1)")
        .bind(session.as_ref()).bind(&execution).execute(store.test_pool()).await.unwrap();
    let request = command("clear-lead-admission-race");
    let ContextClearPreparation::Ready(boundary) = store.preflight_context_clear(&expected_owner, &session, &request).await.unwrap()
        else { panic!("a terminal lead must permit clear preparation"); };
    for status in ["planning", "awaiting_approval", "running", "paused", "waiting_input"] {
        sqlx::query("UPDATE agent_executions SET status=? WHERE execution_id=?")
            .bind(status).bind(&execution).execute(store.test_pool()).await.unwrap();
        let before = store.current_cursor(&session).await.unwrap();
        assert!(store.preflight_context_clear(&expected_owner, &session, &request).await.is_err());
        assert!(store.validate_context_clear_boundary(&expected_owner, &session, &request, &boundary).await.is_err());
        assert!(store.clear_context(&expected_owner, &session, &request, &boundary).await.is_err());
        assert_eq!(store.current_cursor(&session).await.unwrap(), before);
    }
    sqlx::query("UPDATE conversation_execution_links SET active=0 WHERE conversation_id=?")
        .bind(session.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(!store.clear_context(&expected_owner, &session, &request, &boundary).await.unwrap().duplicate);
    sqlx::query("UPDATE conversation_execution_links SET active=1 WHERE conversation_id=?")
        .bind(session.as_ref()).execute(store.test_pool()).await.unwrap();
    let request = command("clear-terminal-lead");
    for status in ["completed", "completed_with_failures", "failed", "cancelled"] {
        sqlx::query("UPDATE agent_executions SET status=? WHERE execution_id=?")
            .bind(status).bind(&execution).execute(store.test_pool()).await.unwrap();
        assert!(matches!(store.preflight_context_clear(&expected_owner, &session, &request).await.unwrap(),
            ContextClearPreparation::Ready(_)));
    }
    sqlx::query("UPDATE agent_executions SET status='running',deleted_at=1 WHERE execution_id=?")
        .bind(&execution).execute(store.test_pool()).await.unwrap();
    assert!(matches!(store.preflight_context_clear(&expected_owner, &session, &request).await.unwrap(), ContextClearPreparation::Ready(_)));
    sqlx::query("UPDATE agent_executions SET deleted_at=NULL,user_id=? WHERE execution_id=?")
        .bind(Uuid::now_v7().to_string()).bind(&execution).execute(store.test_pool()).await.unwrap();
    assert!(matches!(store.preflight_context_clear(&expected_owner, &session, &request).await.unwrap(), ContextClearPreparation::Ready(_)));
}

#[tokio::test]
async fn context_clear_continuation_requires_the_original_input_binding_and_active_set() {
    for change in ["input", "clear", "agent", "model", "resource", "active_set"] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let (session, _) = create_ready(&store, change).await;
        let request = command("continuation-clear");
        let boundary = prepare(&store, &session.agent_session_id, &request).await;
        let cleared = store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.unwrap();
        store.validate_context_clear_continuation(&owner(), &session.agent_session_id, &request).await.unwrap();
        let SessionEventPayloadRef::InlineJson(payload) = &cleared.record.as_ref().unwrap().payload else { panic!("clear proof must be inline"); };
        assert_eq!(payload.0["admission"]["binding_ref"], serde_json::to_value(AgentHandoffBindingRefV1::from(&session.agent_binding)).unwrap());
        assert_eq!(payload.0["admission"]["active_set_generation"], 0);
        match change {
            "input" => {
                let (_, turn) = store.start_turn(&session.agent_session_id, "session_api".into(), "continuation-successor-input".into(),
                    "continuation-successor-turn".into(), StrictJsonValue(json!({"content":"new request must survive"}))).await.unwrap();
                store.append_event(&append(&session.agent_session_id, "continuation-successor-terminal", "runtime_supervisor", "continuation-successor-terminal",
                    "turn/completed", "continuation-successor-turn", Some(turn.ack.unwrap().event_id), json!({}))).await.unwrap();
            }
            "clear" => { clear_context(&store, &session.agent_session_id, "continuation-newer-clear").await; }
            "agent" => {
                let expected = session.agent_binding.clone();
                store.replace_session_agent_binding(&owner(), &session.agent_session_id,
                    agent_switch(expected.clone(), agent_replacement(&expected, "continuation-agent"), "continuation-agent")).await.unwrap();
            }
            "model" => {
                let expected = session.agent_binding.clone();
                store.replace_session_model_binding(&owner(), &session.agent_session_id, &expected,
                    agent_replacement(&expected, "continuation-model")).await.unwrap();
            }
            "resource" => {
                let expected = session.agent_binding.clone(); let mut replacement = expected.clone();
                replacement.binding_version += 1;
                replacement.typed_resource_bindings = vec![TypedResourceBinding {
                    binding_id: "continuation-resource".into(), resource_kind: "workspace".into(), resource_id: "continuation-workspace".into(),
                    owner_id: owner().principal_id, operations: BTreeSet::from(["read".into()]), connection_config_ref: None,
                    typed_parameters: BTreeMap::new(),
                }];
                store.replace_session_resource_bindings(&owner(), &session.agent_session_id, &expected, replacement, "workspace").await.unwrap();
            }
            "active_set" => {
                let ids = vec!["coding.workspace"];
                let opening = store.read_events(&session.agent_session_id, None, 500).await.unwrap().events[0].event_id.clone();
                store.append_event(&append(&session.agent_session_id, "continuation-active-set", "capability_host", "continuation-active-set",
                    "capability/active-set-committed", session.agent_session_id.as_ref(), Some(opening),
                    json!({"generation":1,"active_capability_ids":ids,"active_set_digest":nomifun_agent_contracts::digest_payload(&ids).unwrap(),"delta":[]}))).await.unwrap();
            }
            _ => unreachable!(),
        }
        let head = store.head(&session.agent_session_id).await.unwrap();
        assert_eq!(head.status, "ready");
        assert!(store.validate_context_clear_continuation(&owner(), &session.agent_session_id, &request).await.is_err(), "{change}");
        let replay = store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.unwrap();
        assert!(replay.duplicate); assert_eq!(replay.ack, cleared.ack);
        assert_eq!(store.head(&session.agent_session_id).await.unwrap(), head);
    }
}

#[tokio::test]
async fn context_clear_continuation_rejects_unproven_and_partial_payloads_but_retains_historical_command_replay() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "continuation-proof").await;
    let request = command("continuation-proof-clear");
    assert!(store.validate_context_clear_continuation(&owner(), &session.agent_session_id, &request).await.is_err());
    let boundary = prepare(&store, &session.agent_session_id, &request).await;
    let clear = store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.unwrap();
    let clear_id = clear.record.as_ref().unwrap().event_id.as_ref();
    let foreign = PrincipalRef { principal_id:"foreign".into(), ..owner() };
    assert!(store.validate_context_clear_continuation(&foreign, &session.agent_session_id, &request).await.is_err());
    // A current-generation reason-only fact retains its original command
    // receipt, but cannot acquire the new missing-input continuation authority.
    sqlx::query("UPDATE agent_events SET inline_json=? WHERE event_id=?")
        .bind(json!({"reason":"user_requested"}).to_string()).bind(clear_id).execute(store.test_pool()).await.unwrap();
    assert!(store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await.unwrap().duplicate);
    assert!(matches!(store.validate_context_clear_continuation(&owner(), &session.agent_session_id, &request).await,
        Err(SessionStoreError::Conflict(message)) if message.contains("no exact binding proof")));
    sqlx::query("UPDATE agent_events SET inline_json=? WHERE event_id=?")
        .bind(json!({"reason":"user_requested","admission":{"binding_ref":AgentHandoffBindingRefV1::from(&session.agent_binding)}}).to_string())
        .bind(clear_id).execute(store.test_pool()).await.unwrap();
    assert!(matches!(store.clear_context(&owner(), &session.agent_session_id, &request, &boundary).await, Err(SessionStoreError::InvalidPayload(_))));
    assert!(store.validate_context_clear_continuation(&owner(), &session.agent_session_id, &request).await.is_err());
}
