use super::*;
use nomifun_agent_runtime::{AgentExecutionCheckpoint, AgentExecutionSegmentState, AgentSegmentPolicy, EngineBinding};
#[path = "engine_journal_settlement_tests.rs"]
mod settlement_tests;

fn checkpoint(journal:&EngineTurnJournal, renewed:bool) -> AgentExecutionCheckpoint {
    AgentExecutionCheckpoint { version:1,
        binding:EngineBinding::new(journal.0.session.clone(),"native-binding".into(),"test".into(),"a".repeat(64).into(),journal.0.snapshot.clone()).unwrap(),
        turn_operation_id:journal.0.operation.clone(),active_set_generation:0,model_steps:u16::from(renewed),tool_call_count:0,
        accepted_input_count:1,applied_steering_receipts:vec![],plan:Default::default(),work:Default::default(),patch_recovery:Default::default(),
        control_rejections:Default::default(),pending_user_input: None, awaiting_children: None, collaboration: Default::default(), completion_review:Default::default(),segments:Some(AgentExecutionSegmentState {
            policy:AgentSegmentPolicy { max_segments:2,max_no_progress_segments:2 },model_steps_per_segment:1,
            segment:if renewed {2} else {1},segment_start_step:u16::from(renewed),no_progress_segments:0,
            progress_at_segment_start:0,progress_fingerprints:vec![],
        }) }
}

fn owner(journal:&EngineTurnJournal) -> nomifun_agent_contracts::PrincipalRef {
    nomifun_agent_contracts::PrincipalRef { principal_kind:"user".into(),principal_id:journal.0.user.clone() }
}

#[tokio::test]
async fn production_journal_pure_child_wait_tail_reclaims_the_same_turn_without_replaying_any_input_or_tool() {
    use nomifun_agent_runtime::{AgentChildrenWait,AgentChildrenWaitMode,AgentCollaborationObservation,AgentCollaborationSnapshot,AgentToolResult,AgentTurnRecovery};
    use nomifun_chat_model_broker::ChatToolCall;
    let (journal,pool)=test_fixture().await;
    let mut checkpoint=checkpoint(&journal,false);checkpoint.segments=None;checkpoint.model_steps=1;checkpoint.tool_call_count=1;
    let wait=AgentChildrenWait {execution_ids:vec!["original-delegation".into()],after_cursor:None,mode:AgentChildrenWaitMode::All};
    let cursor=AgentCollaborationSnapshot::observation_cursor(std::collections::BTreeMap::from([("original-delegation".into(),
        AgentCollaborationObservation {state:"4".repeat(64),ready:"5".repeat(64)})])).unwrap();
    let prefix=vec![AgentEngineEvent::TurnStarted {binding:checkpoint.binding.clone(),turn_operation_id:journal.0.operation.clone()},
        AgentEngineEvent::ModelStepStarted {step:1,operation_id:"original:model:1".into(),correction_source:None},
        AgentEngineEvent::ToolCallCompleted {step:1,call:ChatToolCall {call_id:"original-wait".into(),name:"agent_wait".into(),
            arguments:StrictJsonValue(serde_json::to_value(&wait).unwrap()),provider_metadata:None}},
        AgentEngineEvent::ToolCompleted {step:1,runtime_hold:None,result:AgentToolResult::text("original-wait".into(),
            json!({"state":"awaiting_children","cursor":cursor}).to_string(),false)}];
    for event in &prefix {journal.append(serde_json::to_string(event).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();}
    checkpoint.collaboration.pending_child_executions.insert("original-delegation".into());checkpoint.awaiting_children=Some(wait.clone());
    let saved=journal.save_execution_checkpoint(checkpoint.clone(),owner(&journal)).await.unwrap().unwrap();
    let wait_event=AgentEngineEvent::AwaitingChildren {model_steps:1,request:wait};
    journal.append(serde_json::to_string(&wait_event).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    let original_root=journal.0.root.clone();let original_operation=journal.0.operation.clone();
    let store=journal.0.store.clone();
    sqlx::query("UPDATE agent_turns SET execution_lease_until=0 WHERE session_id=? AND operation_id=?")
        .bind(journal.0.session.as_ref()).bind(journal.0.operation.as_ref()).execute(&pool).await.unwrap();
    let lease=store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {owner:owner(&journal),
        agent_session_id:journal.0.session.clone(),operation_id:original_operation.clone(),snapshot:journal.0.snapshot.clone(),
        active_set_generation:0,holder:"same-original-wait-recovery".into(),expected_fence:journal.0.lease.fence(),
        checkpoint:Some((saved.revision,saved.digest.clone(),saved.through_seq))}).await.unwrap();
    let facts=store.native_recovery_facts(&journal.0.session,&original_operation).await.unwrap();
    let mut before=Vec::new();let mut after=Vec::new();
    for event in facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"&&event.correlation_id.as_ref()==original_operation.as_ref()) {
        let typed=serde_json::from_value::<AgentEngineEvent>(facts.event_payloads[event.event_id.as_ref()]["event"].clone()).unwrap();
        if event.seq<=saved.through_seq {before.push(typed);}else {after.push(typed);}
    }
    assert_eq!(after,vec![wait_event]);
    let recovery=AgentTurnRecovery::new(checkpoint.clone(),saved.revision,lease.fence(),before,after,vec![]).unwrap();
    assert_eq!(recovery.checkpoint(),&checkpoint);assert_eq!(recovery.last_model_step(),1);
    assert_eq!(facts.head.active_turn_id.as_deref(),Some(original_operation.as_ref()));
    assert!(facts.events.iter().any(|event|event.event_id==original_root&&event.kind.0=="message/user-accepted"));
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_turns WHERE session_id=?),\
        (SELECT COUNT(*) FROM agent_effects WHERE session_id=?),(SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='message/user-accepted')")
        .bind(journal.0.session.as_ref()).bind(journal.0.session.as_ref()).bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(counts,(1,0,1));
    assert!(journal.append(json!({"event":"model_step_started","step":2,"operation_id":"late-old-owner"}).to_string(),None,EngineJournalWrite::Progress).await.is_err());
}

