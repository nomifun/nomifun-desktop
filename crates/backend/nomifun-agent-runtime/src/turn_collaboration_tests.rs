#[derive(Debug, Default)]
struct CollaborationTestPort {
    entered: Notify,
    ready: Notify,
    wait_calls: AtomicUsize,
    status_calls: AtomicUsize,
    messages: std::sync::Mutex<Vec<crate::AgentCollaborationMessage>>,
}

fn ready_children_snapshot(ids: &[String]) -> crate::AgentCollaborationSnapshot {
    crate::collaboration::test_snapshot(json!({
        "executions":ids.iter().map(|id|json!({"execution_id":id,"sequence":7,"owned_step_ids":["child"],
            "owned_terminal_step_ids":["child"],"children":[{"step_id":"child","status":"completed","attempt_id":"attempt",
                "terminal_receipt":{"execution_id":id,"execution_event_sequence":7,"step_id":"child","status":"completed"},
                "source":{"agent_session_id":"child-session","turn_operation_id":"child-turn","terminal_event_id":"child-terminal"},
                "workspace_change_sources":[],
                "result":"The child's exact recorded result."}]})).collect::<Vec<_>>(),"all_ready":true,"any_ready":true}))
}

#[derive(Debug)]
struct ReadyCollaborationPort;

#[derive(Debug, Default)]
struct UnavailableCollaborationPort {
    calls: AtomicUsize,
    status_calls: AtomicUsize,
    failed_status_calls: std::sync::Mutex<std::collections::BTreeSet<usize>>,
    entered: Notify,
    available: std::sync::atomic::AtomicBool,
    invalid_contract: bool,
}

#[async_trait]
impl crate::AgentCollaborationPort for UnavailableCollaborationPort {
    async fn status(&self, _: &ChatCausality, selection: &crate::AgentCollaborationSelection, _: CancellationToken)
        -> Result<crate::AgentCollaborationSnapshot, AgentEngineError> {
        let call=self.status_calls.fetch_add(1, Ordering::SeqCst)+1;
        if self.failed_status_calls.lock().unwrap().remove(&call) {
            return Err(AgentEngineError::ObservationUnavailable {message:"database is busy".into()});
        }
        Ok(ready_children_snapshot(&selection.execution_ids))
    }

    async fn wait(&self, _: &ChatCausality, request: &crate::AgentChildrenWait, _: CancellationToken)
        -> Result<crate::AgentCollaborationSnapshot, AgentEngineError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.invalid_contract {
            return Err(AgentEngineError::InvalidContract("exact child source was revoked".into()));
        }
        if !self.available.load(Ordering::SeqCst) {
            return Err(AgentEngineError::ObservationUnavailable { message: "connection acquisition timed out".into() });
        }
        Ok(ready_children_snapshot(&request.execution_ids))
    }

    async fn send(&self, _: &ChatCausality, _: &ToolCallId, _: &crate::AgentCollaborationMessage, _: CancellationToken)
        -> Result<StrictJsonValue, AgentEngineError> {
        unreachable!("read retry must not send another command")
    }
}

#[tokio::test]
async fn collaboration_read_unavailability_preserves_one_wait_turn_checkpoint_and_model_budget() {
    let model = silent_model(vec![collaboration_wait_step(),
        control_step("read-result", crate::collaboration::STATUS, json!({"execution_ids":["owned-execution"]})),
        text_step("Integrated the exact original result.")]);
    let port = Arc::new(UnavailableCollaborationPort::default());
    let journal = Arc::new(SettlementJournal::default());
    let session = Arc::new(input_session(model.clone(), Arc::new(EchoTool), journal.clone()));
    let running = session.clone();
    let turn_request = AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 0)
        .with_collaboration_port(port.clone()).with_max_model_steps(4);
    let task = tokio::spawn(async move { running.run_turn(turn_request).await });
    tokio::time::timeout(Duration::from_secs(2), port.entered.notified()).await.unwrap();
    assert!(session.is_turn_active().await);
    assert!(!task.is_finished());
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    let checkpoint = journal.checkpoints.lock().unwrap().last().cloned().unwrap();
    assert_eq!((checkpoint.model_steps, checkpoint.tool_call_count), (1, 1));
    assert_eq!(checkpoint.awaiting_children.as_ref().unwrap().execution_ids, ["owned-execution"]);
    port.available.store(true, Ordering::SeqCst);
    let result = tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert_eq!((result.model_steps, result.tool_call_count), (3, 2));
    assert!(port.calls.load(Ordering::SeqCst) >= 2);
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].causality.turn_operation_id, requests[2].causality.turn_operation_id);
    let events = journal.events.lock().unwrap();
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::AwaitingChildren { .. })).count(), 1);
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::ChildrenWaitResolved { .. })).count(), 1);
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. } | AgentEngineEvent::TurnPaused { .. })));
}

#[tokio::test]
async fn collaboration_user_stop_preempts_temporary_read_retry_without_waiting_for_backoff() {
    let model = silent_model(vec![collaboration_wait_step()]);
    let port = Arc::new(UnavailableCollaborationPort::default());
    let journal = Arc::new(SettlementJournal::default());
    let session = Arc::new(input_session(model.clone(), Arc::new(EchoTool), journal.clone()));
    let running = session.clone();
    let turn_request = AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 0).with_collaboration_port(port.clone());
    let task = tokio::spawn(async move { running.run_turn(turn_request).await });
    tokio::time::timeout(Duration::from_secs(2), port.entered.notified()).await.unwrap();
    assert!(session.cancel().await);
    let result = tokio::time::timeout(Duration::from_millis(250), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Cancelled));
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::ChildrenWaitResolved { .. })));
}

