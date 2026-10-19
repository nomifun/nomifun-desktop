//! Installed background control retains the recording-time execution/native
//! target. The scripted provider is held; no physical playback is claimed.
use super::*;

const BACKGROUND_CORRECTION_INPUT: &str = "补充这个后台任务：报告只保存到本地，不要发送给任何人。";
const BACKGROUND_CORRECTION_PATH: &str = "background-corrected-report.txt";

pub(super) fn correction_step(
    body: &Value,
    state: &ModelState,
    user: &str,
) -> Option<(Value, &'static str)> {
    if !user.contains(BACKGROUND_CORRECTION_INPUT) {
        return None;
    }
    if tool_result(body, "background-correction-plan").is_none() {
        assert!(tool_available(body, "update_plan"),
            "the native Runtime exposes its plan recovery control after accepted steering");
        return Some(tool_call("update_plan", "background-correction-plan", json!({
            "explanation":"The accepted correction requires a local report. Preserve the original task and all accepted constraints; save and read back the report before reporting completion.",
            "plan":[{"step":"Save the report locally and verify its exact contents.","status":"in_progress"}]
        })));
    }
    let plan = tool_result(body, "background-correction-plan").unwrap();
    let planned: Value = serde_json::from_str(plan["content"].as_str().unwrap())
        .expect("the native plan control supplies its canonical JSON receipt");
    assert!(matches!(planned["status"].as_str(),Some("updated"|"unchanged")),
        "the accepted correction must settle its native replan before effects: {planned}");
    Some(if let Some(result) = tool_result(body, "background-correction-read") {
        let content = result["content"].as_str().unwrap();
        let observed: Value = serde_json::from_str(content).unwrap_or_else(|error|
            panic!("the original correction read must succeed with real workspace JSON: {content}; {error}"));
        assert_eq!(observed["content"], state.file_marker);
        assert_eq!(observed["path"], BACKGROUND_CORRECTION_PATH);
        assert_eq!(observed["eof"], true);
        assert!(tool_available(body, "report_completion"));
        tool_call("report_completion", "background-correction-complete", json!({
            "summary":format!("The report is saved locally with verified content {}.",state.file_marker),
            "criteria":[{"disposition":"supported","evidence_paths":[BACKGROUND_CORRECTION_PATH],
                "rationale":"The original workspace read returned the exact report bytes after the canonical correction's local write."}]
        }))
    } else if tool_result(body, "background-correction-write").is_some() {
        selected_tool(body, "read_file", "background-correction-read", "workspace.files/read",
            json!({"path":BACKGROUND_CORRECTION_PATH}))
    } else {
        selected_tool(body, "write_file", "background-correction-write", "workspace.files/write",
            json!({"path":BACKGROUND_CORRECTION_PATH,"content":state.file_marker}))
    })
}

async fn held_execution(
    flow: &PhysicalEntryFixture,
    server: &CompanionFlowModelServer,
    key: &str,
) -> nomifun_api_types::AgentExecutionDetail {
    send_and_complete_robot_turn(
        &flow.fixture,
        &flow.owner,
        &flow.request(key, BACKGROUND_INPUT),
        key,
    )
    .await;
    tokio::time::timeout(
        Duration::from_secs(20),
        server.state.held_entered.notified(),
    )
    .await
    .unwrap();
    flow.fixture
        .voice_execution
        .list(&flow.owner, Some(20), Some(0))
        .await
        .unwrap()
        .into_iter()
        .filter(|execution| !execution.status.is_terminal())
        .next()
        .map(|execution| execution.execution_id)
        .map(|id| async move {
            flow.fixture
                .voice_execution
                .get(&flow.owner, &id)
                .await
                .unwrap()
        })
        .unwrap()
        .await
}

async fn record_control(
    flow: &PhysicalEntryFixture,
    key: &str,
    text: &str,
) -> nomifun_robot::services::RobotTurnRequest {
    let mut control = flow.request(key, text);
    control.captured_context = Some(
        flow.fixture
            .owner
            .capture_robot_input_context(&flow.owner, &control)
            .await
            .unwrap(),
    );
    control
}

