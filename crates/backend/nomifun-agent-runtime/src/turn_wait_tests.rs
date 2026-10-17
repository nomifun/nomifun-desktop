#[derive(Debug, Default)]
struct ParkTestPort {
    entered: Notify,
    wake: Notify,
    observation: std::sync::Mutex<Option<crate::AgentOwnedProcessObservation>>,
    calls: AtomicUsize,
}

#[async_trait]
impl crate::AgentWaitPort for ParkTestPort {
    async fn wait(&self, _: &ChatCausality, request: &crate::AgentWaitRequest, cancellation: CancellationToken)
        -> Result<crate::AgentWaitWake, AgentEngineError> {
        assert_eq!(request.processes.len(), 1);
        assert_eq!(request.processes[0].process_id, "owned");
        assert_eq!(request.processes[0].output_cursor, 5);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        tokio::select! { biased;
            _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
            _ = self.wake.notified() => {}
        }
        Ok(crate::AgentWaitWake::ProcessesChanged(vec![self.observation.lock().unwrap().take().unwrap()]))
    }
}

fn wait_ledger_step() -> Vec<Result<ChatModelEvent, ChatModelError>> {
    let mut step = control_step("inspect-a", "read_file", json!({"path":"a"}));
    step.insert(1, Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall { call_id: "inspect-b".into(), name: "read_file".into(),
        arguments: StrictJsonValue(json!({"path":"b"})), provider_metadata: None } }));
    step
}

fn parked_timeout() -> crate::AgentOwnedProcessObservation {
    crate::AgentOwnedProcessObservation { process_id: "owned".into(), poll: nomifun_engine_core::EngineProcessPoll::TimedOut {
        output: nomifun_engine_core::EngineProcessOutput { text: "Authorization expired".into(), next_cursor: 26,
            retained_bytes: 26, dropped_bytes: 0, source_encoding: "utf-8".into(), decode_errors: 0 },
        cleanup: nomifun_engine_core::EngineCleanupReport { interrupt_attempted: true, terminate_attempted: false,
            force_kill_attempted: false, reaped: true, elapsed_ms: 1, errors: Vec::new() },
    } }
}

#[tokio::test]
async fn live_user_action_wait_stops_sampling_and_timeout_keeps_task_paused() {
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(vec![
        wait_ledger_step(), plan_step("in_progress", "inspect"),
        control_step("launch", "start_process", json!({"command":"helper", "wait_ms":0})),
        input_step("authorization"),
        control_step("blocked", "report_completion", json!({"summary":"The installation remains available; authorization must be restarted by the user.",
            "observed_command_failure_count":1,
            "criteria":[{"disposition":"blocked","rationale":"The original authorization process expired while waiting for the user."}]})),
        text_step("must not sample again"),
    ]) });
    let journal = Arc::new(SettlementJournal::default());
    let wait = Arc::new(ParkTestPort::default());
    let tools = Arc::new(SettlementTool::default());
    let token = CancellationToken::new();
    let task = tokio::spawn(run_turn(binding(), model.clone(), tools.clone(), journal.clone(),
        input_request(settlement_tool_plan()).with_wait_port(wait.clone()), AgentContextBudget::default(), token));
    tokio::time::timeout(Duration::from_secs(2), wait.entered.notified()).await.unwrap();
    assert_eq!(model.requests.lock().unwrap().len(), 4, "parking must stop model sampling");
    assert!(!task.is_finished(), "waiting must not return a terminal to SDK cleanup");
    assert!(journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnWaiting {
        request_id: Some(id), reason: nomifun_agent_contracts::AgentWaitReason::UserInput, .. } if id == "authorization")));
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event,
        AgentEngineEvent::TurnFailed { .. } | AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::TurnPaused { .. })));
    *wait.observation.lock().unwrap() = Some(parked_timeout());
    wait.wake.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_DEPENDENCY_BLOCKED"));
    assert_eq!(model.requests.lock().unwrap().len(), 5);
    let events = journal.events.lock().unwrap();
    let work = events.iter().rev().find_map(|event| match event { AgentEngineEvent::WorkStatus { status } => Some(status), _ => None }).unwrap();
    assert!(work.running_processes.is_empty());
    assert_eq!(work.failed_commands, 1);
    assert_eq!(work.failed_tools, 0, "a host observation is not a failed tool call");
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. })));
    assert!(crate::user_input_request::recorded_pending(&events).unwrap().is_none());
    assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::OwnedProcessObserved { .. })));
    assert_eq!(tools.calls.lock().unwrap().iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>(), ["read_file", "read_file", "start_process"], "no replay, cancellation or synthetic poll");
    assert!(journal.checkpoints.lock().unwrap().iter().all(|checkpoint| checkpoint.work.running_processes.is_empty()));
}

