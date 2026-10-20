use super::*;
use crate::{CommittedToolCallCausation, NativeExecutionClaim};

struct ClosedToolFixture {
    session: AgentSessionLiveRecord,
    turn: OperationId,
    operation: OperationId,
    root: SessionEventRecord,
    start: SessionEventRecord,
    tool: SessionEventRecord,
    terminal: SessionEventRecord,
}

async fn closed_tool_fixture(
    store: &AgentSessionStore, key: &str, stored: bool, duplicate: bool,
) -> ClosedToolFixture {
    let (session, _) = create_ready(store, key).await;
    let sid = &session.agent_session_id;
    let turn = OperationId::from(format!("committed-parent:{key}"));
    let operation = OperationId::from(format!("committed-operation:{key}"));
    let route = ChatRouteIdentity::new("coding.codex@1", nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT, "chat-route".into(), 4);
    let (root, start) = store.start_turn(sid, "session_api".into(), key.into(), turn.clone(),
        StrictJsonValue(json!({"content":"read the workspace", "admission":{
            "route_identity":route, "resolved_snapshot_ref":snapshot_ref(),
        }}))).await.unwrap();
    let root = root.record.unwrap();
    let start = start.record.unwrap();
    let lease = store.claim_native_execution(NativeExecutionClaim {
        owner:owner(), agent_session_id:sid.clone(), operation_id:turn.clone(), snapshot:snapshot_ref(),
        active_set_generation:0, holder:format!("holder-{key}"), expected_fence:0, checkpoint:None,
    }).await.unwrap();
    let call_id = format!("call-{key}");
    let value = json!({"operation_id":operation, "call_id":call_id,
        "capability_id":"workspace.files", "action_id":"workspace.files/read"});
    let mut call = append(sid, &format!("tool-{key}"), "runtime_supervisor", &format!("tool-{key}"),
        "tool/call-started", &call_id, Some(start.event_id.clone()), value.clone());
    let payload = stored.then(|| {
        let bytes = canonical_json_bytes(&value).unwrap();
        let payload = SessionPayloadRecord {
            payload_id:format!("payload-{key}").into(), agent_session_id:sid.clone(), media_type:"application/json".into(),
            byte_len:bytes.len() as u64, digest:digest_bytes(&bytes), body:SessionPayloadBody::Json(StrictJsonValue(value)),
        };
        call.semantic_event.payload = SessionEventPayloadRef::Stored(payload.payload_id.clone());
        payload
    });
    let tool = store.append_native_event(&lease, &call, payload.as_ref()).await.unwrap().record.unwrap();
    if !stored {
        assert_eq!(store.native_tool_call_causation(&owner(), sid, &turn, &operation,
            &"workspace.files".into(), &"workspace.files/read".into()).await.unwrap().tool_call.event_id, tool.event_id,
            "the historical fixture begins with a real admitted native call");
    }
    if duplicate {
        let second = append(sid, &format!("duplicate-tool-{key}"), "runtime_supervisor", &format!("duplicate-tool-{key}"),
            "tool/call-started", &format!("duplicate-call-{key}"), Some(start.event_id.clone()),
            json!({"operation_id":operation,"call_id":format!("duplicate-call-{key}"),
                "capability_id":"workspace.files","action_id":"workspace.files/read"}));
        store.append_native_event(&lease, &second, None).await.unwrap();
    }
    let result = append(sid, &format!("result-{key}"), "runtime_supervisor", &format!("result-{key}"),
        "tool/result-recorded", &call_id, Some(tool.event_id.clone()),
        json!({"operation_id":operation,"call_id":call_id,"output":{"observed":true},"error":null}));
    store.append_native_observation(&lease, &result, None).await.unwrap();
    let part = append(sid, &format!("part-{key}"), "runtime_supervisor", &format!("part-{key}"),
        "message/content-part", &format!("assistant-{key}"), Some(start.event_id.clone()),
        json!({"content":"done","turn_id":root.event_id}));
    store.append_native_observation(&lease, &part, None).await.unwrap();
    let message = append(sid, &format!("message-{key}"), "runtime_supervisor", &format!("message-{key}"),
        "message/completed", &format!("assistant-{key}"), Some(part.event_id),
        json!({"part_count":1,"content_digest":digest_bytes(b"done")}));
    let terminal = append(sid, &format!("terminal-{key}"), "runtime_supervisor", &format!("terminal-{key}"),
        "turn/completed", turn.as_ref(), Some(message.event_id.clone()), json!({"model_steps":1}));
    let (_, terminal) = store.append_native_chat_completion(&lease, &message, &terminal).await.unwrap();
    ClosedToolFixture {session,turn,operation,root,start,tool,terminal:terminal.record.unwrap()}
}