#[tokio::test]
async fn collaboration_contract_failure_is_not_retried_as_temporary_storage_contention() {
    let model = silent_model(vec![collaboration_wait_step()]);
    let port = Arc::new(UnavailableCollaborationPort { invalid_contract: true, ..Default::default() });
    let result = open_session(model.clone(), Arc::new(EchoTool)).run_turn(
        AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 0).with_collaboration_port(port.clone())).await;
    assert!(matches!(result, Err(AgentEngineError::InvalidContract(message)) if message == "exact child source was revoked"));
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn collaboration_read_retries_cover_wait_acceptance_and_result_consumption_without_reinvoking_tools() {
    let model = silent_model(vec![collaboration_wait_step(),
        control_step("read-result", crate::collaboration::STATUS, json!({"execution_ids":["owned-execution"]})),
        text_step("Verified and integrated the original child result.")]);
    let port = Arc::new(UnavailableCollaborationPort::default());
    port.available.store(true, Ordering::SeqCst);
    port.failed_status_calls.lock().unwrap().extend([1,3]);
    let journal = Arc::new(SettlementJournal::default());
    let session = input_session(model.clone(), Arc::new(EchoTool), journal.clone());
    let result = session.run_turn(AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 0)
        .with_collaboration_port(port.clone()).with_max_model_steps(4)).await.unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
    assert_eq!((result.model_steps, result.tool_call_count), (3,2));
    assert_eq!(port.status_calls.load(Ordering::SeqCst), 4);
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.requests.lock().unwrap().len(), 3);
    let events = journal.events.lock().unwrap();
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::ToolCallCompleted { .. })).count(), 2);
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::ToolStarted { .. })).count(), 0,
        "pure collaboration reads never acquire a new Kernel effect");
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::ToolCompleted { .. })).count(), 2);
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::AwaitingChildren { .. })).count(), 1);
    assert_eq!(events.iter().filter(|event| matches!(event, AgentEngineEvent::ChildrenWaitResolved { .. })).count(), 1);
    assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { .. })));
}
#[async_trait]
impl crate::AgentCollaborationPort for ReadyCollaborationPort {
    async fn status(&self, _: &ChatCausality, selection:&crate::AgentCollaborationSelection, _:CancellationToken)
        ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> { Ok(ready_children_snapshot(&selection.execution_ids)) }
    async fn wait(&self, _: &ChatCausality, request:&crate::AgentChildrenWait, _:CancellationToken)
        ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> { Ok(ready_children_snapshot(&request.execution_ids)) }
    async fn send(&self,_:&ChatCausality,_:&ToolCallId,_:&crate::AgentCollaborationMessage,_:CancellationToken)
        ->Result<StrictJsonValue,AgentEngineError> { unreachable!("this fixture does not send child guidance") }
}

fn read_ready_delegation() -> Vec<Result<ChatModelEvent, ChatModelError>> {
    control_step("read-delegated-result",crate::collaboration::STATUS,
        json!({"execution_ids":["0190f5fe-7c00-7a00-8000-000000000002"]}))
}

#[async_trait]
impl crate::AgentCollaborationPort for CollaborationTestPort {
    async fn status(&self, _: &ChatCausality, selection: &crate::AgentCollaborationSelection, _: CancellationToken)
        -> Result<crate::AgentCollaborationSnapshot, AgentEngineError> {
        assert!(!selection.execution_ids.is_empty());
        self.status_calls.fetch_add(1, Ordering::SeqCst);
        Ok(ready_children_snapshot(&selection.execution_ids))
    }
    async fn wait(&self, _: &ChatCausality, wait: &crate::AgentChildrenWait, cancellation: CancellationToken)
        -> Result<crate::AgentCollaborationSnapshot, AgentEngineError> {
        assert!(!wait.execution_ids.is_empty());
        self.wait_calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        tokio::select! { biased;
            _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
            _ = self.ready.notified() => {},
        }
        Ok(ready_children_snapshot(&wait.execution_ids))
    }
    async fn send(&self, _: &ChatCausality, _: &ToolCallId, message: &crate::AgentCollaborationMessage, _: CancellationToken)
        -> Result<StrictJsonValue, AgentEngineError> {
        self.messages.lock().unwrap().push(message.clone());
        Ok(StrictJsonValue(json!({"accepted":true,"execution_id":message.execution_id,"step_id":message.step_id,"idempotency_key":message.idempotency_key})))
    }
}

fn collaboration_wait_step() -> Vec<Result<ChatModelEvent, ChatModelError>> {
    control_step("wait-children", crate::collaboration::WAIT,
        json!({"execution_ids":["owned-execution"],"after_cursor":crate::AgentCollaborationSnapshot::observation_cursor(
            std::collections::BTreeMap::from([("owned-execution".into(),crate::AgentCollaborationObservation{state:"6".repeat(64),ready:"6".repeat(64)})])).unwrap(),"mode":"all"}))
}

#[derive(Debug,Default)]
struct DecisionBoundaryPort { waits:std::sync::Mutex<Vec<crate::AgentChildrenWait>> }

impl DecisionBoundaryPort {
    fn observation(ids:&[String])->crate::AgentCollaborationSnapshot {
        crate::collaboration::test_snapshot(json!({"executions":ids.iter().map(|id|json!({
            "execution_id":id,"sequence":12,"status":"waiting_input","owned_step_ids":["review-child"],
            "owned_terminal_step_ids":[],"children":[{"step_id":"review-child","attempt_id":"original-attempt",
                "status":"waiting_input","workspace_change_sources":[],"review_blocked":{"reason":"original_receipt_missing"}}]
        })).collect::<Vec<_>>(),"all_ready":false,"any_ready":true,"needs_attention":true,"can_progress":false}))
    }
}

