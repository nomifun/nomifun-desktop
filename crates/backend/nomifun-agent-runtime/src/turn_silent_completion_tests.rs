#[derive(Default)]
struct SilentRobotTools {
    calls: std::sync::Mutex<Vec<String>>,
    fail_on: Option<&'static str>,
    unknown_on: Option<&'static str>,
}

#[async_trait]
impl AgentToolInvoker for SilentRobotTools {
    async fn invoke(&self, invocation: AgentToolInvocation, _: CancellationToken)
        -> Result<AgentToolResult, AgentEngineError> {
        let id = invocation.call.call_id.as_ref();
        self.calls.lock().unwrap().push(id.to_owned());
        if self.unknown_on == Some(id) {
            return Err(AgentEngineError::CapabilityKernel {
                code:"CAPABILITY_UNAVAILABLE".into(), message:"The owner outcome is unknown; reconcile it before retrying.".into(),
            });
        }
        Ok(AgentToolResult::text(invocation.call.call_id.clone(),
            if self.fail_on == Some(id) { "Device refused the action." } else { "Device accepted the command." },
            self.fail_on == Some(id)))
    }
}

fn silent_robot_plan() -> AgentToolPlan {
    AgentToolPlan::new([
        tool_binding("robot_motion", "robot", "robot/motion", AgentEffectClass::ExternalUncertainEffect, false),
        tool_binding("robot_display", "robot", "robot/display", AgentEffectClass::ExternalUncertainEffect, false),
        tool_binding("weather", "fixture.weather", "fixture.weather/read", AgentEffectClass::ReadOnly, true),
    ]).unwrap()
}

fn silent_batch(calls: &[(&str, &str)]) -> Vec<Result<ChatModelEvent, ChatModelError>> {
    let mut events = calls.iter().map(|(id, name)| Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
        call_id:(*id).into(), name:(*name).into(), arguments:StrictJsonValue(json!({})), provider_metadata:None,
    }})).collect::<Vec<_>>();
    events.push(Ok(ChatModelEvent::Completed { finish_reason:ChatFinishReason::ToolCalls }));
    events
}

fn silent_model(steps: Vec<Vec<Result<ChatModelEvent, ChatModelError>>>) -> Arc<ObservingModel> {
    Arc::new(ObservingModel { steps:std::sync::Mutex::new(steps), requests:Default::default() })
}

fn silent_request() -> AgentTurnRequest {
    AgentTurnRequest::new(request(), silent_robot_plan(), principal(), 0).with_silent_robot_actions(true)
}

fn silent_markers(journal: &SettlementJournal) -> Vec<(u16, Vec<ToolCallId>)> {
    journal.events.lock().unwrap().iter().filter_map(|event| match event {
        AgentEngineEvent::SilentCompletion { step, tool_call_ids } => Some((*step, tool_call_ids.clone())),
        _ => None,
    }).collect()
}

#[tokio::test]
async fn silent_robot_final_action_batch_finishes_in_one_model_call_without_public_receipt() {
    let model = silent_model(vec![silent_batch(&[("motion", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)])]);
    let tools = Arc::new(SilentRobotTools::default());
    let journal = Arc::new(SettlementJournal::default());
    let result = input_session(model.clone(), tools.clone(), journal.clone())
        .run_turn(silent_request().with_max_model_steps(1)).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert!(result.output_text.is_empty());
    assert_eq!(result.model_steps, 1);
    assert_eq!(*tools.calls.lock().unwrap(), ["motion"]);
    assert_eq!(silent_markers(&journal), [(1, vec!["motion".into()])]);
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].input.instructions.iter().any(|text| text.contains("For mixed action-and-question requests")));
    let events = journal.events.lock().unwrap();
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::OutputTextDelta { .. } | AgentEngineEvent::CompletionDelivered { .. })));
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::ToolStarted { call_id, .. } if call_id.as_ref()=="finish")));
    assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::ToolCompleted { result, .. } if result.call_id.as_ref()=="finish" && !result.is_error)));
    let marker = events.iter().position(|event| matches!(event, AgentEngineEvent::SilentCompletion { .. })).unwrap();
    let terminal = events.iter().position(|event| matches!(event, AgentEngineEvent::TurnCompleted { .. })).unwrap();
    assert!(marker < terminal);
    let mut replay = vec![];
    crate::replay_closed_turn(&mut replay, request().input.messages[0].clone(), &events).unwrap();
    assert!(replay.iter().any(|message| message.content.iter().any(|part|
        matches!(part, ChatContentPart::ToolCall { name, .. } if name==crate::silent_completion::TOOL_NAME))),
        "the internal control keeps its canonical protocol pairing");
}

