#[derive(Debug, Default)]
struct ParkTestPort {
    entered: Notify,
    wake: Notify,
    observation: std::sync::Mutex<Option<crate::AgentOwnedProcessObservation>>,
    calls: AtomicUsize,
    policies: std::sync::Mutex<Vec<crate::AgentWaitWakePolicy>>,
}

#[async_trait]
impl crate::AgentWaitPort for ParkTestPort {
    async fn wait(&self, _: &ChatCausality, request: &crate::AgentWaitRequest, cancellation: CancellationToken)
        -> Result<crate::AgentWaitWake, AgentEngineError> {
        assert_eq!(request.processes.len(), 1);
        assert_eq!(request.processes[0].process_id, "owned");
        assert_eq!(request.processes[0].output_cursor, 5);
        self.policies.lock().unwrap().push(request.wake_policy);
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
async fn user_question_finishes_computation_with_live_process_and_never_samples_a_restart() {
    // The trailing proposal deliberately tries to relaunch with a new call ID.
    // No prompt instruction is needed: the Host cannot reach it without input.
    let model = Arc::new(ObservingModel { requests: Default::default(), steps: std::sync::Mutex::new(vec![
        wait_ledger_step(), plan_step("in_progress", "inspect"),
        control_step("launch", "start_process", json!({"command":"helper", "wait_ms":0})),
        input_step("authorization"),
        control_step("unsolicited-restart", "start_process", json!({"command":"helper", "wait_ms":0})),
    ]) });
    let journal = Arc::new(SettlementJournal::default());
    let wait = Arc::new(ParkTestPort::default());
    *wait.observation.lock().unwrap() = Some(parked_timeout());
    let tools = Arc::new(SettlementTool::default());
    let result = tokio::time::timeout(Duration::from_secs(2), run_turn(binding(), model.clone(), tools.clone(), journal.clone(),
        input_request(settlement_tool_plan()).with_wait_port(wait.clone()), AgentContextBudget::default(), CancellationToken::new()))
        .await.unwrap().unwrap();
    assert!(matches!(&result.terminal, AgentTurnTerminal::InputRequired { request } if request.request_id.as_ref() == "authorization"));
    assert_eq!(model.requests.lock().unwrap().len(), 4);
    assert_eq!(model.steps.lock().unwrap().len(), 1, "background timeout cannot drive another model step");
    assert_eq!(wait.calls.load(Ordering::SeqCst), 0, "a human question never enters the process wait loop");
    let events = journal.events.lock().unwrap();
    assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::TurnInputRequired { .. })));
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnWaiting { .. } | AgentEngineEvent::TurnWaitResolved { .. }
        | AgentEngineEvent::OwnedProcessObserved { .. } | AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::TurnFailed { .. })));
    assert_eq!(crate::user_input_request::recorded_pending(&events).unwrap().unwrap().request_id.as_ref(), "authorization");
    assert_eq!(tools.calls.lock().unwrap().iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>(), ["read_file", "read_file", "start_process"]);
    assert!(journal.checkpoints.lock().unwrap().iter().all(|checkpoint| checkpoint.pending_user_input.is_none()
        && checkpoint.work.running_processes.is_empty()), "normal questions do not create executable recovery state");
}

#[test]
fn process_wait_resolution_does_not_answer_a_pending_human_question() {
    let call = ChatToolCall { call_id:"question".into(), name:"request_user_input".into(),
        arguments:StrictJsonValue(json!({"question":"Please authorize in the browser.","remaining_work":"Verify actual login."})),provider_metadata:None };
    let question = crate::user_input_request::from_call(&call).unwrap();
    let events = vec![AgentEngineEvent::ModelStepStarted {step:1,operation_id:"model-1".into(),correction_source:None},
        AgentEngineEvent::ToolCallCompleted {step:1,call}, AgentEngineEvent::UserInputRequested {step:1,request:question.clone()},
        AgentEngineEvent::TurnWaitResolved {model_steps:1}];
    assert_eq!(crate::user_input_request::recorded_pending(&events).unwrap(), Some(question));
}

fn blocked_without_timeout() -> Vec<Result<ChatModelEvent,ChatModelError>> {
    control_step("blocked-decision","report_completion",json!({"summary":"The observed files remain available; a user decision is still required.",
        "criteria":[{"disposition":"blocked","rationale":"The user must supply the remaining choice."}]}))
}

