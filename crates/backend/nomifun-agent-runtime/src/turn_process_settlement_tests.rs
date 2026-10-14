use nomifun_agent_contracts::StrictJsonValue;

#[derive(Default)]
struct SettlementJournal {
    events: std::sync::Mutex<Vec<AgentEngineEvent>>,
    checkpoints: std::sync::Mutex<Vec<crate::AgentExecutionCheckpoint>>,
    no_renewal_ack: bool,
    fail_renewal: bool,
    renew_while_running: bool,
    stop_after_steps: Option<(u16, crate::AgentExecutionStopReason)>,
}

#[async_trait]
impl AgentEventSink for SettlementJournal {
    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
    fn supports_checkpoints(&self) -> bool { true }
    async fn execution_pressure(&self) -> Result<crate::AgentExecutionPressure, AgentEngineError> {
        let events = self.events.lock().unwrap();
        let step = events.iter().filter_map(|event| match event {
            AgentEngineEvent::ModelStepStarted { step, .. } => Some(*step), _ => None,
        }).max().unwrap_or(0);
        let running = events.iter().rev().find_map(|event| match event {
            AgentEngineEvent::WorkStatus { status } => Some(!status.running_processes.is_empty()), _ => None,
        }).unwrap_or(false);
        Ok(crate::AgentExecutionPressure {
            renew_window: self.renew_while_running && running,
            stop: self.stop_after_steps.and_then(|(at, reason)| (step >= at).then_some(reason)),
        })
    }
    async fn save_checkpoint(&self, state: crate::AgentExecutionCheckpoint)
        -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
        state.validate()?;
        assert!(state.work.running_processes.is_empty(), "live handles must never be checkpointed");
        if state.segments.as_ref().is_some_and(|segments| segments.segment > 1) {
            if self.fail_renewal { return Err(AgentEngineError::EventSink("checkpoint write failed".into())); }
            if self.no_renewal_ack { return Ok(None); }
        }
        let mut checkpoints = self.checkpoints.lock().unwrap();
        let revision = checkpoints.len() as u64 + 1;
        let digest = nomifun_agent_contracts::digest_payload(&serde_json::to_value(&state).unwrap()).unwrap();
        let mut events = self.events.lock().unwrap();
        events.push(AgentEngineEvent::ExecutionCheckpointSaved { step: state.model_steps, revision, digest: digest.clone() });
        checkpoints.push(state);
        Ok(Some(crate::AgentCheckpointReceipt { revision, through_seq: events.len() as u64, digest }))
    }
}

#[derive(Default)]
struct SettlementTool {
    calls: std::sync::Mutex<Vec<(String, String)>>,
    forever_running: bool,
    unreaped: bool,
    steer: Option<Arc<OneSteer>>,
    block_input: Option<Arc<Notify>>,
}

#[async_trait]
impl AgentToolInvoker for SettlementTool {
    async fn invoke(&self, invocation: AgentToolInvocation, cancellation: CancellationToken)
        -> Result<AgentToolResult, AgentEngineError> {
        if let Some(result) = instruction_result(&invocation) { return Ok(result); }
        self.calls.lock().unwrap().push((invocation.call.call_id.as_ref().to_owned(), invocation.call.name.clone()));
        if invocation.call.name == "read_file" { return Ok(workspace_result(invocation)); }
        if invocation.call.name == "write_process_stdin" && let Some(entered) = &self.block_input {
            entered.notify_one();
            cancellation.cancelled().await;
            return Err(AgentEngineError::Cancelled);
        }
        if invocation.call.name == "start_process" && let Some(steer) = &self.steer {
            *steer.input.lock().unwrap() = Some(crate::AgentSteeringInput {
                receipt_operation_id: "owner-correction".into(), message_id: "steering-message".into(),
                text: "Stop the original helper before continuing.".into(), files: vec![], inject_skills: vec![],
                image_count: 0, prepared_images: vec![], prepared_skill_instructions: vec![],
            });
        }
        let terminal = invocation.call.name == "cancel_process"
            || (invocation.call.name == "poll_process" && !self.forever_running);
        let state = if terminal {
            if self.unreaped { "lost" } else if invocation.call.name == "cancel_process" { "cancelled" } else { "exited" }
        } else { "running" };
        let output = json!({"process_id":"owned", "state":state,
            "exit_code": if state == "exited" { Some(0) } else { None },
            "output":{"text":if terminal { "done" } else { "READY" }, "next_cursor":if terminal { 9 } else { 5 }},
            "cleanup":{"reaped":terminal && !self.unreaped, "errors":[]}});
        Ok(AgentToolResult::text(invocation.call.call_id, output.to_string(), self.unreaped && terminal))
    }
}