#[async_trait]
impl crate::AgentCollaborationPort for DecisionBoundaryPort {
    async fn status(&self,_:&ChatCausality,selection:&crate::AgentCollaborationSelection,_:CancellationToken)
        ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {Ok(Self::observation(&selection.execution_ids))}
    async fn wait(&self,_:&ChatCausality,request:&crate::AgentChildrenWait,_:CancellationToken)
        ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {
        self.waits.lock().unwrap().push(request.clone());
        let mut snapshot=Self::observation(&request.execution_ids);
        if request.after_cursor.as_ref()==Some(&snapshot.cursor) {snapshot.result.0["wait_outcome"]=json!("decision_blocked");}
        Ok(snapshot)
    }
    async fn send(&self,_:&ChatCausality,_:&ToolCallId,_:&crate::AgentCollaborationMessage,_:CancellationToken)
        ->Result<StrictJsonValue,AgentEngineError> {unreachable!("waiting never submits a task command")}
}

#[tokio::test]
async fn repeated_wait_on_the_same_unowned_decision_pauses_without_another_model_or_fake_question() {
    let model=silent_model(vec![collaboration_wait_step(),
        control_step("read-review",crate::collaboration::STATUS,json!({"execution_ids":["owned-execution"]})),
        control_step("wait-same-review",crate::collaboration::WAIT,json!({"execution_ids":["owned-execution"],"mode":"all"}))]);
    let port=Arc::new(DecisionBoundaryPort::default());
    let journal=Arc::new(SettlementJournal::default());
    let session=input_session(model.clone(),Arc::new(EchoTool),journal.clone());
    let result=session.run_turn(AgentTurnRequest::new(request(),AgentToolPlan::default(),principal(),0)
        .with_collaboration_port(port.clone()).with_max_model_steps(4)).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Paused{ref reason} if reason=="EXECUTION_INTERVENTION_REQUIRED"));
    assert_eq!(result.model_steps,3);
    assert_eq!(model.requests.lock().unwrap().len(),3,"the same decision does not buy a fourth model sample");
    let waits=port.waits.lock().unwrap();assert_eq!(waits.len(),2);
    let current=DecisionBoundaryPort::observation(&["owned-execution".into()]);
    assert_eq!(waits[1].after_cursor.as_ref(),Some(&current.cursor),"omitted cursor uses the same latest source observation");
    drop(waits);
    let checkpoint=journal.checkpoints.lock().unwrap().last().cloned().unwrap();
    assert!(checkpoint.collaboration.pending_child_executions.contains("owned-execution"));
    assert!(checkpoint.awaiting_children.is_some(),"pause keeps the exact unresolved original wait");
    let events=journal.events.lock().unwrap();
    assert_eq!(events.iter().filter(|event|matches!(event,AgentEngineEvent::ChildrenWaitResolved{..})).count(),1);
    assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::UserInputRequested{..}|AgentEngineEvent::TurnInputRequired{..}|AgentEngineEvent::TurnCompleted{..})),
        "a technical decision boundary is neither a human question nor successful delivery");
    let (replayed,pending)=crate::collaboration::recorded_progress(&events).unwrap();
    assert_eq!(replayed,checkpoint.collaboration);
    assert_eq!(pending,checkpoint.awaiting_children);
}

#[derive(Debug)]
struct CandidateWaitPort {
    candidates:std::sync::Mutex<std::collections::VecDeque<crate::AgentCollaborationSnapshot>>,
    waits:std::sync::Mutex<Vec<crate::AgentChildrenWait>>,
}
#[async_trait]
impl crate::AgentCollaborationPort for CandidateWaitPort {
    async fn status(&self,_:&ChatCausality,_:&crate::AgentCollaborationSelection,_:CancellationToken)
        ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {unreachable!("the original wait subscribes without a new status tool")}
    async fn wait(&self,_:&ChatCausality,request:&crate::AgentChildrenWait,_:CancellationToken)
        ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {
        self.waits.lock().unwrap().push(request.clone());
        Ok(self.candidates.lock().unwrap().pop_front().expect("only the original read candidates are used"))
    }
    async fn send(&self,_:&ChatCausality,_:&ToolCallId,_:&crate::AgentCollaborationMessage,_:CancellationToken)
        ->Result<StrictJsonValue,AgentEngineError> {unreachable!("observation cannot submit another task command")}
}

#[tokio::test]
async fn removing_old_ready_sources_advances_only_the_local_subscription_and_preserves_real_input_or_blocked_wakes() {
    let ids=vec!["owned-execution".into()];
    let mut initial=DecisionBoundaryPort::observation(&ids);
    let terminal=ready_children_snapshot(&ids).result.0["executions"][0]["children"][0].clone();
    let row=&mut initial.result.0["executions"][0];
    row["owned_step_ids"]=json!(["review-child","child"]);row["owned_terminal_step_ids"]=json!(["child"]);
    row["children"].as_array_mut().unwrap().push(terminal);
    initial=crate::collaboration::test_snapshot(initial.result.0);
    let mut progress=crate::AgentCollaborationProgress::default();progress.pending_child_executions.insert(ids[0].clone());
    progress.observe_status(&initial).unwrap();
    let original=progress.clone();
    let wait=progress.wait().unwrap();
    let mut removed=initial.clone();
    let row=&mut removed.result.0["executions"][0];row["sequence"]=json!(13);row["owned_terminal_step_ids"]=json!([]);
    row["children"][1]["status"]=json!("pending");row["children"][1]["attempt_id"]=json!("accepted-new-attempt");
    removed=crate::collaboration::test_snapshot(removed.result.0);
    for outcome in ["parent_input_pending","decision_blocked"] {
        let mut next=removed.clone();next.result.0["wait_outcome"]=json!(outcome);
        let port=CandidateWaitPort {candidates:std::sync::Mutex::new(std::collections::VecDeque::from([removed.clone(),next])),waits:Default::default()};
        let result=crate::collaboration_wait::wait_for_children(&port,&request().causality,&wait,&progress,CancellationToken::new()).await.unwrap();
        assert_eq!(result.result.0["wait_outcome"],outcome,"real input and no-owner intervention cannot be swallowed by readiness deduplication");
        let waits=port.waits.lock().unwrap();assert_eq!(waits.len(),2);
        assert_eq!(waits[0],wait);assert_eq!(waits[1].after_cursor.as_ref(),Some(&removed.cursor));
        assert_eq!(progress,original,"quiet cursor advancement adds no canonical event, checkpoint mutation or second error ledger");
    }
}

