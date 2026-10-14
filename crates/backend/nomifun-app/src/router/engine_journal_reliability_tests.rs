use super::*;
use nomifun_agent_runtime::{AgentExecutionCheckpoint, AgentExecutionSegmentState, AgentSegmentPolicy, EngineBinding};

fn checkpoint(journal:&EngineTurnJournal, renewed:bool) -> AgentExecutionCheckpoint {
    AgentExecutionCheckpoint { version:1,
        binding:EngineBinding::new(journal.0.session.clone(),"native-binding".into(),"test".into(),"a".repeat(64).into(),journal.0.snapshot.clone()).unwrap(),
        turn_operation_id:journal.0.operation.clone(),active_set_generation:0,model_steps:u16::from(renewed),tool_call_count:0,
        accepted_input_count:1,applied_steering_receipts:vec![],plan:Default::default(),work:Default::default(),patch_recovery:Default::default(),
        control_rejections:Default::default(),pending_user_input: None, completion_review:Default::default(),segments:Some(AgentExecutionSegmentState {
            policy:AgentSegmentPolicy { max_segments:2,max_no_progress_segments:2 },model_steps_per_segment:1,
            segment:if renewed {2} else {1},segment_start_step:u16::from(renewed),no_progress_segments:0,
            progress_at_segment_start:0,progress_fingerprints:vec![],
        }) }
}

fn owner(journal:&EngineTurnJournal) -> nomifun_agent_contracts::PrincipalRef {
    nomifun_agent_contracts::PrincipalRef { principal_kind:"user".into(),principal_id:journal.0.user.clone() }
}

fn rejected_arguments(step: u16) -> AgentEngineEvent {
    use nomifun_agent_runtime::{AgentModelResponseRejection, AgentRejectedToolProposal};
    use nomifun_chat_model_broker::{ToolArgumentDiagnostic, ToolArgumentDiagnosticCategory};
    AgentEngineEvent::ModelResponseRejected {
        step,
        discarded_tool_call_ids: vec!["invalid-patch".into()],
        continuation: true,
        tool_hint: Some("apply_patch".into()),
        rejection: Some(AgentModelResponseRejection::ToolArguments {
            diagnostics: vec![AgentRejectedToolProposal {
                call_id: "invalid-patch".into(),
                name: "apply_patch".into(),
                diagnostic: ToolArgumentDiagnostic {
                    category: ToolArgumentDiagnosticCategory::Syntax,
                    line: Some(1), column: Some(12), byte_len: 12,
                    digest: digest_bytes(b"{\"files\":]}}"),
                },
            }],
        }),
        feedback: Some("The completed tool proposal has invalid JSON arguments at line 1 column 12. No tool in this batch was admitted. Generate a new native proposal.".into()),
    }
}

fn enable_fixture_route(journal: &mut EngineTurnJournal) -> ChatRouteIdentity {
    let route = ChatRouteIdentity::new("preset@1", "agent_chat", "fixture-route".into(), 1);
    Arc::get_mut(&mut journal.0).expect("fixture journal is not shared yet").route = Some(route.clone());
    route
}

fn model_causality(journal: &EngineTurnJournal, route: ChatRouteIdentity, step: u16) -> ChatCausality {
    ChatCausality {
        agent_session_id: journal.0.session.clone(),
        turn_operation_id: journal.0.operation.clone(),
        causation_event_id: journal.0.root.clone(),
        resolved_snapshot_ref: journal.0.snapshot.clone(),
        route_identity: route,
        operation_id: format!("{}:model:{step}", journal.0.operation.as_ref()).into(),
    }
}

