//! Exact SDK/Kernel retirement with durable MCP uncertainty and a real writer.
//! MCP outcome facts are supplied by the canonical receipt fixture, not a live
//! remote MCP server; the App, Runtime teardown and Bun process owner are real.
use super::*;
use nomifun_agent_execution::AgentExecutionSessionPort;

#[tokio::test]
async fn duplicate_retirement_joins_proven_original_after_kernel_eviction_and_releases_its_handle() {
    let server = WaitingWriterServer::with_start_hold(true).await;
    let fixture = Fixture::with_process("duplicate-retirement-known-native-cleanup", &server.url).await;
    let owner = fixture.host.options.user_id.clone();
    let setup = fixture.host.options.conversation_id.clone();
    fixture.owner.cancel_turn_for_execution(&owner, &setup, "retirement-setup-stop", "cleanup-retry-turn").await.unwrap();
    fixture.services.agent_runtime_sessions.retire_runtime_if_matches(&setup, &fixture.runtime).await.unwrap();
    let store = fixture.owner.canonical().store();
    let binding = store.get_live_session(&setup.clone().into()).await.unwrap().agent_binding;
    let selected = fixture.host.options.model.as_ref().unwrap();
    let session = AgentSessionId::from(api(&fixture._router, "/api/agent-sessions", json!({
        "preset_id":binding.preset_revision_ref.preset_id,
        "model":{"provider_id":selected.provider_id,"model":selected.use_model.as_deref().unwrap_or(&selected.model)},
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}]
    })).await["agent_session_id"].as_str().unwrap());
    let projection = fixture.owner.get_session(&owner, session.as_ref()).await.unwrap();
    let workspace = PathBuf::from(projection.extra["workspace"].as_str().unwrap());
    std::fs::create_dir_all(&workspace).unwrap();
    let script = workspace.join("waiting-writer.mjs");
    let output = workspace.join("original-writer.log");
    std::fs::write(&script, "import {appendFileSync} from 'node:fs';const target=process.argv[2];appendFileSync(target,'ready\\n');setInterval(()=>appendFileSync(target,'tick\\n'),25);\n").unwrap();
    server.state.launch.set(json!({"command":fixture_bun(),"args":[script,output],
        "execution_identity":"current_user","tty":false,"wait_ms":0})).unwrap();
    let operation = OperationId::from(api(&fixture._router, &format!("/api/agent-sessions/{}/turns", session.as_ref()),
        json!({"idempotency_key":"original-writer-turn","input":{"content":"Read waiting-writer.mjs, start its original background writer, then wait."}}))
        .await["operation_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(15), server.state.started_entered.notified()).await.unwrap();
    let runtime = fixture.services.agent_runtime_sessions.get_runtime(session.as_ref()).unwrap();
    let runtime_weak = match &runtime {
        nomifun_ai_agent::AgentRuntimeHandle::Official(runtime) => Arc::downgrade(runtime),
        _ => panic!("the installed official factory must supply the original Runtime"),
    };
    let original_host = HOSTS.get().unwrap().lock().unwrap().get(session.as_ref()).unwrap().upgrade().unwrap();
    assert!(!runtime.teardown_proven());
    let process_owner = original_host.session_host.clone();
    let process_before = process_owner.process_resource_instances(&owner, &session).unwrap();
    assert_eq!(process_before.len(), 1);
    let native = &process_before[0];
    assert_eq!(&native.process_id, server.state.process.get().unwrap());
    let first_bytes = writer_grows(&output, 0).await;

    // Supply the original MCP owner's formal failure/cleanup outcome through
    // its real durable receipt path, without replaying or reconciling a result.
    let tool_operation = format!("{}:tool:original-uncertain-mcp", operation.as_ref());
    let journal = original_host.active.lock().await.as_ref().unwrap().journal.clone();
    journal.append(json!({"event":"host_tool_dispatch","dispatch":{
        "operation_id":tool_operation,"call_id":"original-uncertain-mcp",
        "capability_id":"mcp.server","action_id":"mcp.resource/read","model_name":"read_mcp_resource"
    }}).to_string(), None, crate::router::engine_journal::EngineJournalWrite::Progress).await.unwrap();
    let receipts = crate::router::mcp_effect_receipts::McpEffectReceipts::new(fixture.pool().clone());
    let receipt = receipts.begin(&owner, session.as_ref(), &tool_operation, "mcp.server", &operation,
        &original_host.snapshot_ref).await.unwrap();
    receipts.settle(receipt, &nomifun_mcp::McpInvocationOutcome::<StrictJsonValue> {
        result:Err(nomifun_mcp::McpOwnerError::new("MCP_REQUEST_CANCELLED", "Original external outcome remains unknown")),
        execution:nomifun_mcp::McpExecutionFacts {request_dispatched:true,local_process_started:true,cancellation_requested:true},
        cleanup:nomifun_mcp::McpCleanupFacts {local:nomifun_mcp::McpCleanupStatus::Confirmed,
            remote:nomifun_mcp::McpCleanupStatus::NotRequired,error:None},
    }).await.unwrap();
    journal.append(json!({"event":"host_tool_settled","operation_id":tool_operation,
        "call_id":"original-uncertain-mcp","result":null,"error":"Original external outcome remains unknown"}).to_string(),
        None, crate::router::engine_journal::EngineJournalWrite::Settlement).await.unwrap();
    let effects = store.list_effects(&session).await.unwrap();
    let uncertain = effects.iter().find(|effect| effect.owner_domain == "mcp").unwrap();
    assert_eq!(uncertain.state, nomifun_agent_session::AgentEffectState::Unknown);
    assert_eq!(uncertain.bounded_observation.as_ref().unwrap()["cleanup"],
        json!({"local":"confirmed","remote":"not_required","error":null}));
    fixture.owner.cancel_turn_for_execution(&owner, session.as_ref(), "stop-original-writer-turn", operation.as_ref()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let carrier = fixture.services.agent_runtime_sessions.observe_runtime_carrier(session.as_ref());
            if !carrier.registered && !carrier.teardown_pending && runtime.teardown_proven() { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("the original full SDK teardown must prove release before this duplicate retirement");
    assert!(process_owner.ensure_completed_effect_resources_closed(&session).await.is_err(),
        "proven teardown must already have evicted the original Kernel weak-cache entry");
    let cursor = store.current_cursor(&session).await.unwrap();
    let calls = server.state.requests.lock().unwrap().len();
    let (first, second) = tokio::join!(
        fixture.owner.retire_completed_runtime(&owner, &session, &operation, &runtime),
        fixture.owner.retire_completed_runtime(&owner, &session, &operation, &runtime),
    );
    first.unwrap(); second.unwrap();
    fixture.owner.schedule_completed_runtime_retirement(owner.clone(), session.clone(), operation.clone(), runtime.clone());
    fixture.owner.schedule_completed_runtime_retirement(owner.clone(), session.clone(), operation.clone(), runtime.clone());
    drop(journal);
    drop(original_host);
    drop(runtime);
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime_weak.strong_count() != 0 { tokio::task::yield_now().await; }
    }).await.expect("duplicate retirement workers must release the exact retired Runtime handle");
    let after = process_owner.process_resource_instances(&owner, &session).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].process_id, native.process_id);
    assert_eq!(after[0].owner_generation, native.owner_generation);
    assert_eq!(after[0].creator_turn_id, native.creator_turn_id);
    assert_eq!(after[0].creator_call_id, native.creator_call_id);
    assert!(!after[0].execution_stopped && !after[0].stop_requested);
    assert!(writer_grows(&output, first_bytes + 32).await > first_bytes);
    assert_eq!(server.state.requests.lock().unwrap().len(), calls);
    assert_eq!(store.list_effects(&session).await.unwrap(), effects);
    assert_eq!(store.current_cursor(&session).await.unwrap(), cursor,
        "resource proof cannot mutate/reconcile Unknown or create another Turn");
    assert!(crate::router::hosted_effect_receipts::HostedEffectReceipts::new(fixture.pool().clone())
        .ensure_settled(&owner, session.as_ref()).await.is_err(), "business admission still rejects Unknown");
    process_owner.cleanup_session_processes(&owner, &session).await.unwrap();
    server.state.started_release.notify_one();
    fixture.finish().await;
    server.close().await;
}