async fn committed_cause(
    store: &AgentSessionStore, f: &ClosedToolFixture,
) -> Result<CommittedToolCallCausation, SessionStoreError> {
    store.committed_tool_call_causation(&f.session.agent_session_id, &f.turn, &f.operation,
        &"workspace.files".into(), &"workspace.files/read".into()).await
}

#[tokio::test]
async fn historical_committed_tool_causation_reads_closed_parent_without_granting_active_or_native_authority() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = closed_tool_fixture(&store, "closed-parent", false, false).await;
    let sid = &f.session.agent_session_id;
    let before = store.current_cursor(sid).await.unwrap();
    assert_eq!(committed_cause(&store, &f).await.unwrap(),
        CommittedToolCallCausation {event_id:f.tool.event_id.clone(),seq:f.tool.seq});
    assert!(store.tool_call_causation(sid, &f.turn, &f.operation, &"workspace.files".into(), &"workspace.files/read".into()).await.is_err());
    assert!(matches!(store.native_tool_call_causation(&owner(), sid, &f.turn, &f.operation,
        &"workspace.files".into(), &"workspace.files/read".into()).await, Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(store.current_cursor(sid).await.unwrap(), before, "historical reads cannot author facts");
    let successor = OperationId::from("committed-successor");
    store.start_turn(sid, "session_api".into(), "committed-successor".into(), successor.clone(),
        StrictJsonValue(json!({"content":"new independent work"}))).await.unwrap();
    assert!(store.committed_tool_call_causation(sid, &successor, &f.operation,
        &"workspace.files".into(), &"workspace.files/read".into()).await.is_err(), "a successor cannot borrow the parent's call");
    assert_eq!(committed_cause(&store, &f).await.unwrap().event_id, f.tool.event_id);
}

#[tokio::test]
async fn historical_committed_tool_causation_rejects_wrong_invocation_and_ambiguity() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = closed_tool_fixture(&store, "wrong-invocation", false, false).await;
    let (foreign, _) = create_ready(&store, "foreign-committed-tool").await;
    for (session, turn, operation, capability, action) in [
        (&foreign.agent_session_id, f.turn.as_ref(), f.operation.as_ref(), "workspace.files", "workspace.files/read"),
        (&f.session.agent_session_id, "wrong-turn", f.operation.as_ref(), "workspace.files", "workspace.files/read"),
        (&f.session.agent_session_id, f.turn.as_ref(), "wrong-operation", "workspace.files", "workspace.files/read"),
        (&f.session.agent_session_id, f.turn.as_ref(), f.operation.as_ref(), "browser", "workspace.files/read"),
        (&f.session.agent_session_id, f.turn.as_ref(), f.operation.as_ref(), "workspace.files", "workspace.files/write"),
    ] {
        assert!(store.committed_tool_call_causation(session, &turn.into(), &operation.into(),
            &capability.into(), &action.into()).await.is_err(), "accepted a different invocation");
    }
    let ambiguous = closed_tool_fixture(&store, "ambiguous-committed", false, true).await;
    assert!(committed_cause(&store, &ambiguous).await.is_err(), "a repeated operation cannot select an arbitrary call");
}