#[tokio::test]
async fn rejected_proposal_and_correction_preserve_canonical_facts_and_accepted_root_authority() {
    let (mut journal, pool) = test_fixture().await;
    let route = enable_fixture_route(&mut journal);
    let rejection = rejected_arguments(1);
    let events = [
        AgentEngineEvent::ModelStepStarted { step: 1, operation_id: "turn:model:1".into(), correction_source: None },
        AgentEngineEvent::ToolCallDelta { step: 1, call_id: "invalid-patch".into(), name: "apply_patch".into(), arguments_delta: "PRIVATE_MALFORMED_ARGUMENTS".into() },
        rejection.clone(),
        AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "turn:model:2".into(), correction_source: Some(1) },
        AgentEngineEvent::ModelResponseValidated { step: 2 },
    ];
    let mut buffer = super::super::runtime_event_buffer::AgentEventBuffer::default();
    for event in &events {
        for record in buffer.project(event) {
            journal.append(serde_json::to_string(&record).unwrap(), None, EngineJournalWrite::Progress).await.unwrap();
        }
    }
    let before = journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    let facts = journal.0.store.native_recovery_facts(&journal.0.session, &journal.0.operation).await.unwrap();
    let progress = facts.events.iter().filter(|event| event.kind.0 == "runtime/progress-recorded").collect::<Vec<_>>();
    assert_eq!(progress.len(), events.len());
    assert!(progress.iter().all(|event| event.causation_event_id.as_ref() == Some(&journal.0.root)));
    let decoded = progress.iter().map(|event| {
        serde_json::from_value::<AgentEngineEvent>(facts.event_payloads[event.event_id.as_ref()]["event"].clone()).unwrap()
    }).collect::<Vec<_>>();
    assert_eq!(decoded[2], rejection, "reason and exact engine feedback must survive hydration");
    assert_eq!(decoded[3], events[3]);
    assert_eq!(decoded[4], events[4]);
    assert!(matches!(&decoded[1], AgentEngineEvent::ToolCallDelta { arguments_delta, .. } if arguments_delta.is_empty()));
    assert!(!serde_json::to_string(&facts.event_payloads).unwrap().contains("PRIVATE_MALFORMED_ARGUMENTS"));

    // An exact canonical receipt retry remains one immutable fact. Reading a
    // fresh fact window does not invent a second rejection or a new budget.
    let saved = progress[2];
    let retry = SessionEventAppend {
        agent_session_id: saved.agent_session_id.clone(), event_id: saved.event_id.clone(),
        producer_id: saved.producer_id.clone(), idempotency_key: saved.idempotency_key.clone(),
        semantic_event: SemanticSessionEventDraft {
            kind: saved.kind.clone(), kind_version: saved.kind_version,
            correlation_id: saved.correlation_id.clone(), causation_event_id: saved.causation_event_id.clone(),
            payload: saved.payload.clone(),
        },
    };
    journal.0.store.append_native_event(&journal.0.lease, &retry, None).await.unwrap();
    let refreshed = journal.0.store.native_recovery_facts(&journal.0.session, &journal.0.operation).await.unwrap();
    assert_eq!(refreshed.event_payloads, facts.event_payloads);
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(), before);
    let effects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=?")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    let admitted_tools: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind IN ('tool/call-started','tool/result-recorded')")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!((effects, admitted_tools), (0, 0), "proposal observations cannot fabricate invocations or effects");

    let valid = model_causality(&journal, route, 2);
    let mut wrong_authority = valid.clone();
    wrong_authority.causation_event_id = saved.event_id.clone();
    assert!(journal.authorize(&wrong_authority).await.is_err(), "correction provenance is not user authority");
    journal.authorize(&valid).await.unwrap();
    journal.authorize(&valid).await.unwrap();
    let claims: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(claims, 1, "an exact model-admission retry preserves the original claim");
}

#[tokio::test]
async fn rejected_proposal_commit_failure_fences_the_successor_model_operation() {
    let (mut journal, pool) = test_fixture().await;
    let route = enable_fixture_route(&mut journal);
    sqlx::query("CREATE TRIGGER reject_proposal_rejection BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='model_response_rejected' BEGIN SELECT RAISE(FAIL,'fixture proposal rejection commit failure'); END")
        .execute(&pool).await.unwrap();
    let before = journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.append(serde_json::to_string(&rejected_arguments(1)).unwrap(), None, EngineJournalWrite::Progress).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(), before);
    assert!(journal.0.cursor.lock().await.uncertain);
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted {
        step: 2, operation_id: "turn:model:2".into(), correction_source: Some(1),
    }).unwrap(), None, EngineJournalWrite::Progress).await.is_err());
    assert!(journal.authorize(&model_causality(&journal, route, 2)).await.is_err());
    let claims: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(claims, 0);
    assert_eq!(journal.sequence(), 0);
}