#[tokio::test]
async fn collaboration_wait_parks_without_model_polling_then_reads_exact_results_in_the_same_turn() {
    let mut wait_step=collaboration_wait_step();
    wait_step.splice(0..0,[Ok(ChatModelEvent::OutputTextDelta {text:"Your follow-up is answered; ".into(),source:None}),
        Ok(ChatModelEvent::OutputTextDelta {text:"the original child is still running.".into(),source:None})]);
    let model = silent_model(vec![wait_step,
        control_step("children-result", crate::collaboration::STATUS, json!({"execution_ids":["owned-execution"]})),
        text_step("Integrated the recorded child result.")]);
    let port = Arc::new(CollaborationTestPort::default());
    let journal = Arc::new(SettlementJournal::default());
    let session = Arc::new(input_session(model.clone(), Arc::new(EchoTool), journal.clone()));
    let running = session.clone();
    let task_request = AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 0)
        .with_collaboration_port(port.clone()).with_max_model_steps(4);
    let task = tokio::spawn(async move { running.run_turn(task_request).await });
    tokio::time::timeout(Duration::from_secs(2), port.entered.notified()).await.unwrap();
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(session.is_turn_active().await);
    let checkpoint = journal.checkpoints.lock().unwrap().last().cloned().unwrap();
    assert_eq!(checkpoint.model_steps, 1);
    assert_eq!(checkpoint.tool_call_count, 1);
    assert_eq!(crate::AgentCollaborationSnapshot::observation_references(checkpoint.awaiting_children.as_ref().unwrap().after_cursor.as_ref().unwrap()).unwrap()["owned-execution"].state,"6".repeat(64));
    assert!(journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::AwaitingChildren { model_steps:1, .. })));
    {
        let events=journal.events.lock().unwrap();
        let returned=events.iter().position(|event|matches!(event,AgentEngineEvent::ToolCompleted {step:1,result,..}
            if result.call_id.as_ref()=="wait-children"&&!result.is_error)).unwrap();
        let committed=events.iter().position(|event|matches!(event,AgentEngineEvent::PublicOutputCommitted {step:1,source:None})).unwrap();
        let parked=events.iter().position(|event|matches!(event,AgentEngineEvent::AwaitingChildren {model_steps:1,..})).unwrap();
        assert!(returned<committed&&committed<parked,"only the settled native wait commits its completed public prefix before parking");
        assert_eq!(events.iter().filter(|event|matches!(event,AgentEngineEvent::PublicOutputCommitted {step:1,..})).count(),1);
    }
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event,
        AgentEngineEvent::TurnPaused {..} | AgentEngineEvent::TurnWaiting {..} | AgentEngineEvent::TurnInputRequired {..} | AgentEngineEvent::TurnCompleted {..})));
    port.ready.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert_eq!(result.model_steps, 3);
    assert_eq!(result.tool_call_count, 2);
    assert!(result.output_text.starts_with("Your follow-up is answered; the original child is still running."));
    assert!(matches!(result.terminal, AgentTurnTerminal::Completed {..}));
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests[0].causality.turn_operation_id, requests[2].causality.turn_operation_id);
    assert!(serde_json::to_string(&requests[2].input.messages).unwrap().contains("child-terminal"));
    drop(requests);
    assert!(journal.checkpoints.lock().unwrap().last().unwrap().awaiting_children.is_none());
    let mut history = Vec::new();
    crate::replay_closed_turn(&mut history, request().input.messages[0].clone(), &journal.events.lock().unwrap()).unwrap();
    assert!(serde_json::to_string(&history).unwrap().contains("Original collaboration owner wake"));
}

#[tokio::test]
async fn collaboration_wait_user_stop_preempts_the_owner_without_another_model_step() {
    let model = silent_model(vec![collaboration_wait_step()]);
    let port = Arc::new(CollaborationTestPort::default());
    let journal = Arc::new(SettlementJournal::default());
    let session = Arc::new(input_session(model.clone(), Arc::new(EchoTool), journal.clone()));
    let running = session.clone();
    let request = AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 0).with_collaboration_port(port.clone());
    let task = tokio::spawn(async move { running.run_turn(request).await });
    tokio::time::timeout(Duration::from_secs(2), port.entered.notified()).await.unwrap();
    assert!(session.cancel().await);
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal, AgentTurnTerminal::Cancelled));
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(!journal.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::ChildrenWaitResolved {..})));
}

#[tokio::test]
async fn collaboration_message_and_status_keep_their_original_exact_targets() {
    let port = Arc::new(CollaborationTestPort::default());
    let model = silent_model(vec![control_step("guide-child", crate::collaboration::MESSAGE,
        json!({"execution_id":"owned-execution","step_id":"child","content":"Keep the frozen module interface.","idempotency_key":"guide-once"})),
        control_step("read-guided-child",crate::collaboration::STATUS,json!({"execution_ids":["owned-execution"]})),
        text_step("Guidance was accepted by the owner.")]);
    let result = open_session(model, Arc::new(EchoTool)).run_turn(
        AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 0).with_collaboration_port(port.clone())).await.unwrap();
    assert_eq!(result.model_steps, 3);
    let messages = port.messages.lock().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].step_id, "child");
    assert_eq!(messages[0].idempotency_key, "guide-once");
}