#[tokio::test]
async fn silent_robot_finishes_only_after_every_same_batch_action_in_order() {
    let model = silent_model(vec![silent_batch(&[("stand", "robot_motion"), ("head", "robot_motion"),
        ("face", "robot_display"), ("finish", crate::silent_completion::TOOL_NAME)])]);
    let tools = Arc::new(SilentRobotTools::default());
    let journal = Arc::new(SettlementJournal::default());
    let result = input_session(model, tools.clone(), journal.clone()).run_turn(silent_request()).await.unwrap();
    assert_eq!(result.model_steps, 1);
    assert_eq!(*tools.calls.lock().unwrap(), ["stand", "head", "face"]);
    assert_eq!(silent_markers(&journal), [(1, vec!["stand".into(), "head".into(), "face".into()])]);
}

#[tokio::test]
async fn silent_robot_does_not_stop_after_first_action_and_supports_single_call_models() {
    for separate_finish in [false, true] {
        let mut steps = vec![silent_batch(&[("stand", "robot_motion")])];
        if separate_finish {
            steps.push(silent_batch(&[("head", "robot_motion")]));
            steps.push(silent_batch(&[("finish", crate::silent_completion::TOOL_NAME)]));
        } else {
            steps.push(silent_batch(&[("head", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)]));
        }
        let model = silent_model(steps);
        let tools = Arc::new(SilentRobotTools::default());
        let journal = Arc::new(SettlementJournal::default());
        let result = input_session(model.clone(), tools.clone(), journal.clone()).run_turn(silent_request()).await.unwrap();
        assert_eq!(result.model_steps, if separate_finish { 3 } else { 2 });
        assert_eq!(*tools.calls.lock().unwrap(), ["stand", "head"]);
        assert_eq!(silent_markers(&journal), [(result.model_steps, vec!["stand".into(), "head".into()])]);
        assert_eq!(model.requests.lock().unwrap().len(), result.model_steps as usize);
    }
}

#[tokio::test]
async fn silent_robot_action_and_question_keep_the_normal_answer() {
    let model = silent_model(vec![silent_batch(&[("stand", "robot_motion")]), text_step("北京今天的天气查询结果。")]);
    let journal = Arc::new(SettlementJournal::default());
    let result = input_session(model, Arc::new(SilentRobotTools::default()), journal.clone()).run_turn(silent_request()).await.unwrap();
    assert_eq!(result.output_text, "北京今天的天气查询结果。");
    assert!(silent_markers(&journal).is_empty());
}

#[tokio::test]
async fn silent_robot_failure_or_unknown_result_holds_later_actions_and_requires_a_reply() {
    for unknown in [false, true] {
        let model = silent_model(vec![silent_batch(&[("failed", "robot_motion"), ("later", "robot_motion"),
            ("finish", crate::silent_completion::TOOL_NAME)]), text_step("动作没有确认成功，需要检查设备。")]);
        let tools = Arc::new(SilentRobotTools { fail_on:(!unknown).then_some("failed"),
            unknown_on:unknown.then_some("failed"), ..Default::default() });
        let journal = Arc::new(SettlementJournal::default());
        let result = input_session(model, tools.clone(), journal.clone()).run_turn(silent_request()).await.unwrap();
        assert_eq!(*tools.calls.lock().unwrap(), ["failed"]);
        assert!(!result.output_text.is_empty());
        assert!(silent_markers(&journal).is_empty());
        assert!(journal.events.lock().unwrap().iter().any(|event|matches!(event,
            AgentEngineEvent::ToolCompleted { result, .. } if result.call_id.as_ref()=="finish" && result.is_error)));
    }
}

#[tokio::test]
async fn silent_robot_mixed_business_work_or_invalid_finish_holds_the_entire_batch() {
    let mixed = silent_batch(&[("stand", "robot_motion"), ("query", "weather"), ("finish", crate::silent_completion::TOOL_NAME)]);
    let misplaced = silent_batch(&[("finish", crate::silent_completion::TOOL_NAME), ("stand", "robot_motion")]);
    let empty = silent_batch(&[("finish", crate::silent_completion::TOOL_NAME)]);
    let mut invalid = silent_batch(&[("stand", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)]);
    if let Ok(ChatModelEvent::ToolCallCompleted { call }) = &mut invalid[1] { call.arguments.0 = json!({"unexpected":true}); }
    for batch in [mixed, misplaced, empty, invalid] {
        let model = silent_model(vec![batch, text_step("需要正常回应。")]);
        let tools = Arc::new(SilentRobotTools::default());
        let journal = Arc::new(SettlementJournal::default());
        let result = input_session(model, tools.clone(), journal.clone()).run_turn(silent_request()).await.unwrap();
        assert!(tools.calls.lock().unwrap().is_empty());
        assert_eq!(result.output_text, "需要正常回应。");
        assert!(silent_markers(&journal).is_empty());
    }
}

