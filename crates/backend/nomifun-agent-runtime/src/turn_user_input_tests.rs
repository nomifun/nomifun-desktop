#[derive(Debug, Default)]
struct InputPolicy { deny: bool, cancel: Option<CancellationToken> }

#[async_trait]
impl crate::AgentUserInputRequestPort for InputPolicy {
    async fn check(&self, _: &ChatCausality, _: &nomifun_agent_contracts::AgentUserInputRequest)
        -> Result<Option<String>, AgentEngineError> {
        if let Some(token) = &self.cancel { token.cancel(); }
        Ok(self.deny.then(|| "The automatic strategy does not permit a user-input handoff.".into()))
    }
}

#[derive(Default)]
struct InputJournal {
    journal: SettlementJournal,
    fail_source: bool,
    no_pending_ack: bool,
}

#[derive(Debug)]
struct InputFenceSteer {
    input: TerminalFenceSteer,
    journal: Arc<InputJournal>,
}

impl std::fmt::Debug for InputJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("InputJournal") }
}

#[async_trait]
impl crate::AgentInputPort for InputFenceSteer {
    async fn take(&self, causality: &ChatCausality, close_if_empty: bool) -> Result<Vec<crate::AgentSteeringInput>,AgentEngineError> {
        let inputs = self.input.take(causality,close_if_empty).await?;
        if !inputs.is_empty() { self.journal.emit(AgentEngineEvent::SteeringInputs {inputs:inputs.clone()}).await?; }
        Ok(inputs)
    }
    async fn has_pending(&self, causality: &ChatCausality) -> Result<bool,AgentEngineError> { self.input.has_pending(causality).await }
}

#[async_trait]
impl AgentEventSink for InputJournal {
    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        if self.fail_source && matches!(event, AgentEngineEvent::UserInputRequested { .. }) {
            return Err(AgentEngineError::EventSink("source question write failed".into()));
        }
        self.journal.emit(event).await
    }
    fn supports_checkpoints(&self) -> bool { true }
    async fn save_checkpoint(&self, checkpoint: crate::AgentExecutionCheckpoint)
        -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
        if self.no_pending_ack && checkpoint.pending_user_input.is_some() { return Ok(None); }
        self.journal.save_checkpoint(checkpoint).await
    }
}

fn input_step(id: &str) -> Vec<Result<ChatModelEvent, ChatModelError>> {
    control_step(id, crate::user_input_request::TOOL_NAME,
        json!({"question":"What name and interests should the self-introduction use?", "remaining_work":"Create and verify the requested PPT after the user supplies these details."}))
}

fn input_request(plan: AgentToolPlan) -> AgentTurnRequest {
    AgentTurnRequest::new(request(), plan, principal(), 0)
        .with_user_input_request_port(Arc::new(InputPolicy::default()))
}

fn input_session(model: Arc<dyn AgentModelPort>, tools: Arc<dyn AgentToolInvoker>, sink: Arc<dyn AgentEventSink>)
    -> crate::engine::AgentEngineSession {
    AgentEngine::new(AgentEngineBuild { build_id: "coding-dev".into(), build_digest:"a".repeat(64).into() }).unwrap()
        .open_session(binding(), model, tools, Some(sink)).unwrap()
}

#[tokio::test]
async fn user_input_direct_handoff_has_source_and_unfinished_requirements_without_fake_answer() {
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![input_step("question")]), requests:Default::default() });
    let journal = Arc::new(InputJournal::default());
    let result = input_session(model.clone(), Arc::new(EchoTool), journal.clone()).run_turn(input_request(Default::default())).await.unwrap();
    let AgentTurnTerminal::InputRequired { request: question } = &result.terminal else { panic!("{result:?}"); };
    assert_eq!(question.request_id.as_ref(), "question");
    assert!(result.output_text.is_empty(), "the typed question must not fabricate original assistant text");
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    let events = journal.journal.events.lock().unwrap().clone();
    assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::UserInputRequested { request, .. } if request == question)));
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::CompletionReported { .. })));
    let cp = journal.journal.checkpoints.lock().unwrap().last().unwrap().clone();
    assert_eq!(cp.pending_user_input.as_ref(), Some(question));
    assert_eq!(cp.plan.requirements[0].source.input, 0);
    assert_eq!(cp.plan.requirements[0].source.quote, "inspect");
    let mut history = vec![];
    crate::replay_closed_turn(&mut history, request().input.messages[0].clone(), &events).unwrap();
    let encoded = serde_json::to_string(&history).unwrap();
    assert!(encoded.contains("source question") && encoded.contains(&question.question) && encoded.contains("inspect"));
}