fn settlement_tool_plan() -> AgentToolPlan {
    AgentToolPlan::new([
        tool_binding("read_file", "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, true),
        tool_binding("write_file", "workspace.files", "workspace.files/write", AgentEffectClass::ManagedEffect, false),
        tool_binding("exec_command", "workspace.process", "workspace.process/exec", AgentEffectClass::ManagedEffect, false),
        tool_binding("start_process", "workspace.process", "workspace.process/start", AgentEffectClass::ManagedEffect, false),
        tool_binding("write_process_stdin", "workspace.process", "workspace.process/input", AgentEffectClass::ManagedEffect, false),
        tool_binding("close_process_stdin", "workspace.process", "workspace.process/close_stdin", AgentEffectClass::ManagedEffect, false),
        tool_binding("poll_process", "workspace.process", "workspace.process/poll", AgentEffectClass::ReadOnly, true),
        tool_binding("cancel_process", "workspace.process", "workspace.process/cancel", AgentEffectClass::ManagedEffect, false),
    ]).unwrap()
}

fn settlement_request(width: u16, windows: u16) -> AgentTurnRequest {
    AgentTurnRequest::new(request(), settlement_tool_plan(), principal(), 0)
        .with_max_model_steps(width)
        .with_execution_segments(crate::AgentSegmentPolicy { max_segments: windows, max_no_progress_segments: 2 })
}

fn settlement_steps() -> Vec<Vec<Result<ChatModelEvent, ChatModelError>>> {
    vec![
        control_step("launch", "start_process", json!({"command":"helper", "wait_ms":0})),
        control_step("stdin", "write_process_stdin", json!({"process_id":"owned", "input":"payload"})),
        control_step("eof", "close_process_stdin", json!({"process_id":"owned"})),
        control_step("poll", "poll_process", json!({"process_id":"owned", "cursor":5, "wait_ms":15000})),
        control_step("report", "report_completion", json!({"summary":"The original helper received its input and EOF, then exited successfully.",
            "criteria":[{"disposition":"supported", "evidence_call_ids":["poll"],
                "rationale":"The original owner terminal records the helper's exit 0, output and reaped tree."}]})),
        text_step("The original helper completed."),
    ]
}

#[tokio::test]
async fn process_boundary_finishes_original_stdin_eof_poll_before_checkpoint_renewal() {
    let model = Arc::new(ObservingModel { steps: std::sync::Mutex::new(settlement_steps()), requests: Default::default() });
    let tools = Arc::new(SettlementTool::default());
    let journal = Arc::new(SettlementJournal::default());
    let result = run_turn(binding(), model.clone(), tools.clone(), journal.clone(), settlement_request(1, 8),
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert_eq!(tools.calls.lock().unwrap().iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>(),
        ["start_process", "write_process_stdin", "close_process_stdin", "poll_process"]);
    let checkpoints = journal.checkpoints.lock().unwrap();
    assert!(checkpoints.iter().all(|checkpoint| checkpoint.work.running_processes.is_empty()));
    assert!(checkpoints.iter().any(|checkpoint| checkpoint.model_steps == 4
        && checkpoint.segments.as_ref().is_some_and(|segments| segments.segment == 2 && segments.segment_start_step == 4 && segments.total_model_limit() == 8)));
    let events = journal.events.lock().unwrap();
    let renewal = events.iter().position(|event| matches!(event,
        AgentEngineEvent::ExecutionSegmentRenewed { model_steps: 4, .. })).unwrap();
    assert!(matches!(&events[renewal - 1], AgentEngineEvent::ExecutionCheckpointSaved { step: 4, .. }));
    for request in &*model.requests.lock().unwrap() {
        assert_eq!(request.causality.agent_session_id, binding().agent_session_id().clone());
        assert_eq!(request.causality.turn_operation_id.as_ref(), "turn");
        assert_eq!(request.causality.resolved_snapshot_ref, binding().resolved_snapshot_ref().clone());
    }
}

#[tokio::test]
async fn process_boundary_reproduces_the_real_coding_step_64_without_more_budget() {
    let mut steps = (0..63).map(|index| control_step(&format!("read-{index}"), "read_file", json!({"path":"a"}))).collect::<Vec<_>>();
    steps.extend(settlement_steps());
    let model = Arc::new(ObservingModel { steps: std::sync::Mutex::new(steps), requests: Default::default() });
    let journal = Arc::new(SettlementJournal::default());
    let request = AgentTurnRequest::new(request(), settlement_tool_plan(), principal(), 0)
        .with_execution_segments(crate::AgentSegmentPolicy::default());
    assert_eq!(request.max_model_steps, 64);
    let result = run_turn(binding(), model, Arc::new(SettlementTool::default()), journal.clone(), request,
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert!(journal.checkpoints.lock().unwrap().iter().any(|checkpoint| checkpoint.model_steps == 67
        && checkpoint.segments.as_ref().is_some_and(|segments| segments.segment == 2 && segments.total_model_limit() == 1024)));
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnPaused { .. })));
}