#[tokio::test]
async fn ordinary_blocked_account_rejects_text_stop_and_requires_a_real_input_declaration() {
    let model=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(vec![
        wait_ledger_step(),blocked_without_timeout(),text_step("This text cannot answer the pending user decision."),input_step("real-decision")])});
    let journal=Arc::new(SettlementJournal::default());
    let result=run_turn(binding(),model.clone(),Arc::new(SettlementTool::default()),journal.clone(),
        input_request(settlement_tool_plan()),AgentContextBudget::default(),CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::InputRequired {request} if request.request_id.as_ref()=="real-decision"));
    let events=journal.events.lock().unwrap();
    assert!(events.iter().any(|event|matches!(event,AgentEngineEvent::ModelResponseRejected {step:3,
        rejection:Some(AgentModelResponseRejection::RequiredInputHandoff),continuation:true,..})));
    assert!(events.iter().any(|event|matches!(event,AgentEngineEvent::ModelStepStarted {step:4,correction_source:Some(3),..})));
    assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::TurnCompleted {..}|AgentEngineEvent::TurnFailed {..}|AgentEngineEvent::TurnPaused {..})));
    assert_eq!(model.requests.lock().unwrap().len(),4);
    assert!(!result.output_text.contains("This text cannot answer"));
}

#[tokio::test]
async fn repeated_plain_text_cannot_complete_a_blocked_account_or_extend_its_correction_budget() {
    let model=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(vec![
        wait_ledger_step(),blocked_without_timeout(),text_step("First unaccepted text stop"),text_step("Second unaccepted text stop"),
        text_step("Third unaccepted text stop"),input_step("must-not-get-an-extra-model-step")])});
    let journal=Arc::new(SettlementJournal::default());
    let result=run_turn(binding(),model.clone(),Arc::new(SettlementTool::default()),journal.clone(),
        input_request(settlement_tool_plan()),AgentContextBudget::default(),CancellationToken::new()).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Paused {ref reason} if reason=="EXECUTION_INTERVENTION_REQUIRED"));
    assert_eq!(model.requests.lock().unwrap().len(),5);
    let events=journal.events.lock().unwrap();
    let continuations=events.iter().filter_map(|event|match event {AgentEngineEvent::ModelResponseRejected {
        rejection:Some(AgentModelResponseRejection::RequiredInputHandoff),continuation,..}=>Some(*continuation),_=>None}).collect::<Vec<_>>();
    assert_eq!(continuations,[true,true,false]);
    assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::TurnCompleted {..}|AgentEngineEvent::TurnFailed {..})));
    let mut restored=crate::protocol_recovery::ProtocolRecovery::default();restored.restore(events.iter()).unwrap();
    assert!(restored.correction_source().is_none(),"native recovery cannot buy a continuing correction after the original rejection exhausted its budget");
    assert!(restored.continuation_stopped());
}

#[tokio::test]
async fn unknown_or_unreaped_owner_terminal_keeps_the_wait_fenced_without_model_polling() {
    for lost in [false,true] {
        let model=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(vec![
            wait_ledger_step(),plan_step("in_progress","inspect"),
            control_step("launch","start_process",json!({"command":"authorization-helper","wait_ms":0})),
            text_step("The helper is running."),text_step("The helper has not exited."),text_step("must not sample an unknown terminal")])});
        let wait=Arc::new(ParkTestPort::default());
        let journal=Arc::new(SettlementJournal::default());
        let task=tokio::spawn(run_turn(binding(),model.clone(),Arc::new(SettlementTool::default()),journal.clone(),
            input_request(settlement_tool_plan()).with_wait_port(wait.clone()),AgentContextBudget::default(),CancellationToken::new()));
        tokio::time::timeout(Duration::from_secs(2),wait.entered.notified()).await.unwrap();
        let nomifun_engine_core::EngineProcessPoll::TimedOut {output,mut cleanup}=parked_timeout().poll else {unreachable!()};
        cleanup.reaped=lost;
        cleanup.errors=if lost {Vec::new()} else {vec!["original owner cleanup is unconfirmed".into()]};
        *wait.observation.lock().unwrap()=Some(crate::AgentOwnedProcessObservation {process_id:"owned".into(),
            poll:if lost {nomifun_engine_core::EngineProcessPoll::Lost {pid:1,output,cleanup}}
                else {nomifun_engine_core::EngineProcessPoll::TimedOut {output,cleanup}}});
        wait.wake.notify_one();
        assert!(matches!(tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap(),Err(AgentEngineError::WaitOwnerUnavailable)));
        assert_eq!(wait.calls.load(Ordering::SeqCst),1);
        assert_eq!(model.requests.lock().unwrap().len(),5);
        let events=journal.events.lock().unwrap();
        assert_eq!(events.iter().filter(|event|matches!(event,AgentEngineEvent::OwnedProcessObserved {..})).count(),1);
        assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::TurnWaitResolved {..}|AgentEngineEvent::TurnCompleted {..}|AgentEngineEvent::TurnInputRequired {..}|AgentEngineEvent::TurnFailed {..})));
    }
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
    assert_eq!(*wait.policies.lock().unwrap(), [crate::AgentWaitWakePolicy::OutputOrTerminal]);
    assert!(journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnWaiting {
        request_id: None, reason: nomifun_agent_contracts::AgentWaitReason::ExternalResource, .. })));
    token.cancel();
    assert!(matches!(tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap(), Err(AgentEngineError::Cancelled)));
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. })));
}