#[tokio::test]
async fn collaboration_crash_wait_recovers_the_same_checkpoint_and_budget_without_redelegating() {
    let model = silent_model(vec![collaboration_wait_step()]);
    let port = Arc::new(CollaborationTestPort::default());
    let journal = Arc::new(SettlementJournal::default());
    let task_request = AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 0)
        .with_collaboration_port(port.clone()).with_max_model_steps(4);
    let original_request = task_request.clone();
    let task_journal = journal.clone();
    let task = tokio::spawn(async move { run_turn(binding(), model, Arc::new(EchoTool), task_journal,
        task_request, AgentContextBudget::default(), CancellationToken::new()).await });
    tokio::time::timeout(Duration::from_secs(2), port.entered.notified()).await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let checkpoint = journal.checkpoints.lock().unwrap().last().cloned().unwrap();
    assert!(checkpoint.awaiting_children.is_some());
    let events = journal.events.lock().unwrap().clone();
    let boundary = events.iter().rposition(|event| matches!(event, AgentEngineEvent::ExecutionCheckpointSaved {..})).unwrap() + 1;
    let revision = match &events[boundary-1] { AgentEngineEvent::ExecutionCheckpointSaved { revision, .. } => *revision, _ => unreachable!() };
    assert!(matches!(&events[boundary..],[AgentEngineEvent::AwaitingChildren {model_steps,request}]
        if *model_steps==checkpoint.model_steps&&Some(request)==checkpoint.awaiting_children.as_ref()),
        "the real crash image retains the original post-checkpoint WAIT observation");
    let recovery = crate::AgentTurnRecovery::new(checkpoint.clone(), revision, 1,
        events[..boundary].to_vec(), events[boundary..].to_vec(), vec![]).unwrap();
    assert_eq!(recovery.checkpoint(),&checkpoint,"pure WAIT validation preserves the whole original progress image");
    let mut resumed_steps = vec![control_step("read-recovered-children", crate::collaboration::STATUS,
        json!({"execution_ids":["owned-execution"]}))];
    resumed_steps.extend(completion_steps("inspect", &[], false));
    let resumed_model = silent_model(resumed_steps);
    let resumed_journal = journal.clone();
    let observed_model = resumed_model.clone();
    let resumed = tokio::spawn(async move { run_turn(binding(), resumed_model, Arc::new(EchoTool), resumed_journal,
        original_request.with_recovery(recovery), AgentContextBudget::default(), CancellationToken::new()).await });
    tokio::time::timeout(Duration::from_secs(2), port.entered.notified()).await.unwrap();
    assert!(observed_model.requests.lock().unwrap().is_empty());
    port.ready.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), resumed).await.unwrap().unwrap().unwrap();
    assert_eq!(result.model_steps, 4);
    assert_eq!(result.tool_call_count, 4);
    assert_eq!(port.wait_calls.load(Ordering::SeqCst), 2);
    assert_eq!(observed_model.requests.lock().unwrap().len(), 3);
    assert!(matches!(result.terminal,AgentTurnTerminal::Completed {..}));
    let mut history = Vec::new();
    crate::replay_closed_turn(&mut history, request().input.messages[0].clone(), &journal.events.lock().unwrap()).unwrap();
}

#[tokio::test]
async fn collaboration_premature_parent_final_waits_natively_and_requires_exact_results_before_completion() {
    struct OwnedDelegate;
    #[async_trait]
    impl AgentToolInvoker for OwnedDelegate {
        async fn invoke(&self, invocation:AgentToolInvocation, _:CancellationToken) ->Result<AgentToolResult,AgentEngineError> {
            assert_eq!(invocation.binding.action_id.as_ref(),"agent/delegate");
            Ok(AgentToolResult::text(invocation.call.call_id,json!({"execution_id":"owned-execution","status":"running","message":"Exact accepted delegation receipt"}).to_string(),false))
        }
    }
    let model = silent_model(vec![control_step("spawn-children","delegate",json!({"strategy":"parallel","tasks":[]})),
        text_step("Premature provider stop."),
        control_step("read-before-integrating",crate::collaboration::STATUS,json!({"execution_ids":["owned-execution"]})),
        text_step("The exact recorded child output is now integrated.")]);
    let port = Arc::new(CollaborationTestPort::default());
    let journal = Arc::new(SettlementJournal::default());
    let session = Arc::new(input_session(model.clone(),Arc::new(OwnedDelegate),journal.clone()));
    let running=session.clone();
    let task_request=AgentTurnRequest::new(request(),collaboration_plan(false),principal(),0)
        .with_collaboration_port(port.clone()).with_max_model_steps(4);
    let task=tokio::spawn(async move{running.run_turn(task_request).await});
    tokio::time::timeout(Duration::from_secs(2),port.entered.notified()).await.unwrap();
    assert_eq!(model.requests.lock().unwrap().len(),2,"a provider stop parks in the native owner, without model polling");
    assert!(session.is_turn_active().await);
    let checkpoint=journal.checkpoints.lock().unwrap().last().cloned().unwrap();
    assert!(checkpoint.collaboration.pending_child_executions.contains("owned-execution"));
    assert!(checkpoint.awaiting_children.is_some());
    assert!(!journal.events.lock().unwrap().iter().any(|event|matches!(event,AgentEngineEvent::TurnCompleted{..})));
    port.ready.notify_one();
    let result=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Completed{..}));
    assert_eq!(result.model_steps,4);
    let requests=model.requests.lock().unwrap();
    assert_eq!(requests[2].input.tools.iter().map(|tool|tool.name.as_str()).collect::<Vec<_>>(),[crate::collaboration::STATUS]);
    drop(requests);
    assert!(journal.checkpoints.lock().unwrap().last().unwrap().collaboration.pending_child_executions.is_empty());
    let mut history=Vec::new();
    crate::replay_closed_turn(&mut history,request().input.messages[0].clone(),&journal.events.lock().unwrap()).unwrap();
}