#[tokio::test]
async fn process_boundary_holds_foreign_handles_and_new_work_as_one_unexecuted_batch() {
    let mixed = vec![
        Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall { call_id:"held-poll".into(), name:"poll_process".into(),
            arguments:StrictJsonValue(json!({"process_id":"owned"})), provider_metadata:None } }),
        Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall { call_id:"held-write".into(), name:"write_file".into(),
            arguments:StrictJsonValue(json!({"path":"new-work", "content":"forbidden"})), provider_metadata:None } }),
        Ok(ChatModelEvent::Completed { finish_reason:ChatFinishReason::ToolCalls }),
    ];
    let mut steps = vec![control_step("read", "read_file", json!({"path":"a"})), settlement_steps()[0].clone(), mixed,
        control_step("foreign", "poll_process", json!({"process_id":"foreign"}))];
    steps.extend(settlement_steps()[1..].iter().cloned());
    let report = steps[7].iter_mut().find_map(|event| match event { Ok(ChatModelEvent::ToolCallCompleted { call }) => Some(call), _ => None }).unwrap();
    report.arguments.0["observed_tool_error_count"] = json!(3);
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(steps), requests:Default::default() });
    let tools = Arc::new(SettlementTool::default());
    let journal = Arc::new(SettlementJournal::default());
    let result = run_turn(binding(), model.clone(), tools.clone(), journal.clone(), settlement_request(2, 8),
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert!(!tools.calls.lock().unwrap().iter().any(|(id, _)| matches!(id.as_str(), "held-poll" | "held-write" | "foreign")));
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::ToolStarted { call_id, .. }
        if matches!(call_id.as_ref(), "held-poll" | "held-write" | "foreign"))));
    assert!(model.requests.lock().unwrap()[2].input.tools.iter().all(|tool|
        !matches!(tool.name.as_str(), "start_process" | "exec_command" | "write_file" | "report_completion")));
}

#[tokio::test]
async fn process_boundary_allows_authorized_input_reads_without_counting_them_as_settlement_progress() {
    let mut steps = settlement_steps();
    steps.insert(1, control_step("read-stdin", "read_file", json!({"path":"a"})));
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(steps), requests:Default::default() });
    let tools = Arc::new(SettlementTool::default());
    let result = run_turn(binding(), model, tools.clone(), Arc::new(SettlementJournal::default()), settlement_request(1, 8),
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert!(tools.calls.lock().unwrap().iter().any(|(id, _)| id == "read-stdin"));
}

#[tokio::test]
async fn process_boundary_empty_replayed_polls_do_not_buy_more_windows() {
    let mut steps = vec![settlement_steps()[0].clone()];
    steps.extend((0..5).map(|index| control_step(&format!("empty-{index}"), "poll_process",
        json!({"process_id":"owned", "cursor":index, "wait_ms":index + 1}))));
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(steps), requests:Default::default() });
    let journal = Arc::new(SettlementJournal::default());
    let result = run_turn(binding(), model.clone(), Arc::new(SettlementTool { forever_running:true, ..Default::default() }),
        journal.clone(), settlement_request(1, 8), AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { ref reason } if reason == "EXECUTION_NO_PROGRESS"));
    assert_eq!(result.model_steps, 3);
    assert_eq!(model.requests.lock().unwrap().len(), 3);
    assert!(journal.checkpoints.lock().unwrap().iter().all(|checkpoint| checkpoint.model_steps == 0));
}

#[tokio::test]
async fn process_boundary_original_cumulative_budget_still_stops_before_another_poll() {
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(settlement_steps()), requests:Default::default() });
    let tools = Arc::new(SettlementTool::default());
    let result = run_turn(binding(), model.clone(), tools.clone(), Arc::new(SettlementJournal::default()), settlement_request(1, 3),
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { ref reason } if reason == "EXECUTION_TOTAL_MODEL_BUDGET"));
    assert_eq!(result.model_steps, 3);
    assert!(!tools.calls.lock().unwrap().iter().any(|(_, name)| name == "poll_process"));
}