#[tokio::test]
async fn background_voice_cancel_uses_original_recording_target_and_never_replays_as_a_new_turn() {
    let server = CompanionFlowModelServer::with_held_work().await;
    let flow = PhysicalEntryFixture::new(&server, "background-spoken-cancel").await;
    let first = held_execution(&flow, &server, "background-cancel-first").await;
    let control = record_control(&flow, "spoken-cancel-one", "取消后台任务").await;
    let target = control
        .captured_context
        .as_ref()
        .unwrap()
        .background_work
        .as_ref()
        .unwrap();
    assert_eq!(target.execution_id, first.execution.execution_id);
    assert!(
        !control
            .captured_context
            .as_ref()
            .unwrap()
            .background_work_ambiguous
    );
    send_and_complete_robot_turn(
        &flow.fixture,
        &flow.owner,
        &flow.request("continue-while-held", FOLLOWUP_INPUT),
        "continue-while-held",
    )
    .await;
    let store = flow.fixture.owner.canonical().store();
    let turns_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
            .bind(flow.robot_id.as_ref())
            .fetch_one(flow.fixture.pool())
            .await
            .unwrap();
    flow.fixture
        .owner
        .cancel_robot_background_work(&flow.owner, &control, "spoken-cancel-one")
        .await
        .unwrap();
    assert_eq!(
        flow.fixture
            .voice_execution
            .get(&flow.owner, &target.execution_id)
            .await
            .unwrap()
            .execution
            .status,
        nomifun_common::AgentExecutionStatus::Cancelled
    );
    let events = store.read_events(&flow.robot_id, None, 1000).await.unwrap();
    let controls = events
        .events
        .iter()
        .filter(|event| {
            event.kind.0 == "message/user-accepted"
                && event
                    .idempotency_key
                    .as_ref()
                    .starts_with("robot-background-control:")
        })
        .collect::<Vec<_>>();
    assert_eq!(controls.len(), 1);
    assert!(!nomifun_agent_session::is_session_input_event(
        controls[0],
        &nomifun_agent_contracts::PrincipalRef {
            principal_kind: "user".into(),
            principal_id: flow.owner.clone()
        }
    ));
    assert!(
        store
            .pending_session_inputs(&flow.robot_id, None)
            .await
            .unwrap()
            .is_empty(),
        "control inputs are not an ordinary queue item"
    );
    let databases: Vec<(i64, String, String)> = sqlx::query_as("PRAGMA database_list")
        .fetch_all(flow.fixture.pool())
        .await
        .unwrap();
    let path = &databases
        .iter()
        .find(|(_, name, _)| name == "main")
        .unwrap()
        .2;
    assert!(
        !path.is_empty(),
        "this recovery check uses the installed fixture's persistent SQLite file"
    );
    let reopened_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(path)
                .read_only(true),
        )
        .await
        .unwrap();
    let reopened = nomifun_agent_session::AgentSessionStore::from_pool(reopened_pool.clone())
        .await
        .unwrap();
    assert!(
        reopened
            .pending_session_inputs(&flow.robot_id, None)
            .await
            .unwrap()
            .is_empty(),
        "a fresh persistent reader cannot reclaim a control as model work"
    );
    drop(reopened);
    reopened_pool.close().await;
    let payloads = store
        .resolved_event_payloads(&flow.robot_id, &[controls[0].event_id.clone()])
        .await
        .unwrap();
    let recorded = &payloads[controls[0].event_id.as_ref()];
    assert_eq!(recorded["content"], control.text);
    assert_eq!(
        recorded["background_control"]["target"]["execution_id"],
        target.execution_id
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
            .bind(flow.robot_id.as_ref())
            .fetch_one(flow.fixture.pool())
            .await
            .unwrap(),
        turns_before,
        "cancellation does not create a model Turn"
    );
    flow.fixture
        .owner
        .cancel_robot_background_work(&flow.owner, &control, "spoken-cancel-one")
        .await
        .unwrap();
    // Keep the cancelled provider's exact handler blocked. A shared Notify
    // permit could otherwise escape to the successor after its client leaves.
    let successor = held_execution(&flow, &server, "background-cancel-successor").await;
    assert_ne!(successor.execution.execution_id, target.execution_id);
    flow.fixture
        .owner
        .cancel_robot_background_work(&flow.owner, &control, "spoken-cancel-one")
        .await
        .unwrap();
    let still_running = flow
        .fixture
        .voice_execution
        .get(&flow.owner, &successor.execution.execution_id)
        .await
        .unwrap();
    assert_eq!(still_running.execution.version, successor.execution.version);
    assert_eq!(
        still_running.execution.status,
        nomifun_common::AgentExecutionStatus::Running,
        "a late replay cannot cancel a successor Execution"
    );
    send_and_complete_robot_turn(
        &flow.fixture,
        &flow.owner,
        &flow.request("after-control-talk", FOLLOWUP_INPUT),
        "after-control-talk",
    )
    .await;
    assert!(
        store
            .pending_session_inputs(&flow.robot_id, None)
            .await
            .unwrap()
            .is_empty()
    );
    let second_control = record_control(&flow, "cleanup-successor", "取消后台任务").await;
    flow.fixture
        .owner
        .cancel_robot_background_work(&flow.owner, &second_control, "cleanup-successor")
        .await
        .unwrap();
    server.state.held_release.notify_waiters();
    flow.finish().await;
    server.close().await;
}