struct InputHelpTool { calls: AtomicUsize, failed: bool }
#[async_trait]
impl AgentToolInvoker for InputHelpTool {
    async fn invoke(&self, invocation: AgentToolInvocation, _: CancellationToken) -> Result<AgentToolResult, AgentEngineError> {
        if let Some(result) = instruction_result(&invocation) { return Ok(result); }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AgentToolResult::text(invocation.call.call_id, json!({"process_id":"help", "state":"exited", "success":!self.failed, "exit_code":if self.failed {1} else {0},
            "output":{"text":"help usage", "next_cursor":10}, "cleanup":{"reaped":true,"errors":[]}}).to_string(), self.failed))
    }
}

#[tokio::test]
async fn user_input_after_two_help_calls_text_stop_can_declare_input_without_report_failure() {
    let mut help = control_step("help-a", "exec_command", json!({"command":"help batch"}));
    help.insert(1, Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall { call_id:"help-b".into(), name:"exec_command".into(),
        arguments:nomifun_agent_contracts::StrictJsonValue(json!({"command":"help shape"})),provider_metadata:None } }));
    let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![help, text_step("Please supply your name and interests."), input_step("question")]), requests:Default::default() });
    let tool = Arc::new(InputHelpTool { calls:AtomicUsize::new(0), failed:false });
    let journal = Arc::new(InputJournal::default());
    let result = input_session(model.clone(), tool.clone(), journal.clone()).run_turn(input_request(settlement_tool_plan())).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::InputRequired { .. }));
    assert_eq!(tool.calls.load(Ordering::SeqCst), 2);
    let requests = model.requests.lock().unwrap();
    let declaration = &requests[2].input;
    assert_eq!(declaration.tool_choice, ChatToolChoice::Required);
    assert_eq!(declaration.tools.iter().map(|tool| tool.name.as_str()).collect::<BTreeSet<_>>(),
        BTreeSet::from([crate::user_input_request::TOOL_NAME, crate::completion::TOOL_NAME]));
    assert!(journal.journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::EndIntentReview { .. })));
}

#[tokio::test]
async fn user_input_open_plan_is_retained_and_mixed_batch_never_executes() {
    let mut help = control_step("help-a", "exec_command", json!({"command":"help batch"}));
    help.insert(1, Ok(ChatModelEvent::ToolCallCompleted {call:ChatToolCall {call_id:"help-b".into(),name:"exec_command".into(),
        arguments:nomifun_agent_contracts::StrictJsonValue(json!({"command":"help shape"})),provider_metadata:None}}));
    let mut mixed = input_step("mixed-question");
    mixed.insert(1, Ok(ChatModelEvent::ToolCallCompleted { call:ChatToolCall { call_id:"held-help".into(), name:"exec_command".into(),
        arguments:nomifun_agent_contracts::StrictJsonValue(json!({"command":"help"})),provider_metadata:None } }));
    let model = Arc::new(ScriptedModel { steps:std::sync::Mutex::new(vec![help, plan_step("in_progress", "inspect"), mixed, input_step("fresh-question")]) });
    let tool = Arc::new(InputHelpTool {calls:AtomicUsize::new(0),failed:false});
    let journal = Arc::new(InputJournal::default());
    let result = input_session(model, tool.clone(), journal.clone()).run_turn(input_request(settlement_tool_plan())).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::InputRequired { .. }));
    assert_eq!(tool.calls.load(Ordering::SeqCst), 2, "only the initial help calls ran; the mixed batch was held");
    let cp = journal.journal.checkpoints.lock().unwrap().last().unwrap().clone();
    assert!(cp.plan.is_open());
    assert!(cp.work.failed_tools >= 2, "paired refusals remain recorded; handoff is not task success");
    assert_eq!(journal.journal.events.lock().unwrap().iter().filter(|event| matches!(event, AgentEngineEvent::UserInputRequested { .. })).count(),1);
}