#[tokio::test]
async fn process_boundary_without_authorized_controls_keeps_the_original_safe_pause() {
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(settlement_steps()), requests:Default::default() });
    let tools = Arc::new(SettlementTool::default());
    let mut request = settlement_request(1, 8);
    request.tool_plan = AgentToolPlan::new([
        tool_binding("read_file", "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, true),
        tool_binding("start_process", "workspace.process", "workspace.process/start", AgentEffectClass::ManagedEffect, false),
        tool_binding("exec_command", "workspace.process", "workspace.process/exec", AgentEffectClass::ManagedEffect, false),
    ]).unwrap();
    let result = run_turn(binding(), model.clone(), tools.clone(), Arc::new(SettlementJournal::default()), request,
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { ref reason } if reason == "EXECUTION_NON_QUIESCENT_BOUNDARY"));
    assert_eq!(result.model_steps, 1);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert_eq!(tools.calls.lock().unwrap().iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>(), ["start_process"]);
}

#[tokio::test]
async fn process_boundary_specific_tool_choice_stays_within_the_narrowed_schema_surface() {
    for name in ["start_process", "write_file", "read_file"] {
        let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(settlement_steps()), requests:Default::default() });
        let mut request = settlement_request(1, 8);
        request.model_request.input.tool_choice = ChatToolChoice::Specific { name:name.into() };
        let result = run_turn(binding(), model.clone(), Arc::new(SettlementTool::default()), Arc::new(SettlementJournal::default()),
            request, AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        let requests = model.requests.lock().unwrap();
        assert!(requests.iter().all(|request| match &request.input.tool_choice {
            ChatToolChoice::Specific { name } => request.input.tools.iter().any(|tool| &tool.name == name),
            _ => true,
        }));
        if name == "read_file" {
            assert_eq!(requests[1].input.tool_choice, ChatToolChoice::Specific { name:name.into() });
        } else {
            assert_eq!(requests[1].input.tool_choice, ChatToolChoice::Auto);
        }
    }
}

#[tokio::test]
async fn process_boundary_real_host_stops_dominate_deferred_window_pressure() {
    for reason in [crate::AgentExecutionStopReason::UserRequested, crate::AgentExecutionStopReason::TurnJournalBudget,
        crate::AgentExecutionStopReason::SessionPayloadBudget, crate::AgentExecutionStopReason::DispatchWindow] {
        let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(settlement_steps()), requests:Default::default() });
        let journal = Arc::new(SettlementJournal { renew_while_running:true, stop_after_steps:Some((2, reason)), ..Default::default() });
        let result = run_turn(binding(), model.clone(), Arc::new(SettlementTool::default()), journal,
            settlement_request(1, 8), AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Paused { reason:ref actual } if actual == reason.code()));
        assert_eq!(result.model_steps, 2);
    }
}

#[tokio::test]
async fn process_boundary_checkpoint_ack_failure_never_admits_the_successor_window() {
    for write_failure in [false, true] {
        let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(settlement_steps()), requests:Default::default() });
        let journal = Arc::new(SettlementJournal { no_renewal_ack:!write_failure, fail_renewal:write_failure, ..Default::default() });
        let result = run_turn(binding(), model.clone(), Arc::new(SettlementTool::default()), journal.clone(),
            settlement_request(1, 8), AgentContextBudget::default(), CancellationToken::new()).await;
        if write_failure { assert!(matches!(result, Err(AgentEngineError::EventSink(_)))); }
        else { assert!(matches!(result.unwrap().terminal, AgentTurnTerminal::Paused { ref reason } if reason == "EXECUTION_CHECKPOINT_UNAVAILABLE")); }
        assert_eq!(model.requests.lock().unwrap().len(), 4);
        assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::ExecutionSegmentRenewed { .. })));
    }
}

#[tokio::test]
async fn process_boundary_unreaped_terminal_never_becomes_a_checkpoint() {
    let mut steps = vec![settlement_steps()[0].clone()];
    steps.extend((0..5).map(|index| control_step(&format!("unknown-{index}"), "poll_process", json!({"process_id":"owned"}))));
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(steps), requests:Default::default() });
    let journal = Arc::new(SettlementJournal::default());
    let result = run_turn(binding(), model, Arc::new(SettlementTool { unreaped:true, ..Default::default() }),
        journal.clone(), settlement_request(1, 8), AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { .. }));
    assert!(journal.checkpoints.lock().unwrap().iter().all(|checkpoint| checkpoint.model_steps == 0));
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnCompleted { .. })));
}