#[tokio::test]
async fn historical_committed_tool_causation_rejects_damaged_root_start_call_and_terminal_boundary() {
    for defect in ["root-kind", "start-operation", "start-source", "tool-kind", "tool-version", "tool-producer",
        "tool-operation", "tool-capability", "tool-action", "tool-call-id", "tool-cause", "tool-before-start", "tool-after-terminal",
        "terminal-kind", "terminal-version", "terminal-correlation", "terminal-session"] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let f = closed_tool_fixture(&store, "damaged-committed", false, false).await;
        if defect == "tool-cause" {
            sqlx::query("UPDATE agent_events SET causation_event_id=? WHERE event_id=?")
                .bind(f.root.event_id.as_ref()).bind(f.tool.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
        } else if defect == "tool-before-start" {
            sqlx::query("UPDATE agent_events SET seq=seq+1000 WHERE session_id=? AND seq>=?")
                .bind(f.session.agent_session_id.as_ref()).bind(f.start.seq as i64).execute(store.test_pool()).await.unwrap();
            sqlx::query("UPDATE agent_events SET seq=500 WHERE event_id=?")
                .bind(f.tool.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
        } else if defect == "terminal-session" {
            let (foreign, _) = create_ready(&store, "foreign-terminal").await;
            sqlx::query("UPDATE agent_events SET session_id=? WHERE event_id=?")
                .bind(foreign.agent_session_id.as_ref()).bind(f.terminal.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
        } else {
            let (sql, event) = match defect {
                "root-kind" => ("UPDATE agent_events SET kind='session/ready' WHERE event_id=?", &f.root),
                "start-operation" => ("UPDATE agent_events SET inline_json=json_set(inline_json,'$.operation_id','foreign-turn') WHERE event_id=?", &f.start),
                "start-source" => ("UPDATE agent_events SET inline_json=json_set(inline_json,'$.source_message_id','foreign-root') WHERE event_id=?", &f.start),
                "tool-kind" => ("UPDATE agent_events SET kind='session/ready' WHERE event_id=?", &f.tool),
                "tool-version" => ("UPDATE agent_events SET kind_version=2 WHERE event_id=?", &f.tool),
                "tool-producer" => ("UPDATE agent_events SET producer_id='session_api' WHERE event_id=?", &f.tool),
                "tool-operation" => ("UPDATE agent_events SET inline_json=json_set(inline_json,'$.operation_id','foreign-operation') WHERE event_id=?", &f.tool),
                "tool-capability" => ("UPDATE agent_events SET inline_json=json_set(inline_json,'$.capability_id','browser') WHERE event_id=?", &f.tool),
                "tool-action" => ("UPDATE agent_events SET inline_json=json_set(inline_json,'$.action_id','workspace.files/write') WHERE event_id=?", &f.tool),
                "tool-call-id" => ("UPDATE agent_events SET inline_json=json_remove(inline_json,'$.call_id') WHERE event_id=?", &f.tool),
                "tool-after-terminal" => ("UPDATE agent_events SET seq=seq+1000 WHERE event_id=?", &f.tool),
                "terminal-kind" => ("UPDATE agent_events SET kind='session/ready' WHERE event_id=?", &f.terminal),
                "terminal-version" => ("UPDATE agent_events SET kind_version=2 WHERE event_id=?", &f.terminal),
                "terminal-correlation" => ("UPDATE agent_events SET correlation_id='foreign-turn' WHERE event_id=?", &f.terminal),
                _ => unreachable!(),
            };
            sqlx::query(sql).bind(event.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
        }
        assert!(committed_cause(&store, &f).await.is_err(), "accepted damaged {defect}");
    }
}

#[tokio::test]
async fn historical_committed_tool_causation_resolves_stored_call_and_checks_integrity_and_owner() {
    for defect in ["none", "length", "bytes", "owner"] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let f = closed_tool_fixture(&store, "stored-committed", true, false).await;
        let SessionEventPayloadRef::Stored(payload_id) = &f.tool.payload else {panic!("formal writer must persist a Stored call")};
        match defect {
            "none" => {
                assert_eq!(committed_cause(&store, &f).await.unwrap().event_id, f.tool.event_id);
                continue;
            }
            "length" => {sqlx::query("UPDATE agent_payloads SET byte_len=byte_len+1 WHERE payload_id=?")
                .bind(payload_id.as_ref()).execute(store.test_pool()).await.unwrap();}
            "bytes" => {
                let value = json!({"operation_id":f.operation,"call_id":"call-stored-committeX",
                    "capability_id":"workspace.files","action_id":"workspace.files/read"});
                let stored_length: i64 = sqlx::query_scalar("SELECT byte_len FROM agent_payloads WHERE payload_id=?")
                    .bind(payload_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
                assert_eq!(canonical_json_bytes(&value).unwrap().len() as i64, stored_length,
                    "same-length valid JSON damage must exercise the digest guard");
                let body = serde_json::to_vec(&SessionPayloadBody::Json(StrictJsonValue(value))).unwrap();
                sqlx::query("UPDATE agent_payloads SET body=? WHERE payload_id=?")
                    .bind(body).bind(payload_id.as_ref()).execute(store.test_pool()).await.unwrap();
            }
            "owner" => {
                let (foreign, _) = create_ready(&store, "stored-call-foreign").await;
                sqlx::query("UPDATE agent_payloads SET session_id=? WHERE payload_id=?")
                    .bind(foreign.agent_session_id.as_ref()).bind(payload_id.as_ref()).execute(store.test_pool()).await.unwrap();
            }
            _ => unreachable!(),
        }
        assert!(matches!(committed_cause(&store, &f).await, Err(SessionStoreError::InvalidPayload(_))),
            "stored call must reject {defect} damage through the canonical payload resolver");
    }
}