#[tokio::test]
async fn ordinary_stop_with_open_plan_pauses_for_intervention_without_failure() {
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(vec![
        wait_ledger_step(), plan_step("in_progress", "inspect"), text_step("This task needs another step."), text_step("Remaining work is not done."),
        text_step("must not sample again"),
    ]) });
    let journal = Arc::new(SettlementJournal::default());
    let result = run_turn(binding(), model.clone(), Arc::new(EchoTool), journal.clone(), input_request(tool_plan()),
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_INTERVENTION_REQUIRED"));
    assert_eq!(model.requests.lock().unwrap().len(), 4);
    assert!(journal.checkpoints.lock().unwrap().last().unwrap().plan.is_open());
    assert_eq!(journal.checkpoints.lock().unwrap().last().unwrap().completion_review.phase, AgentExecutionPhase::EndIntentReview,
        "the one declaration opportunity survives native recovery without a second flag");
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. })));
}

#[tokio::test]
async fn live_stop_without_wait_intent_parks_for_intervention_and_cancels_exactly() {
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(vec![
        wait_ledger_step(), plan_step("in_progress", "inspect"), control_step("launch", "start_process", json!({"command":"helper"})),
        text_step("Progress."), text_step("Still unfinished."), text_step("must not sample again"),
    ]) });
    let journal = Arc::new(SettlementJournal::default());
    let wait = Arc::new(ParkTestPort::default());
    let token = CancellationToken::new();
    let task = tokio::spawn(run_turn(binding(), model.clone(), Arc::new(SettlementTool::default()), journal.clone(),
        input_request(settlement_tool_plan()).with_wait_port(wait.clone()), AgentContextBudget::default(), token.clone()));
    tokio::time::timeout(Duration::from_secs(2), wait.entered.notified()).await.unwrap();
    assert_eq!(model.requests.lock().unwrap().len(), 5);
    assert!(journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnWaiting {
        request_id: None, reason: nomifun_agent_contracts::AgentWaitReason::InterventionRequired, .. })));
    token.cancel();
    assert!(matches!(tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap(), Err(AgentEngineError::Cancelled)));
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. })));
}

struct ParkBoundaryJournal { journal: SettlementJournal, input: Arc<OneSteer> }
impl std::fmt::Debug for ParkBoundaryJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("ParkBoundaryJournal") }
}
#[async_trait]
impl AgentEventSink for ParkBoundaryJournal {
    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        if matches!(event, AgentEngineEvent::TurnWaiting { .. }) {
            *self.input.input.lock().unwrap() = Some(crate::AgentSteeringInput { receipt_operation_id: "authorize-reply".into(),
                message_id: "authorize-reply-message".into(), text: "Authorization is complete; continue the original task.".into(),
                files: Vec::new(), inject_skills: Vec::new(), image_count: 0, prepared_images: Vec::new(), prepared_skill_instructions: Vec::new() });
        }
        self.journal.emit(event).await
    }
}

#[tokio::test]
async fn accepted_reply_at_park_publication_boundary_is_consumed_before_waiting() {
    let input = Arc::new(OneSteer { input: Default::default() });
    let journal = Arc::new(ParkBoundaryJournal { journal: SettlementJournal::default(), input: input.clone() });
    let wait = Arc::new(ParkTestPort::default());
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(vec![
        wait_ledger_step(), plan_step("in_progress", "inspect"), control_step("launch", "start_process", json!({"command":"helper"})), input_step("authorization"),
        control_step("poll", "poll_process", json!({"process_id":"owned", "cursor":5})),
        plan_step("completed", "inspect"),
        control_step("done", "report_completion", json!({"summary":"The original authorization helper exited successfully.",
            "criteria":[{"disposition":"supported","evidence_call_ids":["poll"],"rationale":"The original process owner returned exit zero and proved cleanup."}]})),
    ]) });
    let result = run_turn(binding(), model.clone(), Arc::new(SettlementTool::default()), journal.clone(),
        input_request(settlement_tool_plan()).with_wait_port(wait.clone()).with_input_port(input),
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert_eq!(wait.calls.load(Ordering::SeqCst), 0, "an already accepted answer must not be stranded in a wait");
    assert!(model.requests.lock().unwrap()[4].input.messages.iter().flat_map(|message| &message.content).any(|part|
        matches!(part, ChatContentPart::Text { text } if text == "Authorization is complete; continue the original task.")));
}