#[tokio::test]
async fn silent_robot_prior_business_work_and_prior_public_text_cannot_be_erased() {
    for public in [false, true] {
        let mut first = silent_batch(&[("first", if public { "robot_motion" } else { "weather" })]);
        if public { first.insert(0, Ok(ChatModelEvent::OutputTextDelta { text:"正在处理。".into() })); }
        let model = silent_model(vec![first, silent_batch(&[("last", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)]),
            text_step("保留需要提供的回答。")]);
        let tools = Arc::new(SilentRobotTools::default());
        let journal = Arc::new(SettlementJournal::default());
        let result = input_session(model, tools.clone(), journal.clone()).run_turn(silent_request()).await.unwrap();
        assert_eq!(*tools.calls.lock().unwrap(), ["first"]);
        assert!(result.output_text.contains("保留需要提供的回答。"));
        assert!(silent_markers(&journal).is_empty());
    }
}

#[tokio::test]
async fn silent_robot_policy_is_opt_in_and_unexposed_control_cannot_dispatch_a_prefix() {
    let model = silent_model(vec![silent_batch(&[("stand", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)]), text_step("正常回应。")]);
    let tools = Arc::new(SilentRobotTools::default());
    let journal = Arc::new(SettlementJournal::default());
    let result = input_session(model.clone(), tools.clone(), journal.clone())
        .run_turn(AgentTurnRequest::new(request(), silent_robot_plan(), principal(), 0)).await.unwrap();
    assert!(tools.calls.lock().unwrap().is_empty());
    assert_eq!(result.output_text, "正常回应。");
    assert!(silent_markers(&journal).is_empty());
    assert!(model.requests.lock().unwrap().iter().all(|request|
        !request.input.tools.iter().any(|tool|tool.name==crate::silent_completion::TOOL_NAME)
            && !request.input.instructions.iter().any(|text| text.contains(crate::silent_completion::INSTRUCTIONS))));
}

#[tokio::test]
async fn silent_robot_terminal_steering_keeps_turn_open_and_does_not_replay_the_action() {
    let model = silent_model(vec![silent_batch(&[("stand", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)]),
        vec![Err(ChatModelError::protocol_violation("controlled stop after steering"))]]);
    let tools = Arc::new(SilentRobotTools::default());
    let journal = Arc::new(SettlementJournal::default());
    let port = Arc::new(TerminalFenceSteer { input:std::sync::Mutex::new(Some(crate::AgentSteeringInput {
        receipt_operation_id:"last-second-steer".into(), message_id:"steer-message".into(), text:"also explain".into(),
        files:vec![], inject_skills:vec![], image_count:0, prepared_images:vec![], prepared_skill_instructions:vec![],
    })) });
    assert!(input_session(model.clone(), tools.clone(), journal.clone())
        .run_turn(silent_request().with_input_port(port)).await.is_err());
    assert_eq!(*tools.calls.lock().unwrap(), ["stand"]);
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    assert!(serde_json::to_string(&model.requests.lock().unwrap()[1].input.messages).unwrap().contains("also explain"));
    assert!(silent_markers(&journal).is_empty());
}

#[derive(Debug)]
struct SilentCancelFence(CancellationToken);
#[async_trait]
impl crate::AgentInputPort for SilentCancelFence {
    async fn take(&self, _: &ChatCausality, terminal: bool) -> Result<Vec<crate::AgentSteeringInput>, AgentEngineError> {
        if terminal { self.0.cancel(); }
        Ok(vec![])
    }
    async fn has_pending(&self, _: &ChatCausality) -> Result<bool, AgentEngineError> { Ok(false) }
}

#[tokio::test]
async fn silent_robot_cancellation_at_the_terminal_fence_wins_over_finish_intent() {
    let model = silent_model(vec![silent_batch(&[("stand", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)])]);
    let tools = Arc::new(SilentRobotTools::default());
    let journal = Arc::new(SettlementJournal::default());
    let token = CancellationToken::new();
    let result = input_session(model, tools.clone(), journal.clone()).run_turn_cancellable(
        silent_request().with_input_port(Arc::new(SilentCancelFence(token.clone()))), token).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Cancelled));
    assert_eq!(*tools.calls.lock().unwrap(), ["stand"]);
    assert!(silent_markers(&journal).is_empty());
}

#[tokio::test]
async fn silent_robot_accepted_delegate_marks_waiting_without_claiming_child_completion() {
    let model = silent_model(vec![control_step("delegate", "delegate", json!({"strategy":"planned","goal":"weather"}))]);
    let journal = Arc::new(SettlementJournal::default());
    let result = input_session(model.clone(), Arc::new(EchoTool), journal.clone()).run_turn(
        AgentTurnRequest::new(request(), collaboration_plan(false), principal(), 0).with_silent_robot_actions(true)).await.unwrap();
    assert_eq!(result.model_steps, 1);
    assert!(result.output_text.is_empty());
    assert_eq!(silent_markers(&journal), [(1, vec!["delegate".into()])]);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
}