#[tokio::test]
async fn collaboration_verified_human_reply_carries_only_prior_references_into_its_new_turn() {
    let mut prior=crate::AgentCollaborationProgress::default();
    prior.pending_child_executions.insert("owned-execution".into());
    let model=silent_model(vec![control_step("read-after-human-reply",crate::collaboration::STATUS,
        json!({"execution_ids":["owned-execution"]})),text_step("The actual human reply and original child result are integrated.")]);
    let journal=Arc::new(SettlementJournal::default());
    let result=input_session(model.clone(),Arc::new(EchoTool),journal.clone()).run_turn(
        AgentTurnRequest::new(request(),AgentToolPlan::default(),principal(),0)
            .with_collaboration_port(Arc::new(ReadyCollaborationPort)).with_prior_collaboration(prior.clone())).await.unwrap();
    assert_eq!(result.model_steps,2,"a real human reply starts a normal new Turn budget");
    let events=journal.events.lock().unwrap().clone();
    assert!(matches!(&events[1],AgentEngineEvent::CollaborationInherited{progress} if progress==&prior));
    let progress=crate::collaboration_progress_from_events(&events).unwrap();
    assert!(progress.pending_child_executions.is_empty());
    assert_eq!(progress.results["owned-execution"].source_digests.len(),1);
    let mut forged=events.clone();
    forged.push(AgentEngineEvent::CollaborationInherited{progress:prior});
    assert!(crate::collaboration_progress_from_events(&forged).is_err());
    let mut history=Vec::new();
    crate::replay_closed_turn(&mut history,request().input.messages[0].clone(),&events).unwrap();
}

#[test]
fn collaboration_progress_reader_ignores_execution_shaped_text_from_other_tools() {
    let call=ChatToolCall {call_id:"read".into(),name:"read_file".into(),provider_metadata:None,arguments:StrictJsonValue(json!({"path":"data.json"}))};
    let mut events=vec![AgentEngineEvent::TurnStarted{binding:binding(),turn_operation_id:"turn".into()},
        AgentEngineEvent::ToolCallCompleted{step:1,call},
        AgentEngineEvent::ToolStarted{step:1,call_id:"read".into(),capability_id:"workspace.files".into(),action_id:"workspace.files/read".into()},
        AgentEngineEvent::ToolCompleted{step:1,runtime_hold:None,result:AgentToolResult::text("read".into(),
            json!({"execution_id":"owned-execution","status":"running","message":"Exact accepted delegation receipt"}).to_string(),false)}];
    assert!(crate::collaboration_progress_from_events(&events).unwrap().pending_child_executions.is_empty());
    events[2]=AgentEngineEvent::ToolStarted{step:1,call_id:"read".into(),capability_id:"agent.collaboration".into(),action_id:"agent/delegate".into()};
    assert!(crate::collaboration_progress_from_events(&events).unwrap().pending_child_executions.contains("owned-execution"));
}

#[tokio::test]
async fn collaboration_completion_report_cannot_close_an_unread_child_obligation() {
    let mut reads=control_step("inspect-a","read_file",json!({"path":"a"}));
    reads.insert(1,Ok(ChatModelEvent::ToolCallCompleted{call:ChatToolCall{call_id:"inspect-b".into(),name:"read_file".into(),
        arguments:StrictJsonValue(json!({"path":"b"})),provider_metadata:None}}));
    let mut steps=vec![reads,control_step("spawn","delegate",json!({"strategy":"planned","goal":"work"})),
        plan_step("completed","inspect"),control_step("premature-report",crate::completion::TOOL_NAME,
            json!({"summary":"Premature completion proposal.","criteria":[{"requirement_ids":["request"],"disposition":"unverified","rationale":"Child results are not yet consumed."}]})),
        read_ready_delegation()];
    steps.push(completion_steps_after_tool_errors("inspect",&[],false,1).pop().unwrap());
    let model=silent_model(steps);
    let port=Arc::new(CollaborationTestPort::default());
    let journal=Arc::new(SettlementJournal::default());
    let plan=AgentToolPlan::new([
        tool_binding("read_file","workspace.files","workspace.files/read",AgentEffectClass::ReadOnly,true),
        tool_binding("delegate","agent.collaboration","agent/delegate",AgentEffectClass::ManagedEffect,false),
    ]).unwrap();
    let session=Arc::new(input_session(model.clone(),Arc::new(EchoTool),journal.clone()));
    let running=session.clone();
    let task_request=AgentTurnRequest::new(request(),plan,principal(),0).with_collaboration_port(port.clone());
    let task=tokio::spawn(async move{running.run_turn(task_request).await});
    tokio::time::timeout(Duration::from_secs(2),port.entered.notified()).await.unwrap();
    assert_eq!(model.requests.lock().unwrap().len(),4);
    assert!(!journal.events.lock().unwrap().iter().any(|event|matches!(event,
        AgentEngineEvent::CompletionReported{..}|AgentEngineEvent::TurnCompleted{..})));
    port.ready.notify_one();
    let result=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Completed{..}));
    assert_eq!(result.model_steps,6);
}

#[tokio::test]
async fn explicit_delegation_is_forced_once_with_all_tools_then_returns_to_auto() {
    let plan=AgentToolPlan::new([
        tool_binding("delegate","agent.collaboration","agent/delegate",AgentEffectClass::ManagedEffect,false),
        tool_binding("read_file","workspace.files","workspace.files/read",AgentEffectClass::ReadOnly,true),
    ]).unwrap();
    let model=silent_model(vec![control_step("spawn-required","delegate",json!({"strategy":"planned","goal":"work"})),
        read_ready_delegation(),text_step("The exact delegated result is integrated.")]);
    let result=input_session(model.clone(),Arc::new(EchoTool),Arc::new(SettlementJournal::default())).run_turn(
        AgentTurnRequest::new(request(),plan,principal(),0).with_collaboration_port(Arc::new(ReadyCollaborationPort))
            .with_required_delegation(Some("delegate".into()))).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Completed{..}));
    let requests=model.requests.lock().unwrap();
    assert_eq!(requests[0].input.tool_choice,ChatToolChoice::Specific{name:"delegate".into()});
    assert!(requests[0].input.tools.iter().any(|tool|tool.name=="read_file"),"one-shot dispatch must retain the complete authorized tool surface");
    assert_eq!(requests[1].input.tool_choice,ChatToolChoice::Auto);
    assert_eq!(requests[2].input.tool_choice,ChatToolChoice::Auto);
}