#[tokio::test]
async fn validation_observation_after_cancellation_cannot_open_a_model_or_cross_a_superseded_lease() {
    let (mut journal, pool) = test_fixture().await;
    let route = enable_fixture_route(&mut journal);
    journal.0.cancellation.cancel();
    journal.append(serde_json::to_string(&AgentEngineEvent::ModelResponseValidated { step: 1 }).unwrap(), None,
        EngineJournalWrite::Settlement).await.unwrap();
    assert!(journal.authorize(&model_causality(&journal, route, 2)).await.is_err());
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted {
        step: 2, operation_id: "turn:model:2".into(), correction_source: Some(1),
    }).unwrap(), None, EngineJournalWrite::Progress).await.is_err());
    let before = journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    sqlx::query("UPDATE agent_turns SET execution_fence=execution_fence+1 WHERE session_id=? AND operation_id=?")
        .bind(journal.0.session.as_ref()).bind(journal.0.operation.as_ref()).execute(&pool).await.unwrap();
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::ModelResponseValidated { step: 2 }).unwrap(), None,
        EngineJournalWrite::Settlement).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(), before);
}

#[tokio::test]
async fn correction_observations_do_not_settle_an_unknown_effect_or_admit_a_successor_model() {
    use nomifun_agent_session::{EffectEventRequest, EffectStrategy, EffectTerminalState};
    let (mut journal, pool) = test_fixture().await;
    let route = enable_fixture_route(&mut journal);
    let tool_event_id = EventId::from("admitted-write");
    let tool_operation = OperationId::from("turn:tool:write");
    journal.0.store.append_native_event(&journal.0.lease, &SessionEventAppend {
        agent_session_id: journal.0.session.clone(), event_id: tool_event_id.clone(),
        producer_id: "runtime_supervisor".into(), idempotency_key: "admitted-write".into(),
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("tool/call-started".into()), kind_version: 1, correlation_id: "write-call".into(),
            causation_event_id: Some(journal.0.turn_started.clone()),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "operation_id": tool_operation, "capability_id": "workspace.files", "action_id": "workspace.files/write"
            }))),
        },
    }, None).await.unwrap();
    let effect = EffectEventRequest {
        agent_session_id: journal.0.session.clone(), effect_id: "unknown-write".into(),
        turn_id: journal.0.operation.clone(), operation_id: tool_operation,
        owner_domain: "workspace".into(), capability_module: "workspace.files".into(), action_id: "workspace.files/write".into(),
        resource_binding_id: None, resource_key: Some("workspace:fixture".into()),
        input_digest: digest_bytes(b"write-input"), recorded_at: 1_788_000_000_010,
        event_id: "effect-write-started".into(), producer_id: "capability-host".into(), idempotency_key: "effect-write-started".into(),
        correlation_id: "unknown-write".into(), strategy: EffectStrategy::ExternalUncertainEffect,
        causation_event_id: Some(tool_event_id), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
    };
    let started = journal.0.store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap();
    journal.0.store.record_effect_terminal(EffectEventRequest {
        event_id: "effect-write-unknown".into(), producer_id: "workspace-owner".into(),
        causation_event_id: Some(started.event_id), ..effect
    }, EffectTerminalState::Uncertain).await.unwrap();
    // Unknown effects move the Session out of running admission. Late model
    // observations can still close, using the settlement-only write path.
    for event in [rejected_arguments(2), AgentEngineEvent::ModelResponseValidated { step: 3 }] {
        journal.append(serde_json::to_string(&event).unwrap(), None, EngineJournalWrite::Settlement).await.unwrap();
    }
    assert!(journal.authorize(&model_causality(&journal, route, 4)).await.is_err());
    let retained: (String, String) = sqlx::query_as("SELECT state,terminal_event_id FROM agent_effects WHERE effect_id='unknown-write'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(retained, ("unknown".into(), "effect-write-unknown".into()));
    let claims: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(claims, 0);
}