#[derive(Debug)]
struct SilentUnusedDiscovery;
#[async_trait]
impl crate::AgentToolDiscoveryPort for SilentUnusedDiscovery {
    async fn select(&self, _: &ChatCausality, _: u64, _: &str, _: &[crate::AgentToolDiscoveryCandidate],
        _: usize, _: CancellationToken) -> Result<Vec<String>, AgentEngineError> {
        panic!("this fixture uses already visible Robot tools")
    }
}

#[tokio::test]
async fn silent_robot_keeps_provider_batch_preference_while_discovery_guards_stay_atomic() {
    for enabled in [false, true] {
        let model = silent_model(vec![text_step("普通聊天。")]);
        let mut initial = request();
        initial.input.parallel_tool_calls = Some(true);
        input_session(model.clone(), Arc::new(SilentRobotTools::default()), Arc::new(SettlementJournal::default()))
            .run_turn(AgentTurnRequest::new(initial, silent_robot_plan(), principal(), 0)
                .with_silent_robot_actions(enabled).with_tool_discovery_port(Arc::new(SilentUnusedDiscovery))).await.unwrap();
        assert_eq!(model.requests.lock().unwrap()[0].input.parallel_tool_calls, Some(enabled));
    }
}

#[tokio::test]
async fn silent_robot_open_plan_cannot_be_closed_by_a_no_reply_declaration() {
    let model = silent_model(vec![plan_step("in_progress", "inspect"),
        silent_batch(&[("stand", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)]),
        vec![Err(ChatModelError::protocol_violation("controlled stop after rejection"))]]);
    let tools = Arc::new(SilentRobotTools::default());
    let journal = Arc::new(SettlementJournal::default());
    assert!(input_session(model, tools.clone(), journal.clone()).run_turn(silent_request()).await.is_err());
    assert!(tools.calls.lock().unwrap().is_empty());
    assert!(silent_markers(&journal).is_empty());
}

#[tokio::test]
async fn silent_robot_unfinished_plan_pauses_without_publishing_silent_success() {
    let model = silent_model(vec![plan_step("in_progress", "inspect"),
        silent_batch(&[("stand", "robot_motion"), ("finish", crate::silent_completion::TOOL_NAME)]),
        text_step("The remaining plan still needs intervention."),
        text_step("The plan has not been completed."),
        text_step("must not sample again")]);
    let tools = Arc::new(SilentRobotTools::default());
    let journal = Arc::new(SettlementJournal::default());
    let result = input_session(model, tools.clone(), journal.clone()).run_turn(silent_request()).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Paused { .. }));
    assert!(tools.calls.lock().unwrap().is_empty());
    assert!(silent_markers(&journal).is_empty());
}

#[tokio::test]
async fn silent_robot_recovery_does_not_invent_local_action_completion_evidence() {
    let journal = Arc::new(SettlementJournal::default());
    let tools = Arc::new(SilentRobotTools::default());
    let model = silent_model(vec![silent_batch(&[("accepted", "robot_motion")]),
        vec![Err(ChatModelError::protocol_violation("controlled stop at checkpoint"))]]);
    assert!(input_session(model, tools.clone(), journal.clone()).run_turn(silent_request()).await.is_err());
    let checkpoint = journal.checkpoints.lock().unwrap().last().unwrap().clone();
    let events = journal.events.lock().unwrap().clone();
    let through = events.iter().rposition(|event| matches!(event, AgentEngineEvent::ExecutionCheckpointSaved { .. })).unwrap() + 1;
    let revision = match &events[through-1] { AgentEngineEvent::ExecutionCheckpointSaved { revision, .. } => *revision, _ => unreachable!() };
    let recovery = crate::AgentTurnRecovery::new(checkpoint, revision, 1, events[..through].to_vec(), vec![], vec![]).unwrap();
    let resumed = silent_model(vec![vec![Err(ChatModelError::protocol_violation("inspect recovered surface only"))]]);
    let resumed_journal = Arc::new(SettlementJournal::default());
    assert!(input_session(resumed.clone(), tools.clone(), resumed_journal.clone())
        .run_turn(silent_request().with_recovery(recovery)).await.is_err());
    assert_eq!(*tools.calls.lock().unwrap(), ["accepted"]);
    let requests = resumed.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].input.tools.iter().any(|tool|tool.name==crate::silent_completion::TOOL_NAME));
    assert!(silent_markers(&resumed_journal).is_empty());
}