#[tokio::test]
async fn process_boundary_cancellation_interrupts_the_original_input_wait() {
    let entered = Arc::new(Notify::new());
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(settlement_steps()), requests:Default::default() });
    let journal = Arc::new(SettlementJournal::default());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(run_turn(binding(), model.clone(), Arc::new(SettlementTool { block_input:Some(entered.clone()), ..Default::default() }),
        journal.clone(), settlement_request(1, 8), AgentContextBudget::default(), cancellation.clone()));
    tokio::time::timeout(Duration::from_secs(2), entered.notified()).await.unwrap();
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Cancelled));
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    assert!(journal.checkpoints.lock().unwrap().iter().all(|checkpoint| checkpoint.work.running_processes.is_empty()));
}

#[tokio::test]
async fn process_boundary_accepted_steering_can_replan_and_stop_only_the_original_process() {
    let steer = Arc::new(OneSteer { input:Default::default() });
    let tools = Arc::new(SettlementTool { steer:Some(steer.clone()), ..Default::default() });
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![settlement_steps()[0].clone(),
        plan_step("in_progress", "inspect"), control_step("stop-original", "cancel_process", json!({"process_id":"owned"})),
        text_step("not requested")]), requests:Default::default() });
    let journal = Arc::new(SettlementJournal { stop_after_steps:Some((3, crate::AgentExecutionStopReason::UserRequested)), ..Default::default() });
    let result = run_turn(binding(), model.clone(), tools.clone(), journal.clone(), settlement_request(1, 8).with_input_port(steer),
        AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { ref reason } if reason == "EXECUTION_USER_REQUESTED"));
    assert!(tools.calls.lock().unwrap().iter().any(|(id, name)| id == "stop-original" && name == "cancel_process"));
    assert!(model.requests.lock().unwrap()[1].input.messages.iter().flat_map(|message| &message.content).any(|part|
        matches!(part, ChatContentPart::Text { text } if text.contains("Stop the original helper"))));
    assert!(journal.checkpoints.lock().unwrap().iter().all(|checkpoint| checkpoint.work.running_processes.is_empty()));
}

#[tokio::test]
async fn process_boundary_model_only_recovery_tail_keeps_consumed_steps_past_the_old_soft_window() {
    let checkpoint = crate::AgentExecutionCheckpoint {
        version:1, binding:binding(), turn_operation_id:"turn".into(), active_set_generation:0, model_steps:0, tool_call_count:0,
        accepted_input_count:1, applied_steering_receipts:vec![], plan:Default::default(), work:Default::default(), patch_recovery:Default::default(),
        segments:Some(crate::AgentExecutionSegmentState::new(1, crate::AgentSegmentPolicy { max_segments:8, max_no_progress_segments:2 }).unwrap()),
        control_rejections:Default::default(), completion_review:Default::default(), pending_user_input:None,
    };
    let prefix = vec![AgentEngineEvent::TurnStarted { binding:binding(), turn_operation_id:"turn".into() },
        AgentEngineEvent::ExecutionCheckpointSaved { step:0, revision:1, digest:"a".repeat(64).into() }];
    let tail = (1..=2).map(|step| AgentEngineEvent::ModelStepStarted { step, operation_id:format!("turn:model:{step}").into(), correction_source:None }).collect();
    let recovery = crate::AgentTurnRecovery::new(checkpoint.clone(), 1, 1, prefix.clone(), tail, vec![]).unwrap();
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![text_step("No tool was replayed.")]), requests:Default::default() });
    let journal = Arc::new(SettlementJournal::default());
    let result = run_turn(binding(), model.clone(), Arc::new(EchoTool), journal.clone(),
        settlement_request(1, 8).with_recovery(recovery), AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert_eq!(result.model_steps, 3);
    assert_eq!(journal.checkpoints.lock().unwrap()[0].model_steps, 2);
    assert_eq!(model.requests.lock().unwrap()[0].causality.operation_id.as_ref(), "turn:model:3");
    let over_budget = (1..=9).map(|step| AgentEngineEvent::ModelStepStarted { step, operation_id:format!("turn:model:{step}").into(), correction_source:None }).collect();
    assert!(crate::AgentTurnRecovery::new(checkpoint, 1, 1, prefix, over_budget, vec![]).is_err(), "recovery cannot mint another model allowance");
}