#[tokio::test]
async fn explicit_delegation_cannot_be_ignored_by_a_plain_final_and_verified_prior_does_not_reforce() {
    let model=silent_model(vec![text_step("Ignored the required tool."),
        control_step("actual-required","delegate",json!({"strategy":"planned","goal":"work"})),
        read_ready_delegation(),text_step("Integrated after actual delegation.")]);
    let journal=Arc::new(SettlementJournal::default());
    let result=input_session(model.clone(),Arc::new(EchoTool),journal.clone()).run_turn(
        AgentTurnRequest::new(request(),collaboration_plan(false),principal(),0)
            .with_collaboration_port(Arc::new(ReadyCollaborationPort)).with_required_delegation(Some("delegate".into()))).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Completed{..}));
    assert_eq!(result.model_steps,4);
    let prior=crate::collaboration_progress_from_events(&journal.events.lock().unwrap()).unwrap();
    let continuation=silent_model(vec![text_step("Already accepted and read delegated work remains integrated.")]);
    input_session(continuation.clone(),Arc::new(EchoTool),Arc::new(SettlementJournal::default())).run_turn(
        AgentTurnRequest::new(request(),collaboration_plan(false),principal(),0)
            .with_collaboration_port(Arc::new(ReadyCollaborationPort)).with_prior_collaboration(prior)
            .with_required_delegation(Some("delegate".into()))).await.unwrap();
    assert_eq!(continuation.requests.lock().unwrap()[0].input.tool_choice,ChatToolChoice::Auto);
}

#[tokio::test]
async fn failed_owner_attempt_removes_the_one_shot_force_without_automatic_resend() {
    struct FailedDelegate;
    #[async_trait]
    impl AgentToolInvoker for FailedDelegate {
        async fn invoke(&self,invocation:AgentToolInvocation,_:CancellationToken)->Result<AgentToolResult,AgentEngineError> {
            Ok(AgentToolResult::text(invocation.call.call_id,"Owner refused the delegation.",true))
        }
    }
    let model=silent_model(vec![control_step("refused","delegate",json!({"strategy":"planned","goal":"work"})),
        text_step("Delegation could not be accepted.")]);
    input_session(model.clone(),Arc::new(FailedDelegate),Arc::new(SettlementJournal::default())).run_turn(
        AgentTurnRequest::new(request(),collaboration_plan(false),principal(),0)
            .with_collaboration_port(Arc::new(ReadyCollaborationPort)).with_required_delegation(Some("delegate".into()))).await.unwrap();
    assert_eq!(model.requests.lock().unwrap()[1].input.tool_choice,ChatToolChoice::Auto);
}

#[tokio::test]
async fn a_partial_terminal_result_requires_next_status_page_instead_of_waiting_or_finishing() {
    #[derive(Debug)]
    struct PagedPort;
    #[async_trait]
    impl crate::AgentCollaborationPort for PagedPort {
        async fn status(&self,_:&ChatCausality,selection:&crate::AgentCollaborationSelection,_:CancellationToken)
            ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {
            let mut snapshot=ready_children_snapshot(&selection.execution_ids);
            let child=&mut snapshot.result.0["executions"][0]["children"][0];
            let source=nomifun_agent_contracts::digest_payload(&json!({"attempt_id":child["attempt_id"],"source":child["source"]})).unwrap();
            let offset=selection.result_page.as_ref().map_or(0,|page|page.offset);
            child["result"]=json!(if offset==0 {"中文"}else{"abc"});
            child["result_page"]=json!({"source_digest":source,"sha256":nomifun_agent_contracts::digest_bytes("中文abc".as_bytes()),
                "total_bytes":9,"offset":offset,"next_offset":(offset==0).then_some(6)});
            Ok(snapshot)
        }
        async fn wait(&self,_:&ChatCausality,_:&crate::AgentChildrenWait,_:CancellationToken)
            ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {panic!("all children are terminal; remaining result bytes must be read, not waited for");}
        async fn send(&self,_:&ChatCausality,_:&ToolCallId,_:&crate::AgentCollaborationMessage,_:CancellationToken)
            ->Result<StrictJsonValue,AgentEngineError>{unreachable!()}
    }
    let source=nomifun_agent_contracts::digest_payload(&json!({"attempt_id":"attempt","source":{
        "agent_session_id":"child-session","turn_operation_id":"child-turn","terminal_event_id":"child-terminal"}})).unwrap();
    let model=silent_model(vec![control_step("spawn-paged","delegate",json!({"strategy":"planned","goal":"work"})),
        read_ready_delegation(),text_step("Premature partial result integration."),
        control_step("read-next-bytes",crate::collaboration::STATUS,json!({"execution_ids":["0190f5fe-7c00-7a00-8000-000000000002"],
            "step_ids":["child"],"result_page":{"execution_id":"0190f5fe-7c00-7a00-8000-000000000002","step_id":"child","source_digest":source,"offset":6}})),
        text_step("The full exact result is integrated.")]);
    let result=input_session(model,Arc::new(EchoTool),Arc::new(SettlementJournal::default())).run_turn(
        AgentTurnRequest::new(request(),collaboration_plan(false),principal(),0).with_collaboration_port(Arc::new(PagedPort))).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Completed{..}));
    assert_eq!(result.model_steps,5);
}