#[derive(Debug,Default)]
struct LateAuthorizationOutput {
    calls:AtomicUsize,
}

#[async_trait]
impl crate::AgentWaitPort for LateAuthorizationOutput {
    async fn wait(&self,_:&ChatCausality,request:&crate::AgentWaitRequest,_cancellation:CancellationToken)
        -> Result<crate::AgentWaitWake,AgentEngineError> {
        let call=self.calls.fetch_add(1,Ordering::SeqCst);
        if call==0 {
            assert_eq!(request.wake_policy,crate::AgentWaitWakePolicy::OutputOrTerminal);
            assert_eq!(request.processes[0].output_cursor,5);
            return Ok(crate::AgentWaitWake::ProcessesChanged(vec![crate::AgentOwnedProcessObservation {
                process_id:"owned".into(),poll:nomifun_engine_core::EngineProcessPoll::Running {
                    pid:1,
                    output:nomifun_engine_core::EngineProcessOutput {text:"Open the authorization page to continue".into(),next_cursor:64,
                        retained_bytes:64,dropped_bytes:0,source_encoding:"utf-8".into(),decode_errors:0},
                },
            }]));
        }
        panic!("the question must finish the Turn before a second resource wait: {call}");
    }
}

#[tokio::test]
async fn late_authorization_output_wakes_once_then_the_question_waits_without_more_model_sampling() {
    let model=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(vec![
        wait_ledger_step(),plan_step("in_progress","inspect"),
        control_step("launch","start_process",json!({"command":"authorization-helper","wait_ms":0})),
        text_step("The helper is starting."),text_step("The helper has not completed."),input_step("authorization"),
        text_step("must not sample while the user is authorizing")])});
    let wait=Arc::new(LateAuthorizationOutput::default());
    let journal=Arc::new(SettlementJournal::default());
    let result=tokio::time::timeout(Duration::from_secs(2), run_turn(binding(),model.clone(),Arc::new(SettlementTool::default()),journal.clone(),
        input_request(settlement_tool_plan()).with_wait_port(wait.clone()),AgentContextBudget::default(),CancellationToken::new())).await.unwrap().unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::InputRequired {..}));
    assert_eq!(model.requests.lock().unwrap().len(),6);
    assert!(model.requests.lock().unwrap()[5].input.messages.iter().flat_map(|message|&message.content).any(|part|
        matches!(part,ChatContentPart::Text {text} if text.contains("Open the authorization page"))));
    assert_eq!(wait.calls.load(Ordering::SeqCst),1);
    let events=journal.events.lock().unwrap();
    assert_eq!(events.iter().filter(|event|matches!(event,AgentEngineEvent::OwnedProcessObserved {..})).count(),1);
    assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::TurnFailed {..}|AgentEngineEvent::TurnCompleted {..})));
}

struct ParkBoundaryJournal { journal: SettlementJournal, input: Arc<OneSteer> }
impl std::fmt::Debug for ParkBoundaryJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("ParkBoundaryJournal") }
}
#[async_trait]
impl AgentEventSink for ParkBoundaryJournal {
    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        if matches!(event, AgentEngineEvent::UserInputRequested { .. }) {
            *self.input.input.lock().unwrap() = Some(crate::AgentSteeringInput { receipt_operation_id: "authorize-reply".into(),
                message_id: "authorize-reply-message".into(), text: "Authorization is complete; continue the original task.".into(),
                files: Vec::new(), inject_skills: Vec::new(), image_count: 0, prepared_images: Vec::new(), prepared_skill_instructions: Vec::new() });
        }
        self.journal.emit(event).await
    }
}