#[tokio::test]
async fn user_input_policy_and_shape_rejections_share_existing_control_budget() {
    for malformed in [false, true] {
        let steps = (0..4).map(|index| if malformed { control_step(&format!("bad-{index}"), crate::user_input_request::TOOL_NAME,
            json!({"question":"", "remaining_work":"pending"})) } else { input_step(&format!("denied-{index}")) }).collect();
        let model = Arc::new(ObservingModel {steps:std::sync::Mutex::new(steps),requests:Default::default()});
        let journal = Arc::new(InputJournal::default());
        let turn = input_request(Default::default()).with_user_input_request_port(Arc::new(InputPolicy {deny:true,cancel:None}));
        let result = input_session(model.clone(), Arc::new(EchoTool), journal.clone()).run_turn(turn).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_INTERVENTION_REQUIRED"));
        assert_eq!(journal.journal.checkpoints.lock().unwrap().last().unwrap().work.failed_tools, 4);
        assert_eq!(model.requests.lock().unwrap().len(),4);
        assert!(!journal.journal.events.lock().unwrap().iter().any(|event| matches!(event,
            AgentEngineEvent::UserInputRequested { .. } | AgentEngineEvent::TurnInputRequired { .. })));
    }
}

#[tokio::test]
async fn user_input_source_checkpoint_failure_and_cancellation_never_deliver_handoff() {
    for (fail_source, no_pending_ack, cancel) in [(true,false,false),(false,true,false),(false,false,true)] {
        let journal = Arc::new(InputJournal {fail_source,no_pending_ack,..Default::default()});
        let token = CancellationToken::new();
        let turn = input_request(Default::default()).with_user_input_request_port(Arc::new(InputPolicy {deny:false,cancel:cancel.then(|| token.clone())}));
        let session = input_session(Arc::new(ScriptedModel {steps:std::sync::Mutex::new(vec![input_step("question")])}),Arc::new(EchoTool),journal.clone());
        let result = session.run_turn_cancellable(turn, token).await;
        if cancel { assert!(matches!(result.unwrap().terminal, AgentTurnTerminal::Cancelled)); }
        else { assert!(result.is_err()); }
        assert!(!journal.journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnInputRequired { .. })));
    }
}

#[tokio::test]
async fn user_input_terminal_fence_real_reply_retires_pending_question() {
    let journal = Arc::new(InputJournal::default());
    let steer = Arc::new(InputFenceSteer {journal:journal.clone(),input:TerminalFenceSteer {input:std::sync::Mutex::new(Some(crate::AgentSteeringInput {
        receipt_operation_id:"answer".into(),message_id:"answer-message".into(),text:"Use Alice and hiking.".into(),files:vec![],inject_skills:vec![],
        image_count:0,prepared_images:vec![],prepared_skill_instructions:vec![],
    }))}});
    let model = Arc::new(ObservingModel {steps:std::sync::Mutex::new(vec![input_step("old-question"), input_step("fresh-question")]),requests:Default::default()});
    let result = input_session(model.clone(),Arc::new(EchoTool),journal.clone()).run_turn(input_request(Default::default()).with_input_port(steer)).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::InputRequired { request } if request.request_id.as_ref()=="fresh-question"));
    assert_eq!(model.requests.lock().unwrap().len(),2);
    let cp = journal.journal.checkpoints.lock().unwrap().last().unwrap().clone();
    assert_eq!(cp.accepted_input_count,2);
    assert_eq!(cp.plan.requirements.len(),2);
    assert_eq!(cp.pending_user_input.unwrap().request_id.as_ref(),"fresh-question");
    let events = journal.journal.events.lock().unwrap();
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::TurnInputRequired { .. })).count(),1);
    assert!(crate::user_input_request::recorded_pending(&events).is_ok());
}