#[tokio::test]
async fn owner_success_wakes_original_turn_then_real_poll_supplies_completion_evidence() {
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(vec![
        wait_ledger_step(), plan_step("in_progress", "inspect"),
        control_step("launch", "start_process", json!({"command":"helper", "wait_ms":0})),
        input_step("authorization"),
        control_step("original-poll", "poll_process", json!({"process_id":"owned", "cursor":5})),
        control_step("done", "report_completion", json!({"summary":"The original helper exited successfully after authorization.",
            "criteria":[{"disposition":"supported","evidence_call_ids":["original-poll"],"rationale":"The real original-owner poll returned exit zero and reaped cleanup."}]})),
    ]) });
    let journal = Arc::new(SettlementJournal::default());
    let wait = Arc::new(ParkTestPort::default());
    let tools = Arc::new(SettlementTool::default());
    let task = tokio::spawn(run_turn(binding(), model.clone(), tools.clone(), journal.clone(),
        input_request(settlement_tool_plan()).with_wait_port(wait.clone()), AgentContextBudget::default(), CancellationToken::new()));
    tokio::time::timeout(Duration::from_secs(2), wait.entered.notified()).await.unwrap();
    let expired = parked_timeout();
    let nomifun_engine_core::EngineProcessPoll::TimedOut { output, cleanup } = expired.poll else { unreachable!() };
    *wait.observation.lock().unwrap() = Some(crate::AgentOwnedProcessObservation { process_id: "owned".into(),
        poll: nomifun_engine_core::EngineProcessPoll::Exited { exit_code: Some(0), signal: None, output, cleanup } });
    wait.wake.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert_eq!(tools.calls.lock().unwrap().iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>(), ["read_file", "read_file", "start_process", "poll_process"]);
    let events = journal.events.lock().unwrap();
    assert!(crate::user_input_request::recorded_pending(&events).unwrap().is_none(), "owner-first wake retires the source request without inventing an answer");
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. })));
    let work = events.iter().rev().find_map(|event| match event { AgentEngineEvent::WorkStatus { status } => Some(status), _ => None }).unwrap();
    assert_eq!(work.successful_commands, 1);
    assert_eq!(work.failed_commands, 0);
    let mut replayed = Vec::new();
    crate::replay_closed_turn(&mut replayed, request().input.messages[0].clone(), &events).unwrap();
    let encoded = serde_json::to_string(&replayed).unwrap();
    assert!(encoded.contains("original host process owner observation") || encoded.contains("Original host process owner observation"));
    assert!(encoded.contains("The original helper exited successfully"));
}