#[tokio::test]
async fn pending_question_writer_lease_serializes_cancel_and_late_presentation_is_rejected() {
    let server = CompanionFlowModelServer::with_question_mode().await;
    let flow = PhysicalEntryFixture::new(&server, "question-writer-cancel-boundary").await;
    let robots = flow.fixture.services.robot.as_ref().unwrap();
    let mut playback = robots
        .registry
        .open_playback(&flow.device_id, &flow.connection)
        .await
        .unwrap();
    let input =
        "QUESTION_BACKGROUND_REQUEST: Run the isolated report and ask me for its missing name.";
    send_and_complete_robot_turn(
        &flow.fixture,
        &flow.owner,
        &flow.request("question-cancel-request", input),
        "question-cancel-request",
    )
    .await;
    let command = tokio::time::timeout(Duration::from_secs(25), playback.recv())
        .await
        .unwrap()
        .unwrap();
    let context = command.context.as_ref().unwrap();
    let authority = nomifun_robot::services::RobotQuestionPlaybackAuthority {
        source: command.source.as_ref().unwrap().clone(),
        presentation_id: command.question_presentation_id.as_ref().unwrap().clone(),
    };
    let mut output = flow.request("question-output-writer", &command.text);
    output.captured_context = Some(context.input_context.clone());
    let store = flow.fixture.owner.canonical().store();
    let payloads = store
        .resolved_event_payloads(
            &flow.robot_id,
            &[authority.source.message_id.clone().into()],
        )
        .await
        .unwrap();
    let notice = &payloads[&authority.source.message_id];
    let target: nomifun_voice_contracts::ApprovalTarget =
        serde_json::from_value(notice["decision"]["target"].clone()).unwrap();
    let execution_id = target.execution_id.clone();
    let proofs = || {
        sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='robot/question-playback-accepted'")
        .bind(flow.robot_id.as_ref()).fetch_one(flow.fixture.pool())
    };
    assert_eq!(proofs().await.unwrap(), 0);
    command.accepted.send(Ok(())).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let events = flow
                .fixture
                .voice_execution
                .events(&flow.owner, &execution_id, None, Some(128))
                .await
                .unwrap();
            if events
                .iter()
                .any(|event| event.payload["change"] == "lead_input_notice_reconciled")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        proofs().await.unwrap(),
        0,
        "a playback queue ACK is never presentation"
    );
    let current = flow
        .fixture
        .voice_execution
        .get(&flow.owner, &execution_id)
        .await
        .unwrap();
    assert_eq!(current.execution.decision_policy, nomifun_common::DecisionPolicy::Automatic,
        "a real canonical missing-input question is independent of automatic Agent decisions");
    assert!(current.execution.version >= target.expected_execution_version);
    let backend = robots.conversation_backend_for_test();
    // A damaged Execution event cannot turn another typed request with the
    // same question text into the child's actual canonical InputRequired.
    let original_request: String = sqlx::query_scalar(
        "SELECT payload FROM agent_execution_events WHERE execution_id=? AND sequence=? AND event_type='decision_requested'",
    )
    .bind(&execution_id)
    .bind(target.request_event_sequence)
    .fetch_one(flow.fixture.pool())
    .await
    .unwrap();
    let mut damaged_request: Value = serde_json::from_str(&original_request).unwrap();
    assert!(damaged_request["user_input_request"].is_object());
    damaged_request["user_input_request"]["request_id"] =
        "different-request-with-same-question".into();
    sqlx::query("UPDATE agent_execution_events SET payload=? WHERE execution_id=? AND sequence=?")
        .bind(serde_json::to_string(&damaged_request).unwrap())
        .bind(&execution_id)
        .bind(target.request_event_sequence)
        .execute(flow.fixture.pool())
        .await
        .unwrap();
    assert!(
        backend
            .acquire_output_context(&output, None, Some(&authority))
            .await
            .is_err(),
        "the full typed request must match the real child terminal, not only its question text"
    );
    sqlx::query("UPDATE agent_execution_events SET payload=? WHERE execution_id=? AND sequence=?")
        .bind(original_request)
        .bind(&execution_id)
        .bind(target.request_event_sequence)
        .execute(flow.fixture.pool())
        .await
        .unwrap();
    let (writer, mut received) = tokio::sync::mpsc::channel(1);
    let permit = writer.reserve().await.unwrap();
    let lease = backend
        .acquire_output_context(&output, None, Some(&authority))
        .await
        .unwrap()
        .expect("unchanged question survives outbox version advances");
    let engine = flow.fixture.voice_execution.clone();
    let owner = flow.owner.clone();
    let cancelled_id = execution_id.clone();
    let started = Arc::new(tokio::sync::Notify::new());
    let cancel_started = started.clone();
    let mut cancel = tokio::spawn(async move {
        cancel_started.notify_one();
        engine
            .cancel(
                &owner,
                &nomifun_common::AgentExecutionActor::user(&owner),
                &cancelled_id,
                nomifun_api_types::VersionedAgentExecutionCommand {
                    expected_version: current.execution.version,
                },
            )
            .await
    });
    started.notified().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(80), &mut cancel)
            .await
            .is_err(),
        "cancel cannot commit between pending validation and synchronous writer acceptance"
    );
    permit.send("the writer accepted this question frame");
    drop(lease);
    assert!(received.recv().await.is_some());
    let cancelled = tokio::time::timeout(Duration::from_secs(15), cancel)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        cancelled.execution.status,
        nomifun_common::AgentExecutionStatus::Cancelled
    );
    assert!(
        backend
            .acquire_output_context(&output, None, Some(&authority))
            .await
            .unwrap()
            .is_none(),
        "late PCM/text cannot play a cancelled question"
    );
    assert!(
        backend
            .question_presented(&output, &authority.source, &authority.presentation_id)
            .await
            .is_err(),
        "a callback delayed until after cancel cannot author proof"
    );
    assert_eq!(proofs().await.unwrap(), 0);
    flow.finish().await;
    server.close().await;
}