#[tokio::test]
async fn user_input_pending_checkpoint_recovers_terminal_fence_without_model_or_tool_replay() {
    let journal = Arc::new(InputJournal::default());
    input_session(Arc::new(ScriptedModel {steps:std::sync::Mutex::new(vec![input_step("question")])}),Arc::new(EchoTool),journal.clone())
        .run_turn(input_request(Default::default())).await.unwrap();
    let cp = journal.journal.checkpoints.lock().unwrap().last().unwrap().clone();
    let events = journal.journal.events.lock().unwrap().clone();
    let at = events.iter().rposition(|event| matches!(event, AgentEngineEvent::ExecutionCheckpointSaved { .. })).unwrap();
    let AgentEngineEvent::ExecutionCheckpointSaved { revision, .. } = events[at] else { unreachable!() };
    let prefix = events[..=at].to_vec();
    let recovery = crate::AgentTurnRecovery::new(cp.clone(),revision,2,prefix.clone(),vec![],vec![]).unwrap();
    let model = Arc::new(ObservingModel {steps:std::sync::Mutex::new(vec![]),requests:Default::default()});
    let resumed = Arc::new(InputJournal::default());
    let result = input_session(model.clone(),Arc::new(EchoTool),resumed.clone()).run_turn(input_request(Default::default()).with_recovery(recovery.clone())).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::InputRequired { .. }));
    assert!(model.requests.lock().unwrap().is_empty());
    assert!(!resumed.journal.events.lock().unwrap().iter().any(|event| matches!(event,AgentEngineEvent::UserInputRequested{..}|AgentEngineEvent::ToolStarted{..})));
    let answered = Arc::new(InputJournal::default());
    let steer = Arc::new(InputFenceSteer {journal:answered.clone(),input:TerminalFenceSteer {input:std::sync::Mutex::new(Some(crate::AgentSteeringInput {
        receipt_operation_id:"recovered-answer".into(),message_id:"answer-message".into(),text:"Use Alice.".into(),files:vec![],inject_skills:vec![],
        image_count:0,prepared_images:vec![],prepared_skill_instructions:vec![],
    }))}});
    let next_model = Arc::new(ObservingModel {steps:std::sync::Mutex::new(vec![input_step("fresh-question")]),requests:Default::default()});
    let next = input_session(next_model.clone(),Arc::new(EchoTool),answered.clone()).run_turn(input_request(Default::default()).with_recovery(recovery).with_input_port(steer)).await.unwrap();
    assert!(matches!(next.terminal,AgentTurnTerminal::InputRequired{request} if request.request_id.as_ref()=="fresh-question"));
    assert_eq!(next_model.requests.lock().unwrap().len(),1);
    let new_events = answered.journal.events.lock().unwrap();
    assert!(!new_events.iter().any(|event| matches!(event,AgentEngineEvent::UserInputRequested{request,..}|AgentEngineEvent::TurnInputRequired{request,..} if request.request_id.as_ref()=="question")));
    let missing = prefix.into_iter().filter(|event| !matches!(event,AgentEngineEvent::UserInputRequested{..})).collect();
    assert!(crate::AgentTurnRecovery::new(cp,revision,2,missing,vec![],vec![]).is_err());
}