#[tokio::test]
async fn successful_owner_then_four_invalid_completion_reports_pauses_without_task_failure() {
    let mut steps = vec![wait_ledger_step(), plan_step("in_progress", "inspect"),
        control_step("launch", "start_process", json!({"command":"helper", "wait_ms":0})), input_step("authorization"),
        control_step("original-poll", "poll_process", json!({"process_id":"owned", "cursor":5})), plan_step("completed", "inspect")];
    for index in 0..4 {
        steps.push(control_step(&format!("invalid-report-{index}"), "report_completion", json!({"summary":"The original helper exited successfully."})));
    }
    steps.push(text_step("must not sample after declaration budget exhaustion"));
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(steps) });
    let journal = Arc::new(SettlementJournal::default());
    let wait = Arc::new(ParkTestPort::default());
    let tools = Arc::new(SettlementTool::default());
    let task = tokio::spawn(run_turn(binding(), model.clone(), tools.clone(), journal.clone(),
        input_request(settlement_tool_plan()).with_wait_port(wait.clone()), AgentContextBudget::default(), CancellationToken::new()));
    tokio::time::timeout(Duration::from_secs(2), wait.entered.notified()).await.unwrap();
    let nomifun_engine_core::EngineProcessPoll::TimedOut { mut output, cleanup } = parked_timeout().poll else { unreachable!() };
    output.text = "Authorization completed".into();
    *wait.observation.lock().unwrap() = Some(crate::AgentOwnedProcessObservation { process_id: "owned".into(),
        poll: nomifun_engine_core::EngineProcessPoll::Exited { exit_code: Some(0), signal: None, output, cleanup } });
    wait.wake.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_INTERVENTION_REQUIRED"));
    assert_eq!(model.requests.lock().unwrap().len(), 10);
    assert_eq!(model.steps.lock().unwrap().len(), 1);
    let events = journal.events.lock().unwrap();
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. } | AgentEngineEvent::TurnCompleted { .. })));
    let checkpoint = journal.checkpoints.lock().unwrap().last().cloned().unwrap();
    assert_eq!(checkpoint.work.successful_commands, 1);
    assert_eq!(checkpoint.work.failed_commands, 0);
    assert_eq!(checkpoint.work.failed_tools, 4, "real declaration refusals stay recorded without becoming a task failure");
    assert_eq!(checkpoint.control_rejections.consecutive, 4);
    assert!(checkpoint.work.running_processes.is_empty());
    assert!(checkpoint.completion_review.is_report_only(), "a paused declaration cannot reopen the action surface");
    checkpoint.validate().unwrap();
    assert_eq!(tools.calls.lock().unwrap().iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>(),
        ["read_file", "read_file", "start_process", "poll_process"], "no completed work, process or owner result may be replayed to repair a report");
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::ToolCompleted { result, .. }
        if result.call_id.as_ref().starts_with("invalid-report-") && result.is_error)).count(), 4);
}

#[tokio::test]
async fn exhausted_completion_controls_keep_live_owner_until_reaped_then_pause_without_sampling() {
    let mut steps = vec![wait_ledger_step(), plan_step("in_progress", "inspect"),
        control_step("launch", "start_process", json!({"command":"helper", "wait_ms":0}))];
    for index in 0..4 { steps.push(control_step(&format!("invalid-live-report-{index}"), "report_completion", json!({"summary":"Not a valid account"}))); }
    steps.push(text_step("owner exit is not another automatic correction grant"));
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(steps) });
    let journal = Arc::new(SettlementJournal::default());
    let wait = Arc::new(ParkTestPort::default());
    let tools = Arc::new(SettlementTool::default());
    let task = tokio::spawn(run_turn(binding(), model.clone(), tools.clone(), journal.clone(),
        input_request(settlement_tool_plan()).with_wait_port(wait.clone()), AgentContextBudget::default(), CancellationToken::new()));
    tokio::time::timeout(Duration::from_secs(2), wait.entered.notified()).await.unwrap();
    assert_eq!(model.requests.lock().unwrap().len(), 7);
    assert!(!task.is_finished(), "control budget exhaustion cannot return to SDK cleanup while the original owner is live");
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. } | AgentEngineEvent::TurnPaused { .. })));
    let nomifun_engine_core::EngineProcessPoll::TimedOut { output, cleanup } = parked_timeout().poll else { unreachable!() };
    *wait.observation.lock().unwrap() = Some(crate::AgentOwnedProcessObservation { process_id: "owned".into(),
        poll: nomifun_engine_core::EngineProcessPoll::Exited { exit_code: Some(0), signal: None, output, cleanup } });
    wait.wake.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_INTERVENTION_REQUIRED"));
    assert_eq!(model.requests.lock().unwrap().len(), 7, "a lifecycle wake must not renew an exhausted model-control budget");
    let checkpoint = journal.checkpoints.lock().unwrap().last().cloned().unwrap();
    assert_eq!(checkpoint.work.failed_tools, 4);
    assert_eq!(checkpoint.work.successful_commands, 1);
    assert_eq!(checkpoint.work.failed_commands, 0);
    assert_eq!(tools.calls.lock().unwrap().len(), 3);
    assert!(checkpoint.control_rejections.is_exhausted());
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. })));
}