#[tokio::test]
async fn fenced_progress_before_runtime_signal_does_not_poison_original_cleanup() {
    let(journal,pool)=test_fixture().await;
    journal.0.store.cancel_active_turn(&journal.0.session,"original-user-stop".into(),"session_api".into()).await.unwrap();
    assert!(!journal.0.cancellation.is_cancelled(),"hold the real durable-commit to native-signal boundary");
    let before=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    let late=AgentEngineEvent::ModelStepStarted {step:1,operation_id:"late-original-model-prefix".into(),correction_source:None};
    assert!(journal.append(serde_json::to_string(&late).unwrap(),None,EngineJournalWrite::Progress).await.is_err());
    assert_eq!(journal.sequence(),0);
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before,"fence rejection committed no progress");
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await
        .expect("a known rejected prefix cannot make exact original native cleanup permanently uncertain");
    let receipt=journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap();
    assert_eq!(receipt.status,TurnReceiptStatus::Cancelled);
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='model_step_started'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(count,0,"cleanup never re-admits or retries the rejected model work");
}

#[tokio::test]
async fn fenced_checkpoint_before_runtime_signal_keeps_original_cleanup_writable() {
    let(journal,_pool)=test_fixture().await;
    journal.0.store.cancel_active_turn(&journal.0.session,"original-checkpoint-stop".into(),"session_api".into()).await.unwrap();
    assert!(!journal.0.cancellation.is_cancelled());
    let before=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.save_execution_checkpoint(checkpoint(&journal,false),owner(&journal)).await.is_err());
    assert_eq!(journal.sequence(),0);
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before);
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await
        .expect("transactionally rejected checkpoint is not an unknown native outcome");
    assert!(journal.0.store.load_native_checkpoint(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().is_none());
}