#[tokio::test]
async fn accepted_reply_at_question_publication_is_consumed_before_finite_handoff() {
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
        text_step("The helper is running."),text_step("The helper has not exited."),
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
    assert!(crate::user_input_request::recorded_pending(&events).unwrap().is_none(), "a resource-only wait never creates a user question");
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
async fn start_only_grant_settles_from_the_original_owner_without_minting_a_poll_call() {
    let plan=AgentToolPlan::new([tool_binding("start_process","workspace.process","workspace.process/start",AgentEffectClass::ManagedEffect,false)]).unwrap();
    assert!(plan.model_name_for_action("workspace.process","workspace.process/poll").is_none());
    let model=Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(vec![
        control_step("launch","start_process",json!({"command":"helper","wait_ms":0})),
        text_step("The original helper is still running."),text_step("The original helper has not exited yet."),
        control_step("done","report_completion",json!({"summary":"The original helper exited successfully.",
            "criteria":[{"disposition":"supported","evidence_call_ids":["launch"],"rationale":"The original process owner confirmed exit zero and reaped cleanup for that exact launch."}]}))])});
    let tools=Arc::new(SettlementTool::default());
    let wait=Arc::new(ParkTestPort::default());
    let journal=Arc::new(SettlementJournal::default());
    let turn=input_request(plan).with_wait_port(wait.clone()).with_max_model_steps(2)
        .with_execution_segments(crate::AgentSegmentPolicy {max_segments:4,max_no_progress_segments:2});
    let task=tokio::spawn(run_turn(binding(),model.clone(),tools.clone(),journal.clone(),turn,AgentContextBudget::default(),CancellationToken::new()));
    tokio::time::timeout(Duration::from_secs(2),wait.entered.notified()).await.unwrap();
    assert_eq!(model.requests.lock().unwrap().len(),3,"a passive owner wait survives the soft window without a control grant");
    let nomifun_engine_core::EngineProcessPoll::TimedOut {mut output,cleanup}=parked_timeout().poll else {unreachable!()};
    output.text="HELPER_EXITED_OK".into();
    *wait.observation.lock().unwrap()=Some(crate::AgentOwnedProcessObservation {process_id:"owned".into(),
        poll:nomifun_engine_core::EngineProcessPoll::Exited {exit_code:Some(0),signal:None,output,cleanup}});
    wait.wake.notify_one();
    let result=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Completed {..}));
    assert_eq!(tools.calls.lock().unwrap().iter().map(|(_,name)|name.as_str()).collect::<Vec<_>>(),["start_process"]);
    let requests=model.requests.lock().unwrap();
    assert_eq!(requests.len(),4);
    assert!(requests.iter().all(|request|!request.input.tools.iter().any(|tool|tool.name=="poll_process")));
    assert!(requests[3].input.instructions.iter().any(|text|text.contains("original_process_owner")&&text.contains("HELPER_EXITED_OK")));
    let events=journal.events.lock().unwrap();
    assert_eq!(events.iter().filter(|event|matches!(event,AgentEngineEvent::ToolStarted {..})).count(),1);
    assert_eq!(events.iter().filter(|event|matches!(event,AgentEngineEvent::OwnedProcessObserved {..})).count(),1);
    assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::TurnFailed {..}|AgentEngineEvent::TurnPaused {..})));
}

#[tokio::test]
async fn rejected_completion_report_preserves_successful_owner_receipts() {
    let mut steps = vec![wait_ledger_step(), plan_step("in_progress", "inspect"),
        control_step("launch", "start_process", json!({"command":"helper", "wait_ms":0})), text_step("The helper is running."),text_step("The helper has not exited."),
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
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
    assert!(matches!(result,Err(AgentEngineError::TurnFailed { ref message,
        failure:crate::AgentTurnFailure::TaskIncomplete { reason:nomifun_agent_contracts::AgentTaskIncompleteReason::CompletionReportRejected },
    }) if message.contains("invalid-report-3") && message.contains("INVALID_TOOL_ARGUMENTS")));
    assert_eq!(model.requests.lock().unwrap().len(), 11);
    assert_eq!(model.steps.lock().unwrap().len(), 1);
    let events = journal.events.lock().unwrap();
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnCompleted { .. })));
    let work = events.iter().rev().find_map(|event|if let AgentEngineEvent::WorkStatus {status}=event {Some(status)} else {None}).unwrap();
    assert_eq!(work.successful_commands,1);
    assert_eq!(work.failed_commands,0);
    assert_eq!(work.failed_tools,4,"report refusals do not rewrite the successful original command");
    assert!(work.running_processes.is_empty());
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
    assert_eq!(*wait.policies.lock().unwrap(),[crate::AgentWaitWakePolicy::TerminalOnly],
        "exhausted declarations cannot churn the canonical wait journal on spinner output");
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