#[tokio::test]
async fn control_exhaustion_after_deferred_window_preserves_checkpoint_and_recoverable_owner_tail() {
    let mut steps = vec![wait_ledger_step(), plan_step("in_progress", "inspect"),
        control_step("launch", "start_process", json!({"command":"helper", "wait_ms":0}))];
    for index in 0..4 { steps.push(control_step(&format!("closed-live-report-{index}"), "report_completion", json!({"summary":"Not a valid account"}))); }
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(steps) });
    let journal = Arc::new(SettlementJournal::default());
    let wait = Arc::new(ParkTestPort::default());
    let tools = Arc::new(SettlementTool::default());
    let request = input_request(settlement_tool_plan()).with_wait_port(wait.clone()).with_max_model_steps(1)
        .with_execution_segments(crate::AgentSegmentPolicy { max_segments:16, max_no_progress_segments:16 });
    let task = tokio::spawn(run_turn(binding(), model.clone(), tools.clone(), journal.clone(), request,
        AgentContextBudget::default(), CancellationToken::new()));
    tokio::time::timeout(Duration::from_secs(2), wait.entered.notified()).await.unwrap();
    let nomifun_engine_core::EngineProcessPoll::TimedOut { output, cleanup } = parked_timeout().poll else { unreachable!() };
    *wait.observation.lock().unwrap() = Some(crate::AgentOwnedProcessObservation { process_id: "owned".into(),
        poll: nomifun_engine_core::EngineProcessPoll::Exited { exit_code: Some(0), signal: None, output, cleanup } });
    wait.wake.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_INTERVENTION_REQUIRED"));
    let checkpoint = journal.checkpoints.lock().unwrap().last().cloned().unwrap();
    assert!(result.model_steps > checkpoint.segments.as_ref().unwrap().window_end());
    assert!(checkpoint.work.running_processes.is_empty(), "no live resource is serialized as portable state");
    let events = journal.events.lock().unwrap();
    let boundary = events.iter().rposition(|event| matches!(event, AgentEngineEvent::ExecutionCheckpointSaved { .. })).unwrap();
    let revision = match events[boundary] { AgentEngineEvent::ExecutionCheckpointSaved { revision, .. } => revision, _ => unreachable!() };
    let tail: Vec<_> = events[boundary+1..].iter().filter(|event| !matches!(event, AgentEngineEvent::TurnPaused { .. })).cloned().collect();
    let recovered = crate::reconcile_execution_tail(&checkpoint, revision, &tail, &Default::default(), &Default::default(), &Default::default()).unwrap();
    recovered.checkpoint.validate().unwrap();
    assert_eq!(recovered.checkpoint.model_steps, result.model_steps);
    assert_eq!(recovered.checkpoint.work.failed_tools, 4);
    assert_eq!(recovered.checkpoint.work.successful_commands, 1);
    assert_eq!(recovered.checkpoint.work.failed_commands, 0);
    assert!(recovered.checkpoint.work.running_processes.is_empty());
    assert_eq!(recovered.checkpoint.control_rejections.consecutive, 4);
    assert!(!recovered.observations.iter().any(|event| matches!(event, AgentEngineEvent::ToolStarted { .. } | AgentEngineEvent::ToolCallCompleted { .. })));
    assert_eq!(tools.calls.lock().unwrap().len(), 3, "only original two reads and the one original start ran");
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. })));
}

struct RecordedResumeInput {
    input: std::sync::Mutex<Option<crate::AgentSteeringInput>>,
    journal: Arc<SettlementJournal>,
}
impl std::fmt::Debug for RecordedResumeInput {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str("RecordedResumeInput")}
}

#[async_trait]
impl crate::AgentInputPort for RecordedResumeInput {
    async fn take(&self,_:&ChatCausality,_:bool)->Result<Vec<crate::AgentSteeringInput>,AgentEngineError> {
        let inputs=self.input.lock().unwrap().take().into_iter().collect::<Vec<_>>();
        if !inputs.is_empty() {self.journal.emit(AgentEngineEvent::SteeringInputs {inputs:inputs.clone()}).await?;}
        Ok(inputs)
    }
    async fn has_pending(&self,_:&ChatCausality)->Result<bool,AgentEngineError> {Ok(self.input.lock().unwrap().is_some())}
}

