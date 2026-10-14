#[tokio::test]
async fn user_input_two_real_turns_preserve_original_question_and_requirements_as_incomplete_history() {
    let answer = "Continue the PPT: my name is Alice.";
    let model = Arc::new(ObservingModel {requests:Default::default(),steps:std::sync::Mutex::new(vec![
        input_step("original-question"),
        control_step("resume","resume_task",json!({"turn_operation_id":"turn","source":{"input":0,"quote":answer}})),
        plan_step("in_progress",answer),
        input_step("fresh-question"),
    ])});
    let journal = Arc::new(InputJournal::default());
    let session = input_session(model.clone(),Arc::new(EchoTool),journal.clone());
    let first = session.run_turn(input_request(Default::default())).await.unwrap();
    assert!(matches!(first.terminal,AgentTurnTerminal::InputRequired{..}));
    let first_events = journal.journal.events.lock().unwrap().clone();
    let prior = crate::AgentPriorTask::from_closed_turn("turn",&first_events).unwrap().unwrap();
    assert_eq!(serde_json::to_value(&prior).unwrap()["terminal"],"input_required");
    let mut history = vec![];
    crate::replay_closed_turn(&mut history,request().input.messages[0].clone(),&first_events).unwrap();
    let mut followup = request();
    followup.causality.turn_operation_id = "turn-next".into();
    followup.causality.operation_id = "turn-next:model".into();
    followup.causality.causation_event_id = "answer-input".into();
    followup.input.messages = history;
    followup.input.messages.push(crate::context_lifecycle::text_message(ChatRole::User,answer.into()));
    let second = session.run_turn(AgentTurnRequest::new(followup,Default::default(),principal(),0)
        .with_prior_task(Some(prior)).with_user_input_request_port(Arc::new(InputPolicy::default()))).await.unwrap();
    assert!(matches!(second.terminal,AgentTurnTerminal::InputRequired{request} if request.request_id.as_ref()=="fresh-question"));
    assert_eq!(second.turn_operation_id.as_ref(),"turn-next");
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(),4);
    let first_followup = serde_json::to_string(&requests[1].input).unwrap();
    assert!(first_followup.contains("inspect") && first_followup.contains("original-question") && first_followup.contains(answer));
    let cp = journal.journal.checkpoints.lock().unwrap().last().unwrap().clone();
    assert!(cp.plan.is_open());
    assert!(cp.plan.requirements.iter().any(|requirement|requirement.origin.as_ref().is_some_and(|origin|origin.turn_operation_id=="turn")));
    assert!(cp.plan.requirements.iter().any(|requirement|requirement.source.quote==answer));
    let events = journal.journal.events.lock().unwrap();
    let next_events = &events[first_events.len()..];
    assert!(!next_events.iter().any(|event|matches!(event,AgentEngineEvent::TurnCompleted{..}|AgentEngineEvent::CompletionReported{..})));
    assert_eq!(next_events.iter().filter(|event|matches!(event,AgentEngineEvent::UserInputRequested{..})).count(),1);
    let mut corrupted = first_events;
    corrupted.retain(|event|!matches!(event,AgentEngineEvent::UserInputRequested{..}));
    assert!(crate::AgentPriorTask::from_closed_turn("turn",&corrupted).is_err());
}