#[tokio::test]
async fn background_cancel_stale_cas_and_ambiguous_onset_do_not_refresh_the_target() {
    let server = CompanionFlowModelServer::with_held_work().await;
    let flow = PhysicalEntryFixture::new(&server, "background-cancel-stale").await;
    let original = held_execution(&flow, &server, "stale-background-first").await;
    let control = record_control(&flow, "stale-cancel-utterance", "取消任务").await;
    let frozen = control
        .captured_context
        .as_ref()
        .unwrap()
        .background_work
        .as_ref()
        .unwrap()
        .clone();
    let changed = flow
        .fixture
        .voice_execution
        .rename(
            &flow.owner,
            &nomifun_common::AgentExecutionActor::user(&flow.owner),
            &original.execution.execution_id,
            nomifun_api_types::RenameAgentExecutionRequest {
                goal: "The explicitly changed task keeps running.".into(),
                expected_version: frozen.expected_version,
            },
        )
        .await
        .unwrap();
    assert!(
        flow.fixture
            .owner
            .cancel_robot_background_work(&flow.owner, &control, "stale-cancel-utterance")
            .await
            .is_err()
    );
    let after = flow
        .fixture
        .voice_execution
        .get(&flow.owner, &frozen.execution_id)
        .await
        .unwrap();
    assert_eq!(after.execution.version, changed.version);
    assert_eq!(
        after.execution.status,
        nomifun_common::AgentExecutionStatus::Running
    );
    // Normal creation already refuses a second active lead aggregate. The
    // explicit ambiguous recording state must still be rejected at dispatch,
    // rather than falling back to whichever task currently appears newest.
    let mut ambiguous = control.clone();
    ambiguous.request_id = "ambiguous-cancel".into();
    ambiguous
        .captured_context
        .as_mut()
        .unwrap()
        .background_work_ambiguous = true;
    ambiguous.captured_context.as_mut().unwrap().background_work = None;
    assert!(
        flow.fixture
            .owner
            .cancel_robot_background_work(&flow.owner, &ambiguous, "ambiguous-cancel")
            .await
            .is_err()
    );
    let current = flow
        .fixture
        .voice_execution
        .get(&flow.owner, &frozen.execution_id)
        .await
        .unwrap();
    assert_eq!(
        current.execution.version, changed.version,
        "ambiguity does not mutate either candidate"
    );
    server.state.held_release.notify_waiters();
    flow.finish().await;
    server.close().await;
}