#[tokio::test]
async fn user_input_rejection_keeps_the_bounded_end_intent_surface_closed() {
    let mut help = control_step("help-a", "exec_command", json!({"command":"help batch"}));
    help.insert(1, Ok(ChatModelEvent::ToolCallCompleted {call:ChatToolCall {call_id:"help-b".into(),name:"exec_command".into(),
        arguments:nomifun_agent_contracts::StrictJsonValue(json!({"command":"help shape"})),provider_metadata:None}}));
    let mut steps = vec![help,text_step("Please provide your name.")];
    steps.extend((0..4).map(|index| input_step(&format!("denied-{index}"))));
    let model = Arc::new(ObservingModel {steps:std::sync::Mutex::new(steps),requests:Default::default()});
    let turn = input_request(settlement_tool_plan()).with_user_input_request_port(Arc::new(InputPolicy{deny:true,cancel:None}));
    let result = input_session(model.clone(),Arc::new(InputHelpTool{calls:AtomicUsize::new(0),failed:false}),Arc::new(InputJournal::default())).run_turn(turn).await;
    assert!(matches!(result.unwrap().terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_INTERVENTION_REQUIRED"));
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(),6);
    for request in &requests[2..] {
        assert_eq!(request.input.tools.iter().map(|tool| tool.name.as_str()).collect::<BTreeSet<_>>(),
            BTreeSet::from([crate::user_input_request::TOOL_NAME,crate::completion::TOOL_NAME]));
        assert_eq!(request.input.tool_choice,ChatToolChoice::Required);
    }
}

#[tokio::test]
async fn user_input_recovered_locked_report_phase_cannot_reopen_ending_intent() {
    let cp = crate::AgentExecutionCheckpoint {version:1,binding:binding(),turn_operation_id:"turn".into(),active_set_generation:0,
        model_steps:0,tool_call_count:0,accepted_input_count:1,applied_steering_receipts:vec![],plan:Default::default(),work:Default::default(),
        patch_recovery:Default::default(),segments:None,control_rejections:Default::default(),
        completion_review:crate::AgentCompletionReviewState{phase:AgentExecutionPhase::CompletionReview,input_revision:1,..Default::default()},pending_user_input:None};
    let prefix = vec![AgentEngineEvent::TurnStarted{binding:binding(),turn_operation_id:"turn".into()},
        AgentEngineEvent::RuntimeModulesActivated{modules:crate::adaptive::LEDGER_MODULES.to_vec(),reason:crate::AgentRuntimeActivationReason::ExplicitPlan},
        AgentEngineEvent::ExecutionCheckpointSaved{step:0,revision:1,digest:"d".repeat(64).into()}];
    let recovery = crate::AgentTurnRecovery::new(cp,1,2,prefix,vec![],vec![]).unwrap();
    let model = Arc::new(ObservingModel {steps:std::sync::Mutex::new((0..4).map(|index| input_step(&format!("refused-{index}"))).collect()),requests:Default::default()});
    let journal = Arc::new(InputJournal::default());
    let result = input_session(model.clone(),Arc::new(EchoTool),journal.clone()).run_turn(input_request(Default::default()).with_recovery(recovery)).await;
    assert!(matches!(result.unwrap().terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_INTERVENTION_REQUIRED"));
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(),4);
    assert!(requests.iter().all(|request| request.input.tools.iter().all(|tool|tool.name==crate::completion::TOOL_NAME)));
    assert!(!journal.journal.events.lock().unwrap().iter().any(|event|matches!(event,AgentEngineEvent::UserInputRequested{..}|AgentEngineEvent::TurnInputRequired{..}|AgentEngineEvent::EndIntentReview{..})));
}

#[tokio::test]
async fn user_input_cannot_escape_a_real_failure_stop() {
    let mut steps = vec![control_step("failure","exec_command",json!({"command":"help"}))];
    steps.extend((0..4).map(|index| input_step(&format!("refused-{index}"))));
    let tools = Arc::new(InputHelpTool {calls:AtomicUsize::new(0),failed:true});
    let journal = Arc::new(InputJournal::default());
    let result = input_session(Arc::new(ScriptedModel {steps:std::sync::Mutex::new(steps)}),tools,journal.clone())
        .run_turn(input_request(settlement_tool_plan())).await;
    assert!(matches!(result.unwrap().terminal, AgentTurnTerminal::Paused { reason } if reason == "EXECUTION_INTERVENTION_REQUIRED"));
    assert!(!journal.journal.events.lock().unwrap().iter().any(|event| matches!(event,AgentEngineEvent::UserInputRequested{..}|AgentEngineEvent::TurnInputRequired{..})));
}

#[test]
fn user_input_reconciliation_rebuilds_only_committed_source_and_new_input_retires_it() {
    let cp = crate::AgentExecutionCheckpoint { version:1,binding:binding(),turn_operation_id:"turn".into(),active_set_generation:0,
        model_steps:0,tool_call_count:0,accepted_input_count:1,applied_steering_receipts:vec![],plan:Default::default(),work:Default::default(),
        patch_recovery:Default::default(),segments:None,control_rejections:Default::default(),completion_review:Default::default(),pending_user_input:None };
    let call = ChatToolCall {call_id:"question".into(),name:crate::user_input_request::TOOL_NAME.into(),
        arguments:nomifun_agent_contracts::StrictJsonValue(json!({"question":"Your name?","remaining_work":"Create PPT."})),provider_metadata:None};
    let question = crate::user_input_request::from_call(&call).unwrap();
    let mut tail = vec![AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None},
        AgentEngineEvent::ToolCallCompleted{step:1,call},AgentEngineEvent::UserInputRequested{step:1,request:question.clone()}];
    let restored = crate::reconcile_execution_tail(&cp,1,&tail,&Default::default(),&Default::default(),&Default::default()).unwrap();
    assert_eq!(restored.checkpoint.pending_user_input,Some(question.clone()));
    assert!(restored.observations.iter().any(|event| matches!(event,AgentEngineEvent::ToolOutcomeReconciled{result,source:crate::AgentReconciliationSource::NativeControlReceipt,..}
        if result.call_id==question.request_id && !result.is_error)));
    tail.push(AgentEngineEvent::SteeringInputs{inputs:vec![crate::AgentSteeringInput {receipt_operation_id:"answer".into(),message_id:"message".into(),text:"Alice".into(),
        files:vec![],inject_skills:vec![],image_count:0,prepared_images:vec![],prepared_skill_instructions:vec![]}]});
    let after_input = crate::reconcile_execution_tail(&cp,1,&tail,&Default::default(),&Default::default(),&Default::default()).unwrap();
    assert!(after_input.checkpoint.pending_user_input.is_none());
    assert_eq!(after_input.checkpoint.accepted_input_count,2);
    let mut bad = restored.checkpoint;
    bad.patch_recovery.targets.push("a".into());
    assert!(bad.validate().is_err(),"pending input must never declare a patch obligation settled");
    let unknown = AgentEngineEvent::ToolStarted {step:1,call_id:"question".into(),capability_id:"workspace.process".into(),action_id:"workspace.process/start".into()};
    tail.insert(2,unknown);
    assert!(crate::reconcile_execution_tail(&cp,1,&tail,&Default::default(),&Default::default(),&Default::default()).is_err());
}