#[tokio::test]
async fn inherited_pending_work_can_observe_explicit_deletion_and_continue_without_redelegation() {
    #[derive(Debug)] struct DeletedPort;
    #[async_trait]
    impl crate::AgentCollaborationPort for DeletedPort {
        async fn status(&self,_:&ChatCausality,_:&crate::AgentCollaborationSelection,_:CancellationToken)
            ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {
            Ok(crate::collaboration::test_snapshot(json!({"executions":[{
                "execution_id":"owned-execution","sequence":9,"status":"cancelled","deleted":true,"user_revoked":true,"outputs_available":false,
                "owned_step_ids":[],"owned_terminal_step_ids":[],"children":[],"execution_terminal_receipt":{
                    "execution_id":"owned-execution","execution_event_sequence":9,"status":"cancelled","deleted":true,"event_type":"deleted"}}]})))
        }
        async fn wait(&self,_:&ChatCausality,_:&crate::AgentChildrenWait,_:CancellationToken)
            ->Result<crate::AgentCollaborationSnapshot,AgentEngineError>{panic!("revoked delegated work must never be waited for or restarted");}
        async fn send(&self,_:&ChatCausality,_:&ToolCallId,_:&crate::AgentCollaborationMessage,_:CancellationToken)
            ->Result<StrictJsonValue,AgentEngineError>{unreachable!()}
    }
    let mut prior=crate::AgentCollaborationProgress::default();prior.pending_child_executions.insert("owned-execution".into());
    let model=silent_model(vec![control_step("read-revocation",crate::collaboration::STATUS,json!({"execution_ids":["owned-execution"]})),
        text_step("The delegated work was explicitly deleted. Its result is unavailable; the original task remains unverified.")]);
    let journal=Arc::new(SettlementJournal::default());
    let result=input_session(model.clone(),Arc::new(EchoTool),journal.clone()).run_turn(
        AgentTurnRequest::new(request(),collaboration_plan(false),principal(),0).with_collaboration_port(Arc::new(DeletedPort))
            .with_prior_collaboration(prior).with_required_delegation(Some("delegate".into()))).await.unwrap();
    assert_eq!(result.model_steps,2);
    assert!(result.output_text.contains("unverified"));
    assert!(crate::collaboration_progress_from_events(&journal.events.lock().unwrap()).unwrap().pending_child_executions.is_empty());
    assert!(model.requests.lock().unwrap().iter().all(|request|request.input.tool_choice==ChatToolChoice::Auto));
}

#[derive(Debug)]
struct RejectedChildCommandPort {calls:AtomicUsize,corrupt:bool}

#[async_trait]
impl crate::AgentCollaborationPort for RejectedChildCommandPort {
    async fn status(&self,_:&ChatCausality,selection:&crate::AgentCollaborationSelection,_:CancellationToken)
        ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {Ok(ready_children_snapshot(&selection.execution_ids))}
    async fn wait(&self,_:&ChatCausality,_:&crate::AgentChildrenWait,_:CancellationToken)
        ->Result<crate::AgentCollaborationSnapshot,AgentEngineError> {unreachable!("a rejected command does not create a child wait")}
    async fn send(&self,_:&ChatCausality,_:&ToolCallId,_:&crate::AgentCollaborationMessage,_:CancellationToken)
        ->Result<StrictJsonValue,AgentEngineError> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        if self.corrupt {return Err(AgentEngineError::InvalidContract("original source proof is corrupt".into()));}
        Err(AgentEngineError::CollaborationCommandRejected {
            code:nomifun_agent_contracts::AgentCollaborationCommandRejectCode::ChildRequiresInput,
            message:"the original child is waiting for its exact question reply".into(),
            observation:Some(StrictJsonValue(json!({"status":"waiting_input","question_reply_reference":{
                "agent_session_id":"child-session","turn_operation_id":"child-turn","question_event_id":"question-event","request_id":"question-call"}}))),
        })
    }
}

#[tokio::test]
async fn ordinary_child_command_state_rejection_is_a_tool_result_and_does_not_fail_the_parent() {
    let model=silent_model(vec![control_step("stale-child-guidance",crate::collaboration::MESSAGE,
        json!({"execution_id":"owned-execution","step_id":"child","content":"Follow the original module boundary.","idempotency_key":"original-guide-once"})),
        text_step("The guidance was rejected while the exact child question awaits a reply.")]);
    let port=Arc::new(RejectedChildCommandPort{calls:AtomicUsize::new(0),corrupt:false});
    let journal=Arc::new(SettlementJournal::default());
    let session=input_session(model.clone(),Arc::new(EchoTool),journal.clone());
    let result=session.run_turn(AgentTurnRequest::new(request(),AgentToolPlan::default(),principal(),0).with_collaboration_port(port.clone())).await.unwrap();
    assert!(matches!(result.terminal,AgentTurnTerminal::Completed{..}));
    assert_eq!(port.calls.load(Ordering::SeqCst),1);assert_eq!(model.requests.lock().unwrap().len(),2);
    let events=journal.events.lock().unwrap();
    let output=events.iter().find_map(|event|match event {AgentEngineEvent::ToolCompleted{result,..} if result.call_id.as_ref()=="stale-child-guidance"=>Some(result),_=>None}).unwrap();
    assert!(output.is_error);let body:serde_json::Value=serde_json::from_str(&output.output_text()).unwrap();
    assert_eq!(body["accepted"],false);assert_eq!(body["code"],"CHILD_REQUIRES_INPUT");
    assert!(body["observation"]["question_reply_reference"]["question_event_id"].is_string());
    assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::TurnFailed{..}|AgentEngineEvent::AwaitingChildren{..})));
}

#[tokio::test]
async fn corrupt_child_command_source_is_not_downgraded_to_an_ordinary_state_rejection() {
    let model=silent_model(vec![control_step("corrupt-source-guidance",crate::collaboration::MESSAGE,
        json!({"execution_id":"owned-execution","step_id":"child","content":"Original context.","idempotency_key":"original-guide-once"}))]);
    let port=Arc::new(RejectedChildCommandPort{calls:AtomicUsize::new(0),corrupt:true});
    let result=open_session(model.clone(),Arc::new(EchoTool)).run_turn(
        AgentTurnRequest::new(request(),AgentToolPlan::default(),principal(),0).with_collaboration_port(port.clone())).await;
    assert!(matches!(result,Err(AgentEngineError::InvalidContract(message)) if message=="original source proof is corrupt"));
    assert_eq!(port.calls.load(Ordering::SeqCst),1);assert_eq!(model.requests.lock().unwrap().len(),1);
}