#[tokio::test]
async fn cleanup_retry_preserves_completed_previous_assistant_step() {
    let (journal, pool) = if let Some(root) = std::env::var_os("NOMIFUN_RELIABILITY_EVIDENCE_DIR") {
        let data = std::path::PathBuf::from(root).join("previous-assistant-step").join("data");
        std::fs::create_dir_all(&data).unwrap();
        test_fixture_at(&data.join("journal.db")).await
    } else {
        test_fixture().await
    };
    journal.append(serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
        step: 1, text: "first response".into(),
    }).unwrap(), None, EngineJournalWrite::Progress).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_previous_completion BEFORE INSERT ON agent_events WHEN NEW.kind='message/completed' BEGIN SELECT RAISE(FAIL,'fixture previous step completion failure'); END")
        .execute(&pool).await.unwrap();
    let second = serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
        step: 2, text: "second response".into(),
    }).unwrap();
    let failure = journal.append(second.clone(), None, EngineJournalWrite::Cleanup).await.unwrap_err();
    assert!(failure.to_string().contains("fixture previous step completion failure"));
    assert_eq!(journal.sequence(), 1);
    let before = journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
        step: 2, text: "different response".into(),
    }).unwrap(), None, EngineJournalWrite::Cleanup).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(), before);
    sqlx::query("DROP TRIGGER reject_previous_completion").execute(&pool).await.unwrap();
    journal.append(second, None, EngineJournalWrite::Cleanup).await.unwrap();
    let first_id = canonical_assistant_step_message_id(journal.0.root.as_ref(), 1).unwrap();
    let completion: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='message/completed' AND correlation_id=?")
        .bind(journal.0.session.as_ref()).bind(first_id).fetch_all(&pool).await.unwrap();
    assert_eq!(completion.len(), 1, "cleanup retry lost the previous response completion");
    let completed: Value = serde_json::from_str(&completion[0]).unwrap();
    assert_eq!(completed["part_count"], 1);
    assert_eq!(completed["content_digest"], digest_bytes(b"first response").as_ref());
    let parts: Vec<String> = sqlx::query_scalar("SELECT json_extract(inline_json,'$.content') FROM agent_events WHERE session_id=? AND kind='message/content-part' ORDER BY seq")
        .bind(journal.0.session.as_ref()).fetch_all(&pool).await.unwrap();
    assert_eq!(parts, vec!["first response", "second response"]);
    assert_eq!(journal.sequence(), 2);
}

#[tokio::test]
async fn failed_cleanup_write_allows_only_its_exact_receipt_retry() {
    let (journal,pool)=test_fixture().await;
    let cleanup=json!({"event":"host_cleanup_proven","source":"exact-owner"}).to_string();
    sqlx::query("CREATE TRIGGER reject_cleanup_journal BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_cleanup_proven' BEGIN SELECT RAISE(FAIL,'fixture cleanup write failure'); END")
        .execute(&pool).await.unwrap();
    assert!(journal.append(cleanup.clone(),None,EngineJournalWrite::Cleanup).await.is_err());
    assert_eq!(journal.sequence(),0);
    let before=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.append(json!({"event":"host_cleanup_proven","source":"other-owner"}).to_string(),None,
        EngineJournalWrite::Cleanup).await.is_err());
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted {
        step:1,operation_id:"must-not-run".into(),correction_source:None}).unwrap(),None,EngineJournalWrite::Progress).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before);
    sqlx::query("DROP TRIGGER reject_cleanup_journal").execute(&pool).await.unwrap();
    journal.append(cleanup,None,EngineJournalWrite::Cleanup).await.unwrap();
    assert_eq!(journal.sequence(),1);
    journal.append(serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:2}).unwrap(),None,
        EngineJournalWrite::Terminal).await.unwrap();
    let state:String=sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=?")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(state,"cancelled");
}

#[tokio::test]
async fn partially_committed_terminal_retries_without_a_new_journal_record() {
    let (journal,pool)=test_fixture().await;
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_turn_cancelled BEFORE INSERT ON agent_events WHEN NEW.kind='turn/cancelled' BEGIN SELECT RAISE(FAIL,'fixture terminal commit failure'); END")
        .execute(&pool).await.unwrap();
    let terminal=serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:2}).unwrap();
    assert!(journal.append(terminal.clone(),None,EngineJournalWrite::Terminal).await.is_err());
    let before=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:3}).unwrap(),None,
        EngineJournalWrite::Terminal).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before);
    sqlx::query("DROP TRIGGER reject_turn_cancelled").execute(&pool).await.unwrap();
    journal.append(terminal,None,EngineJournalWrite::Terminal).await.unwrap();
    let journal_records:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    let terminals:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/cancelled'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(journal_records,2);
    assert_eq!(terminals,1);
    assert_eq!(journal.sequence(),2);
}