#[tokio::test]
async fn cancellation_keeps_original_execution_lease_alive_for_retained_settlement() {
    let (journal, pool) = test_fixture().await;
    sqlx::query("UPDATE agent_turns SET execution_lease_until=0 WHERE session_id=? AND operation_id=?")
        .bind(journal.0.session.as_ref()).bind(journal.0.operation.as_ref()).execute(&pool).await.unwrap();
    journal.0.cancellation.cancel();
    journal.start_heartbeat();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let renewed: i64 = sqlx::query_scalar("SELECT execution_lease_until FROM agent_turns WHERE session_id=? AND operation_id=?")
                .bind(journal.0.session.as_ref()).bind(journal.0.operation.as_ref()).fetch_one(&pool).await.unwrap();
            if renewed > now_ms() { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("cancellation must not expire an owner still settling its effects");
    journal.0.store.verify_native_execution(&journal.0.lease).await.unwrap();
    assert!(journal.append(json!({"event":"unadmitted-progress"}).to_string(), None, EngineJournalWrite::Progress).await.is_err(),
        "renewal permits settlement, not new progress after cancellation");
}

#[tokio::test]
async fn shared_tool_bodies_preserve_journal_accounting_model_pairing_and_native_checkpoint_recovery() {
    use nomifun_agent_runtime::{AgentToolResult, AgentTurnRecovery, replay_closed_turn};
    use nomifun_chat_model_broker::{ChatContentPart,ChatMessage,ChatRole,ChatToolCall};
    let (journal,pool)=test_fixture().await;
    let mut checkpoint=checkpoint(&journal,false);
    checkpoint.model_steps=1;checkpoint.tool_call_count=1;checkpoint.segments=None;
    let call=ChatToolCall {call_id:"shared-write".into(),name:"write_file".into(),
        arguments:StrictJsonValue(json!({"path":"report.md","content":"Exact source😀".repeat(9000)})),provider_metadata:None};
    let result=AgentToolResult::text(call.call_id.clone(),"Read result😀".repeat(800),false);
    let native=vec![
        AgentEngineEvent::TurnStarted {binding:checkpoint.binding.clone(),turn_operation_id:journal.0.operation.clone()},
        AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None},
        AgentEngineEvent::ToolCallCompleted {step:1,call:call.clone()},
        AgentEngineEvent::ToolStarted {step:1,call_id:call.call_id.clone(),capability_id:"workspace.files".into(),action_id:"workspace.files/write".into()},
    ];
    let mut minimum_accounted=0usize;
    for event in native {
        let payload=serde_json::to_string(&event).unwrap();minimum_accounted+=payload.len()+256;
        journal.append(payload,None,EngineJournalWrite::Progress).await.unwrap();
    }
    for event in [json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"shared-owner","call_id":call.call_id,
        "capability_id":"workspace.files","action_id":"workspace.files/write","model_name":"write_file"}}),
        json!({"event":"host_tool_settled","operation_id":"shared-owner","call_id":call.call_id,"result":result,"error":null})] {
        let payload=event.to_string();minimum_accounted+=payload.len()+256;
        journal.append(payload,None,EngineJournalWrite::Progress).await.unwrap();
    }
    let completed=AgentEngineEvent::ToolCompleted {step:1,result:result.clone(),runtime_hold:None};
    let payload=serde_json::to_string(&completed).unwrap();minimum_accounted+=payload.len()+256;
    journal.append(payload,None,EngineJournalWrite::Progress).await.unwrap();
    assert_eq!(journal.0.cursor.lock().await.total_bytes,minimum_accounted,
        "content references must not shrink the original hydrated journal allowance");
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_payloads WHERE session_id=? AND media_type='application/vnd.nomifun.tool-output+json'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(count,1,"owner settlement, tool result and Runtime observation share one output body");
    let saved=journal.save_execution_checkpoint(checkpoint.clone(),owner(&journal)).await.unwrap().unwrap();
    let facts=journal.0.store.native_recovery_facts(&journal.0.session,&journal.0.operation).await.unwrap();
    let prefix=facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded")
        .filter_map(|event| {
            let value=facts.event_payloads[event.event_id.as_ref()]["event"].clone();
            if matches!(value.get("event").and_then(Value::as_str),Some("host_tool_dispatch"|"host_tool_settled")){None}
            else{Some(serde_json::from_value::<AgentEngineEvent>(value).unwrap())}
        }).collect::<Vec<_>>();
    assert!(prefix.iter().any(|event|matches!(event,AgentEngineEvent::ToolCallCompleted {call:restored,..} if restored==&call)));
    assert!(prefix.iter().any(|event|matches!(event,AgentEngineEvent::ToolCompleted {result:restored,..} if restored==&result)));
    sqlx::query("UPDATE agent_turns SET execution_lease_until=0 WHERE session_id=?")
        .bind(journal.0.session.as_ref()).execute(&pool).await.unwrap();
    let lease=journal.0.store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
        owner:owner(&journal),agent_session_id:journal.0.session.clone(),operation_id:journal.0.operation.clone(),snapshot:journal.0.snapshot.clone(),
        active_set_generation:0,holder:"recovered-shared-content".into(),expected_fence:journal.0.lease.fence(),
        checkpoint:Some((saved.revision,saved.digest.clone(),saved.through_seq)),
    }).await.unwrap();
    AgentTurnRecovery::new(checkpoint,saved.revision,lease.fence(),prefix.clone(),vec![],vec![]).unwrap();
    let mut closed=prefix;
    closed.push(AgentEngineEvent::TurnFailed {model_steps:1,message:"Saved historical result".into(),failure:None});
    let mut history=Vec::new();
    replay_closed_turn(&mut history,ChatMessage {role:ChatRole::User,content:vec![ChatContentPart::Text {text:"Inspect the source".into()}],provider_round_id:None},&closed).unwrap();
    assert!(history.iter().flat_map(|message|&message.content).any(|part|matches!(part,ChatContentPart::ToolCall {arguments,..} if arguments==&call.arguments)));
    assert!(history.iter().flat_map(|message|&message.content).any(|part|matches!(part,ChatContentPart::ToolResult {output,..} if output==&result.output)));
}