#[tokio::test]
async fn fourth_phase_refusal_only_in_tail_restores_exhaustion_without_sampling_and_real_input_reopens_it() {
    let mut steps=vec![wait_ledger_step(),plan_step("in_progress","inspect"),
        control_step("original-start","start_process",json!({"command":"helper","wait_ms":0})),
        control_step("original-poll","poll_process",json!({"process_id":"owned","cursor":5})),
        plan_step("completed","inspect"),
        control_step("malformed-account","report_completion",json!({"summary":"The original helper exited successfully"}))];
    for index in 0..4 {
        let mut step=control_step(&format!("forbidden-read-{index}"),"read_file",json!({"path":"a"}));
        if index==3 {
            step.insert(1,Ok(ChatModelEvent::ToolCallCompleted {call:ChatToolCall {call_id:"same-rejected-batch".into(),name:"read_file".into(),
                arguments:StrictJsonValue(json!({"path":"b"})),provider_metadata:None}}));
        }
        steps.push(step);
    }
    let source_model=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(steps)});
    let source=Arc::new(SettlementJournal::default());let original_tools=Arc::new(SettlementTool::default());
    let original_request=input_request(settlement_tool_plan());
    let result=run_turn(binding(),source_model,original_tools.clone(),source.clone(),original_request.clone(),AgentContextBudget::default(),CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Paused {..}));
    let snapshots=source.checkpoints.lock().unwrap().clone();
    let (checkpoint_index,checkpoint)=snapshots.iter().enumerate().rev().find(|(_,checkpoint)|checkpoint.control_rejections.phase_corrections==3).unwrap();
    let revision=checkpoint_index as u64+1;
    let events=source.events.lock().unwrap().clone();
    let boundary=events.iter().position(|event|matches!(event,AgentEngineEvent::ExecutionCheckpointSaved {revision:found,..} if *found==revision)).unwrap();
    let next_boundary=events[boundary+1..].iter().position(|event|matches!(event,AgentEngineEvent::ExecutionCheckpointSaved {..})).map(|offset|boundary+1+offset).unwrap();
    let tail=&events[boundary+1..next_boundary];
    assert_eq!(tail.iter().filter(|event|matches!(event,AgentEngineEvent::ToolCompleted {runtime_hold:Some(crate::RuntimeToolHoldReason::PhaseRestricted),..})).count(),2,
        "the fourth rejected batch deliberately has two calls");
    let reconciled=crate::reconcile_execution_tail(checkpoint,revision,tail,&Default::default(),&Default::default(),&Default::default()).unwrap();
    assert_eq!(reconciled.checkpoint.control_rejections.phase_corrections,4,"one batch spends one correction, not zero or two");
    assert_eq!(reconciled.checkpoint.work.failed_tools,checkpoint.work.failed_tools,"phase feedback must not become failed work");
    let retry=crate::reconcile_execution_tail(checkpoint,revision,tail,&Default::default(),&Default::default(),
        &nomifun_agent_contracts::NativeBudgetIncrease {retry_stall_guards:true,..Default::default()}).unwrap();
    assert!(!retry.checkpoint.control_rejections.is_exhausted(),"only the explicit owner retry grant resets recovery guards");
    let mut prefix=events[..next_boundary].to_vec();prefix.extend(reconciled.observations);
    let resumed_revision=revision+1;
    prefix.push(AgentEngineEvent::ExecutionCheckpointSaved {step:reconciled.checkpoint.model_steps,revision:resumed_revision,
        digest:nomifun_agent_contracts::digest_payload(&reconciled.checkpoint).unwrap()});
    let recovery=crate::AgentTurnRecovery::new(reconciled.checkpoint,resumed_revision,1,prefix,vec![],vec![]).unwrap();
    let no_sampling=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(vec![text_step("must not sample")])});
    let tools=Arc::new(SettlementTool::default());let journal=Arc::new(SettlementJournal::default());
    let result=run_turn(binding(),no_sampling.clone(),tools.clone(),journal,original_request.clone().with_recovery(recovery.clone()),AgentContextBudget::default(),CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Paused {reason} if reason=="EXECUTION_INTERVENTION_REQUIRED"));
    assert!(no_sampling.requests.lock().unwrap().is_empty());assert!(tools.calls.lock().unwrap().is_empty());
    let journal=Arc::new(SettlementJournal::default());
    let input=Arc::new(RecordedResumeInput {input:std::sync::Mutex::new(Some(crate::AgentSteeringInput {
        receipt_operation_id:"new-owner-input".into(),message_id:"new-owner-message".into(),text:"Ask me for the missing new input before continuing".into(),
        files:vec![],inject_skills:vec![],image_count:0,prepared_images:vec![],prepared_skill_instructions:vec![],
    })),journal:journal.clone()});
    let model=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(vec![input_step("new-source-question")])});
    let result=run_turn(binding(),model.clone(),tools.clone(),journal.clone(),original_request.with_recovery(recovery).with_input_port(input),AgentContextBudget::default(),CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::InputRequired {..}));
    assert_eq!(model.requests.lock().unwrap().len(),1);assert!(tools.calls.lock().unwrap().is_empty());
    assert!(journal.events.lock().unwrap().iter().any(|event|matches!(event,AgentEngineEvent::SteeringInputs {inputs} if inputs[0].receipt_operation_id=="new-owner-input")));
}