#[tokio::test]
async fn journal_window_renews_only_after_checkpoint_ack_and_keeps_cumulative_usage() {
    let (journal,_) = test_fixture().await;
    journal.save_execution_checkpoint(checkpoint(&journal,false),owner(&journal)).await.unwrap().unwrap();
    journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted { step:1,operation_id:"model:1".into(),correction_source:None }).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    let (before_sequence,before_total) = { let cursor=journal.0.cursor.lock().await; assert!(cursor.bytes>0); (cursor.sequence,cursor.total_bytes) };
    let receipt=journal.save_execution_checkpoint(checkpoint(&journal,true),owner(&journal)).await.unwrap().unwrap();
    let cursor=journal.0.cursor.lock().await;
    assert_eq!(cursor.segment,2); assert_eq!(cursor.bytes,0);
    assert_eq!(cursor.window_start_sequence,before_sequence+1);
    assert!(cursor.total_bytes>before_total);
    assert_eq!(cursor.checkpoint_revision,receipt.revision);
    drop(cursor);
    let saved=journal.0.store.load_native_checkpoint(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
    assert_eq!(saved.through_seq,receipt.through_seq);
    assert_eq!(saved.state.0["model_steps"],1);
    assert_eq!(saved.state.0["segments"]["policy"]["max_segments"],2);
}

#[tokio::test]
async fn lost_checkpoint_commit_never_buys_a_new_window_or_erases_usage() {
    let (journal,pool)=test_fixture().await;
    journal.save_execution_checkpoint(checkpoint(&journal,false),owner(&journal)).await.unwrap().unwrap();
    let before_head=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    let before={ let cursor=journal.0.cursor.lock().await; (cursor.sequence,cursor.total_bytes,cursor.bytes,cursor.window_start_sequence) };
    // Deterministic persistence fault in the isolated test database. The
    // metadata event and checkpoint body must roll back as one transaction.
    sqlx::query("CREATE TRIGGER reject_native_checkpoint BEFORE UPDATE OF native_checkpoint_json ON agent_turns BEGIN SELECT RAISE(FAIL,'injected checkpoint commit loss'); END")
        .execute(&pool).await.unwrap();
    assert!(journal.save_execution_checkpoint(checkpoint(&journal,true),owner(&journal)).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before_head);
    let cursor=journal.0.cursor.lock().await;
    assert_eq!((cursor.sequence,cursor.total_bytes,cursor.bytes,cursor.window_start_sequence),before);
    assert_eq!(cursor.segment,1); assert!(cursor.uncertain);
    drop(cursor);
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted { step:1,operation_id:"must-not-start".into(),correction_source:None }).unwrap(),None,EngineJournalWrite::Progress).await.is_err());
    let saved=journal.0.store.load_native_checkpoint(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
    assert_eq!(saved.revision,1);
    assert_eq!(saved.state.0["segments"]["segment"],1);
}

#[tokio::test]
async fn pause_requires_cleanup_phase_and_never_appends_a_terminal_turn() {
    let (journal,pool)=test_fixture().await;
    journal.save_execution_checkpoint(checkpoint(&journal,false),owner(&journal)).await.unwrap().unwrap();
    let paused=serde_json::to_string(&AgentEngineEvent::TurnPaused { model_steps:0,reason:"EXECUTION_USER_REQUESTED".into() }).unwrap();
    assert!(journal.append(paused.clone(),None,EngineJournalWrite::Terminal).await.is_err());
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    journal.append(paused,None,EngineJournalWrite::Terminal).await.unwrap();
    let inspection=journal.0.store.inspect_latest_native_execution(&owner(&journal),&journal.0.session).await.unwrap().unwrap();
    assert_eq!(inspection.state,"paused"); assert_eq!(inspection.turn_state,"running");
    assert!(inspection.pause.unwrap().cleanup_proven && inspection.checkpoint_retained);
    let terminals:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind IN ('turn/completed','turn/failed','turn/cancelled')")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(terminals,0);
}