#[tokio::test]
async fn large_process_stdout_keeps_typed_exit_timeout_and_cleanup_after_durable_hydration() {
    use nomifun_agent_runtime::AgentToolResult;
    use super::super::engine_tool_host::bounded_engine_tool_result;
    use sha2::{Digest,Sha256};
    let (journal,_pool)=test_fixture().await;
    let stdout="PRIVATE_STDOUT😀\\\"\n".repeat(6000);
    let stdout_digest=format!("{:x}",Sha256::digest(stdout.as_bytes()));
    for timed_out in [false,true] {
        let call=if timed_out {"large-timeout"} else {"large-exit"};
        let original=json!({"state":if timed_out {"timed_out"} else {"exited"},"process_id":"owned-process","success":false,
            "exit_code":if timed_out {Value::Null} else {json!(7)},"signal":null,
            "output":{"text":stdout,"truncated":false,"retained_bytes":stdout.len(),"dropped_bytes":0,"next_cursor":stdout.len()},
            "cleanup":{"reaped":true,"errors":[],"interrupt_attempted":timed_out,"terminate_attempted":timed_out,"force_kill_attempted":false}});
        let live=AgentToolResult::text(call.into(),original.to_string(),true);
        let unchanged=live.clone();
        let bounded=bounded_engine_tool_result(&live);
        assert_eq!(live,unchanged,"durable projection must not mutate the live model result");
        assert!(serde_json::to_vec(&bounded).unwrap().len()<=32*1024);
        let mut persisted:Value=serde_json::from_str(&bounded.output_text()).unwrap();
        let excerpt=persisted["output"]["text"].as_str().unwrap();
        assert!(excerpt.contains("Durable observation truncated: original"));
        assert!(excerpt.contains(&stdout_digest));
        let mut metadata=original.clone();metadata["output"]["text"]=Value::Null;persisted["output"]["text"]=Value::Null;
        assert_eq!(persisted,metadata,"all owner state/cursor/truncation/cleanup metadata must remain unchanged");
        for event in [json!({"event":"host_tool_dispatch","dispatch":{"operation_id":call,"call_id":call,
            "capability_id":"workspace.process","action_id":"workspace.process/start","model_name":"start_process"}}),
            json!({"event":"host_tool_settled","operation_id":call,"call_id":call,"result":bounded,"error":null})] {
            journal.append(event.to_string(),None,EngineJournalWrite::Progress).await.unwrap();
        }
        let mut buffer=super::super::runtime_event_buffer::AgentEventBuffer::default();
        for event in buffer.project(&AgentEngineEvent::ToolCompleted {step:1,result:live,runtime_hold:None}) {
            journal.append(serde_json::to_string(&event).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
        }
        let facts=journal.0.store.native_recovery_facts(&journal.0.session,&journal.0.operation).await.unwrap();
        let result=facts.event_payloads.values().find_map(|payload| {
            let event=payload.get("event")?;
            if event.get("event").and_then(Value::as_str)!=Some("tool_completed") {return None;}
            let result=event.get("result")?;
            (result.get("call_id").and_then(Value::as_str)==Some(call)).then(||serde_json::from_value::<AgentToolResult>(result.clone()).unwrap())
        }).unwrap();
        assert_eq!(result,bounded,"host and Runtime persist the exact same bounded JSON bytes");
        let display=nomifun_engine_core::tool_result_display(Some("workspace.process"),Some("workspace.process/start"),&result);
        if timed_out {assert_eq!(display.command_timed_out,Some(true));assert_eq!(display.command_exit_code,None);}
        else {assert_eq!(display.command_exit_code,Some(7));assert_eq!(display.non_fatal_failure,Some(true));}
        let public=serde_json::to_string(&display).unwrap();
        assert!(!public.contains("PRIVATE_STDOUT")&&!public.contains("output"),"product facts do not republish the stdout excerpt");
        assert_eq!(nomifun_engine_core::tool_result_display(Some("mcp.server"),Some("mcp.tool/call"),&result).command_exit_code,None,
            "preserving JSON does not authorize another capability to impersonate process status");
    }
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

async fn journal_with_original_unknown_effect() -> (EngineTurnJournal, nomifun_db::SqlitePool, ChatRouteIdentity) {
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
    (journal, pool, route)
}

#[tokio::test]
async fn correction_observations_do_not_settle_an_unknown_effect_or_admit_a_successor_model() {
    let (journal, pool, route) = journal_with_original_unknown_effect().await;
    // Unknown effects move the Session out of running admission. Late model
    // observations can still close, using the settlement-only write path.
    for event in [rejected_arguments(2), AgentEngineEvent::ModelResponseValidated { step: 3 }] {
        journal.append(serde_json::to_string(&event).unwrap(), None, EngineJournalWrite::Settlement).await.unwrap();
    }
    assert!(journal.authorize(&model_causality(&journal, route.clone(), 4)).await.is_err());
    let retained: (String, String) = sqlx::query_as("SELECT state,terminal_event_id FROM agent_effects WHERE effect_id='unknown-write'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(retained, ("unknown".into(), "effect-write-unknown".into()));
    let claims: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(claims, 0);
    assert_eq!(journal.0.store.head(&journal.0.session).await.unwrap().status, "reconciliation");
    let original_effect = journal.0.store.read_effect(&journal.0.session, "unknown-write").await.unwrap().unwrap();
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(), None, EngineJournalWrite::Cleanup).await.unwrap();
    journal.append(serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:3}).unwrap(), None,
        EngineJournalWrite::Terminal).await.expect("the same original owner may close its cancelled Turn after actual resource proof");
    assert_eq!(journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap().status, TurnReceiptStatus::Cancelled);
    assert_eq!(journal.0.store.read_effect(&journal.0.session,"unknown-write").await.unwrap().unwrap(), original_effect,
        "cancel terminal is not business reconciliation or replay authority");
    assert!(journal.authorize(&model_causality(&journal, route, 5)).await.is_err());
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
        step: 1, text: "first response".into(), source: None }).unwrap(), None, EngineJournalWrite::Progress).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_previous_completion BEFORE INSERT ON agent_events WHEN NEW.kind='message/completed' BEGIN SELECT RAISE(FAIL,'fixture previous step completion failure'); END")
        .execute(&pool).await.unwrap();
    let second = serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
        step: 2, text: "second response".into(), source: None }).unwrap();
    let failure = journal.append(second.clone(), None, EngineJournalWrite::Cleanup).await.unwrap_err();
    assert!(failure.to_string().contains("fixture previous step completion failure"));
    assert_eq!(journal.sequence(), 1);
    let before = journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
        step: 2, text: "different response".into(), source: None }).unwrap(), None, EngineJournalWrite::Cleanup).await.is_err());
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
    let (journal,pool,route)=journal_with_original_unknown_effect().await;
    let original_effect = journal.0.store.read_effect(&journal.0.session,"unknown-write").await.unwrap().unwrap();
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_turn_cancelled BEFORE INSERT ON agent_events WHEN NEW.kind='turn/cancelled' BEGIN SELECT RAISE(FAIL,'fixture terminal commit failure'); END")
        .execute(&pool).await.unwrap();
    let terminal=serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:2}).unwrap();
    let failed = journal.append(terminal.clone(),None,EngineJournalWrite::Terminal).await.unwrap_err();
    assert!(failed.to_string().contains("fixture terminal commit failure"));
    assert!(journal.0.cursor.lock().await.uncertain, "a failed cancellation transaction retains its exact pending terminal witness");
    assert!(journal.authorize(&model_causality(&journal, route, 4)).await.is_err());
    assert_eq!(journal.0.store.head(&journal.0.session).await.unwrap().status, "reconciliation");
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
    assert_eq!(journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap().status, TurnReceiptStatus::Cancelled);
    assert_eq!(journal.0.store.read_effect(&journal.0.session,"unknown-write").await.unwrap().unwrap(), original_effect);
    let work:(i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='context/model-visible-applied'),(SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='tool/call-started')")
        .bind(journal.0.session.as_ref()).bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(work,(0,1),"only the originally admitted tool exists; a terminal retry never dispatches model/tool work");
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