#[tokio::test]
async fn background_correction_keeps_real_input_in_exact_native_attempt_without_withdrawing_public_output()
 {
    let server = CompanionFlowModelServer::with_held_work().await;
    let flow = PhysicalEntryFixture::new(&server, "background-native-correction").await;
    let original = held_execution(&flow, &server, "correction-background").await;
    let control = record_control(
        &flow,
        "spoken-background-correction",
        BACKGROUND_CORRECTION_INPUT,
    )
    .await;
    let captured = control
        .captured_context
        .as_ref()
        .unwrap()
        .background_work
        .as_ref()
        .unwrap();
    let attempt = captured
        .attempt
        .as_ref()
        .expect("the recording captures the single running native Attempt");
    assert_eq!(captured.execution_id, original.execution.execution_id);
    let child = AgentSessionId::from(attempt.agent_session_id.clone());
    let operation = OperationId::from(attempt.turn_operation_id.clone());
    let store = flow.fixture.owner.canonical().store();
    flow.fixture
        .owner
        .supplement_robot_background_work(
            &flow.owner,
            &control,
            "spoken-background-correction",
            &control.text,
            true,
            "asr-correction-source",
        )
        .await
        .unwrap();
    let facts = store.turn_output_facts(&child, &operation).await.unwrap();
    let steering = facts
        .events
        .iter()
        .find(|event| {
            event.kind.0 == "turn/steer-accepted"
                && event.correlation_id.as_ref() == operation.as_ref()
        })
        .unwrap();
    let (_, input) = store
        .steering_input_source(&child, &steering.event_id)
        .await
        .unwrap();
    assert_eq!(
        input.0["content"], control.text,
        "actual canonical input, not a model-written replacement goal, carries the constraint"
    );
    assert_eq!(
        store
            .native_execution_generation(&child, &operation)
            .await
            .unwrap(),
        attempt.execution_generation
    );
    let child_head = store.head(&child).await.unwrap().last_seq;
    flow.fixture
        .owner
        .supplement_robot_background_work(
            &flow.owner,
            &control,
            "spoken-background-correction",
            &control.text,
            true,
            "asr-correction-source",
        )
        .await
        .unwrap();
    assert_eq!(
        store.head(&child).await.unwrap().last_seq,
        child_head,
        "exact input redelivery does not append a second constraint"
    );
    send_and_complete_robot_turn(
        &flow.fixture,
        &flow.owner,
        &flow.request("talk-with-corrected-background", FOLLOWUP_INPUT),
        "talk-with-corrected-background",
    )
    .await;
    server.state.held_release.notify_one();
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            if flow
                .fixture
                .voice_execution
                .get(&flow.owner, &captured.execution_id)
                .await
                .unwrap()
                .execution
                .status
                .is_terminal()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(flow.fixture.voice_execution.get(&flow.owner,&captured.execution_id).await.unwrap().execution.status,
        nomifun_common::AgentExecutionStatus::Completed);
    let complete = store.turn_output_facts(&child, &operation).await.unwrap();
    let child_session = store.get_live_session(&child).await.unwrap();
    let root = child_session.agent_binding.typed_resource_bindings.iter()
        .find(|binding|binding.resource_kind.0.as_str()=="workspace").unwrap().typed_parameters.get("workspace_root").unwrap();
    assert_eq!(std::fs::read_to_string(std::path::Path::new(root).join(BACKGROUND_CORRECTION_PATH)).unwrap(),server.state.file_marker);
    let effects = store.list_effects(&child).await.unwrap();
    let writes = effects.iter().filter(|effect|effect.turn_id==operation&&effect.action_id.as_ref()=="workspace.files/write").collect::<Vec<_>>();
    assert_eq!(writes.len(),1,"the original Attempt applies the local-only correction once");
    assert_eq!(writes[0].state,nomifun_agent_session::AgentEffectState::Returned);
    assert!(complete.event_payloads.values().any(|payload|matches!(
        serde_json::from_value::<nomifun_agent_runtime::AgentEngineEvent>(payload["event"].clone()),
        Ok(nomifun_agent_runtime::AgentEngineEvent::ToolCompleted {result,..})
            if result.call_id.as_ref()=="background-correction-read"&&!result.is_error)),
        "completion is grounded in the exact original Attempt's canonical read observation");
    assert!(
        complete
            .events
            .iter()
            .filter_map(|event| complete.event_payloads.get(event.event_id.as_ref()))
            .filter_map(|payload| payload.get("event"))
            .any(|event| event["event"] == "steering_inputs"
                && event["inputs"].to_string().contains(&control.text)),
        "the Runtime consumes the accepted input at its safe boundary"
    );
    let mut committed = std::collections::BTreeSet::new();
    for event in complete
        .events
        .iter()
        .filter_map(|event| complete.event_payloads.get(event.event_id.as_ref()))
        .filter_map(|payload| payload.get("event"))
    {
        if event["event"] == "public_output_committed" {
            committed.insert(event["step"].as_u64().unwrap());
        }
        if matches!(
            event["event"].as_str(),
            Some("voice_model_step_superseded" | "delivery_review_superseded")
        ) {
            assert!(
                !committed.contains(&event["step"].as_u64().unwrap()),
                "a background correction cannot retract irrevocable public output"
            );
        }
        assert_ne!(
            event["event"], "voice_model_step_superseded",
            "the background safe-boundary path does not opt into immediate model superseding"
        );
    }
    let source_before = store.read_events(&flow.robot_id, None, 500).await.unwrap().events;
    let child_before = store.read_events(&child, None, 500).await.unwrap().events;
    assert!(source_before.len() < 500 && child_before.len() < 500);
    assert!(
        store
            .pending_session_inputs(&flow.robot_id, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        flow.fixture
            .owner
            .supplement_robot_background_work(
                &flow.owner,
                &control,
                "late-new-correction",
                "different",
                true,
                "new-late-asr"
            )
            .await
            .is_err()
    );
    // Completion and its original report outbox are independent commits. The
    // report may arrive after the source Turn's terminal while this rejected
    // control is observed; it is not a new user input or a reopened task.
    let (source_after, payloads) = tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let events = store.read_events(&flow.robot_id, None, 500).await.unwrap().events;
            assert!(events.len() < 500);
            let ids = events.iter().map(|event| event.event_id.clone()).collect::<Vec<_>>();
            let payloads = store.resolved_event_payloads(&flow.robot_id, &ids).await.unwrap();
            if events.iter().any(|event| event.kind.0 == "message/assistant-projected"
                && payloads[event.event_id.as_ref()]["delegation_report"]["execution_id"] == captured.execution_id) {
                break (events, payloads);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("the original completed Execution directly delivers its own report");
    let reports = source_after.iter().filter(|event| event.kind.0 == "message/assistant-projected"
        && payloads[event.event_id.as_ref()]["delegation_report"]["execution_id"] == captured.execution_id).collect::<Vec<_>>();
    assert_eq!(reports.len(), 1, "the original completion report is published exactly once");
    let report_event = reports[0];
    let payload = &payloads[report_event.event_id.as_ref()];
    assert_eq!(report_event.producer_id.as_ref(), "session_api");
    assert_eq!(report_event.kind_version, 1);
    assert_eq!(payload["origin"], "agent_execution_report");
    let report: nomifun_agent_execution::AgentExecutionReport = serde_json::from_value(payload["delegation_report"].clone()).unwrap();
    assert_eq!(report.source, nomifun_agent_execution::AgentExecutionSource {
        agent_session_id:flow.robot_id.as_ref().to_owned(),turn_operation_id:captured.source_turn_operation_id.clone(),
        tool_operation_id:captured.source_tool_operation_id.clone(),effect_id:captured.source_effect_id.clone(),
    });
    assert_eq!(report.execution_id, captured.execution_id);
    assert_eq!(report.status, "completed");
    assert_eq!(payload["operation_id"], format!("exec-lead-report:{}:event:{}", report.execution_id, report.execution_event_sequence));
    let execution_events = flow.fixture.voice_execution.events(&flow.owner, &captured.execution_id, None, Some(128)).await.unwrap();
    assert!(execution_events.iter().any(|event| event.sequence == report.execution_event_sequence
        && event.event_type == nomifun_common::AgentExecutionEventKind::StatusChanged && event.payload["status"] == "completed"));
    let source_terminal = store.read_turn_receipt(&flow.robot_id, &captured.source_turn_operation_id.clone().into())
        .await.unwrap().terminal_event.unwrap();
    assert_eq!(report_event.causation_event_id.as_ref(), Some(&source_terminal.event_id));
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(report.attempts[0].step_id, attempt.step_id);
    assert_eq!(report.attempts[0].attempt_id, attempt.attempt_id);
    assert_eq!(report.attempts[0].agent_session_id, child.as_ref());
    assert_eq!(report.attempts[0].turn_operation_id, operation.as_ref());
    assert_eq!(report.attempts[0].terminal_event_id,
        store.read_turn_receipt(&child, &operation).await.unwrap().terminal_event.unwrap().event_id.as_ref());
    assert_eq!(report.attempts[0].result_ok, Some(true));
    assert!(report.attempts[0].output_integrity_ok);
    assert!(source_after.starts_with(&source_before), "previous canonical facts stay immutable");
    assert!(source_after[source_before.len()..].iter().all(|event| event.event_id == report_event.event_id),
        "the rejected expired control cannot append input, Turn, steer or public output; only the original report may arrive");
    let child_after = store.read_events(&child, None, 500).await.unwrap().events;
    let controls = |events: &[nomifun_agent_contracts::SessionEventRecord]| events.iter()
        .filter(|event| matches!(event.kind.0.as_str(), "message/user-accepted" | "turn/started" | "turn/steer-accepted"))
        .cloned().collect::<Vec<_>>();
    assert_eq!(controls(&child_after), controls(&child_before), "an expired correction neither steers nor reopens the original native task");
    let final_child = store.turn_output_facts(&child, &operation).await.unwrap();
    let public = |facts: &nomifun_agent_session::ChatCausalityFacts| facts.events.iter().filter(|event| {
        matches!(facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload| payload.pointer("/event/event")).and_then(Value::as_str),
            Some("public_output_committed" | "voice_model_step_superseded" | "delivery_review_superseded"))
    }).cloned().collect::<Vec<_>>();
    assert_eq!(public(&final_child), public(&complete), "rejection cannot publish or withdraw the original public output");
    assert!(store.pending_session_inputs(&flow.robot_id, None).await.unwrap().is_empty());
    flow.finish().await;
    server.close().await;
}