#[tokio::test]
async fn live_tail_reconstructs_each_plan_steps_progress_without_resetting_idempotent_exhaustion() {
    let mut steps=vec![wait_ledger_step(),plan_step("in_progress","inspect"),
        control_step("plan-owner-start","start_process",json!({"command":"helper","wait_ms":0})),
        plan_step("blocked","inspect")];
    for index in 0..4 {steps.push(control_step(&format!("same-live-plan-{index}"),"update_plan",json!({
        "explanation":"The same original helper still needs attention","plan":[{"step":"requested operations","status":"blocked"}]})));}
    let model=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(steps)});
    let source=Arc::new(SettlementJournal::default());let tools=Arc::new(SettlementTool::default());let wait=Arc::new(ParkTestPort::default());
    let task=tokio::spawn(run_turn(binding(),model.clone(),tools.clone(),source.clone(),input_request(settlement_tool_plan()).with_wait_port(wait.clone()),
        AgentContextBudget::default(),CancellationToken::new()));
    tokio::time::timeout(Duration::from_secs(2),wait.entered.notified()).await.unwrap();
    assert_eq!(model.requests.lock().unwrap().len(),8,"one real plan change followed by four unchanged declarations exhausts the same live budget");
    let nomifun_engine_core::EngineProcessPoll::TimedOut {output,cleanup}=parked_timeout().poll else {unreachable!()};
    *wait.observation.lock().unwrap()=Some(crate::AgentOwnedProcessObservation {process_id:"owned".into(),
        poll:nomifun_engine_core::EngineProcessPoll::Exited {exit_code:Some(0),signal:None,output,cleanup}});
    wait.wake.notify_one();
    let result=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Paused {reason} if reason=="EXECUTION_INTERVENTION_REQUIRED"));
    let snapshots=source.checkpoints.lock().unwrap().clone();let final_checkpoint=snapshots.last().unwrap();
    assert_eq!(final_checkpoint.control_rejections.no_progress,4);assert_eq!(final_checkpoint.work.failed_tools,0);
    let (index,checkpoint)=snapshots.iter().enumerate().rev().find(|(_,checkpoint)|checkpoint.model_steps==2).unwrap();
    let revision=index as u64+1;let events=source.events.lock().unwrap().clone();
    let boundary=events.iter().position(|event|matches!(event,AgentEngineEvent::ExecutionCheckpointSaved {revision:found,..} if *found==revision)).unwrap();
    let final_boundary=events.iter().rposition(|event|matches!(event,AgentEngineEvent::ExecutionCheckpointSaved {..})).unwrap();
    let tail=&events[boundary+1..final_boundary];
    assert!(tail.iter().any(|event|matches!(event,AgentEngineEvent::PlanUpdated {plan} if plan.revision>checkpoint.plan.revision)));
    let recovered=crate::reconcile_execution_tail(checkpoint,revision,tail,&Default::default(),&Default::default(),&Default::default()).unwrap();
    assert_eq!(recovered.checkpoint.control_rejections.no_progress,4,"a real change earlier in the tail cannot make later idempotent model steps count as progress");
    assert_eq!(recovered.checkpoint.control_rejections,final_checkpoint.control_rejections);
    assert_eq!(recovered.checkpoint.work.failed_tools,0);assert_eq!(recovered.checkpoint.work.successful_commands,1);
    assert_eq!(tools.calls.lock().unwrap().len(),3,"reconciliation never repeats the original reads or helper launch");
}
