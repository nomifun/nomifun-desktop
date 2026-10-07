#[derive(Debug)]
struct VoiceAttemptTest {
    input:std::sync::Mutex<Option<crate::AgentSteeringInput>>,wake:Notify,
    requests:std::sync::Mutex<Vec<ChatModelRequest>>,
    workers:tokio::sync::Mutex<BTreeMap<OperationId,(CancellationToken,tokio::task::JoinHandle<()>)>>,
    proofs:tokio::sync::Mutex<BTreeMap<OperationId,nomifun_chat_model_broker::OwnedModelCleanupReceipt>>,
    model_opened:Notify,old_observed:Notify,events:std::sync::Mutex<Vec<AgentEngineEvent>>,
    fail_cleanup:bool,admitted_tool:bool,correct_at_admission:bool,
}
impl VoiceAttemptTest {
    fn new(fail_cleanup:bool)->Arc<Self>{Self::new_with_tool(fail_cleanup,false)}
    fn new_with_tool(fail_cleanup:bool,admitted_tool:bool)->Arc<Self>{Arc::new(Self {input:std::sync::Mutex::new(None),wake:Notify::new(),requests:std::sync::Mutex::new(vec![]),workers:Default::default(),proofs:Default::default(),model_opened:Notify::new(),old_observed:Notify::new(),events:Default::default(),fail_cleanup,admitted_tool,correct_at_admission:false})}
    fn correct(&self){*self.input.lock().unwrap()=Some(crate::AgentSteeringInput {receipt_operation_id:"voice-correction".into(),message_id:"voice-correction".into(),text:"also explain".into(),files:vec![],inject_skills:vec![],image_count:0,prepared_images:vec![],prepared_skill_instructions:vec![]});self.wake.notify_waiters();}
    async fn close(&self,operation:&OperationId)->Result<nomifun_chat_model_broker::OwnedModelCleanupReceipt,AgentEngineError>{
        if self.fail_cleanup{return Err(AgentEngineError::InvalidContract("unconfirmed producer cleanup".into()));}
        let mut proofs=self.proofs.lock().await;if let Some(proof)=proofs.get(operation){return Ok(proof.clone());}
        let mut workers=self.workers.lock().await;let (cancel,task)=workers.get_mut(operation).ok_or_else(||AgentEngineError::InvalidContract("unknown owned attempt".into()))?;
        cancel.cancel();let task_id=task.id().to_string();task.await.unwrap();
        let proof=nomifun_chat_model_broker::OwnedModelCleanupReceipt {operation_id:operation.clone(),task_id,stage:nomifun_chat_model_broker::OwnedModelCleanupStage::Producer,outcome:nomifun_chat_model_broker::OwnedModelCleanupOutcome::Joined};
        proofs.insert(operation.clone(),proof.clone());Ok(proof)
    }
}
#[derive(Clone,Debug)]struct VoiceTestModel(Arc<VoiceAttemptTest>);
#[async_trait]impl AgentModelPort for VoiceTestModel {
    async fn open_stream(&self,request:ChatModelRequest,cancellation:CancellationToken)->Result<AgentModelStream,ChatModelError>{
        let ordinal={let mut requests=self.0.requests.lock().unwrap();requests.push(request.clone());requests.len()};let operation=request.causality.operation_id.clone();
        let(sender,receiver)=tokio::sync::mpsc::channel(8);let child=cancellation.child_token();let token=child.clone();
        let admitted_tool=self.0.admitted_tool;
        let task=tokio::spawn(async move {
            if ordinal==1&&admitted_tool {
                sender.send(Ok(ChatModelEvent::ToolCallCompleted {call:ChatToolCall {call_id:"admitted-call".into(),name:"read_file".into(),arguments:nomifun_agent_contracts::StrictJsonValue(json!({"path":"README.md"})),provider_metadata:None}})).await.unwrap();
                sender.send(Ok(ChatModelEvent::Completed {finish_reason:ChatFinishReason::ToolCalls})).await.unwrap();
            }else if ordinal==1 {
                sender.send(Ok(ChatModelEvent::OutputTextDelta {text:"OBSOLETE_DRAFT".into()})).await.unwrap();
                sender.send(Ok(ChatModelEvent::ToolCallDelta {call_id:"withdrawn-call".into(),name:"read_file".into(),arguments_delta:"{".into()})).await.unwrap();
                token.cancelled().await;
                // A late completion from this old producer cannot become an
                // admitted call or reach the replacement step.
                let _=sender.send(Ok(ChatModelEvent::ToolCallCompleted {call:ChatToolCall {call_id:"withdrawn-call".into(),name:"read_file".into(),arguments:nomifun_agent_contracts::StrictJsonValue(json!({"path":"README.md"})),provider_metadata:None}})).await;
            }else{token.cancelled().await;}
        });
        self.0.workers.lock().await.insert(operation,(child,task));self.0.model_opened.notify_one();
        Ok(Box::pin(futures::stream::unfold(receiver,|mut receiver|async move{receiver.recv().await.map(|item|(item,receiver))})))
    }
}
#[async_trait]impl crate::AgentInputPort for VoiceAttemptTest {
    async fn take(&self,_:&ChatCausality,_:bool)->Result<Vec<crate::AgentSteeringInput>,AgentEngineError>{Ok(self.input.lock().unwrap().take().into_iter().collect())}
    async fn has_pending(&self,_:&ChatCausality)->Result<bool,AgentEngineError>{Ok(self.input.lock().unwrap().is_some())}
}
#[async_trait]impl crate::AgentImmediateCorrectionPort for VoiceAttemptTest {
    fn model_port(&self)->Arc<dyn AgentModelPort>{panic!("test scope must use its Arc-backed model adapter")}
    async fn wait(&self,_:&ChatCausality)->Result<Vec<String>,AgentEngineError>{loop{let notified=self.wake.notified();tokio::pin!(notified);notified.as_mut().enable();if self.input.lock().unwrap().is_some(){return Ok(vec!["voice-correction".into()]);}notified.await;}}
    async fn pending(&self,_:&ChatCausality)->Result<Vec<String>,AgentEngineError>{Ok(self.input.lock().unwrap().as_ref().map(|input|input.receipt_operation_id.clone()).into_iter().collect())}
    async fn has_admitted_tools(&self,_:&ChatCausality,step:u16)->Result<bool,AgentEngineError>{Ok(self.events.lock().unwrap().iter().any(|event|matches!(event,AgentEngineEvent::ToolStarted {step:current,..} if *current==step)))}
    async fn quiesce_model(&self,operation:&OperationId,_:tokio::time::Instant)->Result<nomifun_chat_model_broker::OwnedModelCleanupReceipt,AgentEngineError>{self.close(operation).await}
    async fn quiesce_all(&self,_:tokio::time::Instant)->Result<Vec<nomifun_chat_model_broker::OwnedModelCleanupReceipt>,AgentEngineError>{let keys=self.workers.lock().await.keys().cloned().collect::<Vec<_>>();let mut proofs=vec![];for key in keys{proofs.push(self.close(&key).await?);}Ok(proofs)}
}
#[derive(Debug)]struct VoiceTestPort(Arc<VoiceAttemptTest>);
#[async_trait]impl crate::AgentImmediateCorrectionPort for VoiceTestPort {
    fn model_port(&self)->Arc<dyn AgentModelPort>{Arc::new(VoiceTestModel(self.0.clone()))}
    async fn wait(&self,c:&ChatCausality)->Result<Vec<String>,AgentEngineError>{crate::AgentImmediateCorrectionPort::wait(self.0.as_ref(),c).await}
    async fn pending(&self,c:&ChatCausality)->Result<Vec<String>,AgentEngineError>{crate::AgentImmediateCorrectionPort::pending(self.0.as_ref(),c).await}
    async fn has_admitted_tools(&self,c:&ChatCausality,step:u16)->Result<bool,AgentEngineError>{crate::AgentImmediateCorrectionPort::has_admitted_tools(self.0.as_ref(),c,step).await}
    async fn quiesce_model(&self,o:&OperationId,d:tokio::time::Instant)->Result<nomifun_chat_model_broker::OwnedModelCleanupReceipt,AgentEngineError>{crate::AgentImmediateCorrectionPort::quiesce_model(self.0.as_ref(),o,d).await}
    async fn quiesce_all(&self,d:tokio::time::Instant)->Result<Vec<nomifun_chat_model_broker::OwnedModelCleanupReceipt>,AgentEngineError>{crate::AgentImmediateCorrectionPort::quiesce_all(self.0.as_ref(),d).await}
}
#[async_trait]impl AgentEventSink for VoiceAttemptTest {
    async fn emit(&self,event:AgentEngineEvent)->Result<(),AgentEngineError>{if matches!(event,AgentEngineEvent::ToolCallDelta {..}){self.old_observed.notify_one();}self.events.lock().unwrap().push(event);Ok(())}
    async fn admit_tool(&self,event:AgentEngineEvent)->Result<bool,AgentEngineError>{
        if self.correct_at_admission&&matches!(event,AgentEngineEvent::ToolStarted {step,..} if step>0){self.correct();return Ok(false);}
        self.emit(event).await?;Ok(true)
    }
}
#[tokio::test]
async fn voice_immediate_correction_joins_old_step_preserves_same_turn_and_excludes_obsolete_history_and_late_tool(){
    let scope=VoiceAttemptTest::new(false);let parent=CancellationToken::new();let cancel=parent.clone();let run_scope=scope.clone();
    let task=tokio::spawn(async move{run_turn(binding(),Arc::new(VoiceTestModel(run_scope.clone())),Arc::new(EchoTool),run_scope.clone(),
        AgentTurnRequest::new(request(),AgentToolPlan::default(),principal(),0).with_input_port(run_scope.clone()).with_voice_immediate_correction(Arc::new(VoiceTestPort(run_scope))),AgentContextBudget::default(),cancel).await});
    tokio::time::timeout(Duration::from_secs(2),scope.old_observed.notified()).await.unwrap();scope.correct();
    tokio::time::timeout(Duration::from_secs(2),async{loop{if scope.requests.lock().unwrap().len()>1{break;}scope.model_opened.notified().await;}}).await.unwrap();
    let requests=scope.requests.lock().unwrap().clone();assert_eq!(requests[0].causality.turn_operation_id,requests[1].causality.turn_operation_id);
    let encoded=serde_json::to_string(&requests[1].input.messages).unwrap();assert!(!encoded.contains("OBSOLETE_DRAFT"));assert!(encoded.contains("also explain"));
    assert!(scope.proofs.lock().await.contains_key(&requests[0].causality.operation_id),"the next model request is forbidden before the real old worker joined");
    parent.cancel();let result=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();
    assert_eq!(result.tool_call_count,0);assert!(!result.output_text.contains("OBSOLETE_DRAFT"));
    let events=scope.events.lock().unwrap().clone();assert_eq!(events.iter().filter(|event|matches!(event,AgentEngineEvent::TurnStarted {..})).count(),1);
    assert!(events.iter().any(|event|matches!(event,AgentEngineEvent::VoiceModelStepSuperseded {discarded_tool_call_ids,..} if discarded_tool_call_ids.iter().any(|id|id.as_ref()=="withdrawn-call"))));
    assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::ToolStarted {step,..} if *step>0)));
    crate::AgentImmediateCorrectionPort::quiesce_all(scope.as_ref(),tokio::time::Instant::now()+Duration::from_secs(1)).await.unwrap();
}
#[tokio::test]
async fn voice_immediate_unconfirmed_cleanup_cannot_publish_superseded_or_apply_or_open_successor(){
    let scope=VoiceAttemptTest::new(true);let cancel=CancellationToken::new();let stop=cancel.clone();let run_scope=scope.clone();
    let task=tokio::spawn(async move{run_turn(binding(),Arc::new(VoiceTestModel(run_scope.clone())),Arc::new(EchoTool),run_scope.clone(),
        AgentTurnRequest::new(request(),AgentToolPlan::default(),principal(),0).with_input_port(run_scope.clone()).with_voice_immediate_correction(Arc::new(VoiceTestPort(run_scope))),AgentContextBudget::default(),cancel).await});
    tokio::time::timeout(Duration::from_secs(2),scope.old_observed.notified()).await.unwrap();scope.correct();
    assert!(tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().is_err());stop.cancel();
    let events=scope.events.lock().unwrap();assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::VoiceModelStepSuperseded {..}|AgentEngineEvent::SteeringInputs {..})));
    assert_eq!(scope.requests.lock().unwrap().len(),1);drop(events);
    // The fixture models the host's explicit failure cleanup; no unknown proof
    // was upgraded to Applied. Reap its actual owned worker before test exit.
    let mut workers=scope.workers.lock().await;for(_,(_,task))in workers.iter_mut(){task.await.unwrap();}
}
#[test]
fn voice_immediate_none_keeps_original_model_request_bytes_and_has_no_policy_port(){
    let sample=request();let before=serde_json::to_vec(&sample).unwrap();let turn=AgentTurnRequest::new(sample,AgentToolPlan::default(),principal(),0);
    assert!(turn.voice_immediate.is_none());assert_eq!(serde_json::to_vec(&turn.model_request).unwrap(),before);
}

struct VoiceAdmittedTool {entered:Notify,release:Notify,settled:std::sync::atomic::AtomicBool}
#[async_trait]impl AgentToolInvoker for VoiceAdmittedTool {
    async fn invoke(&self,invocation:AgentToolInvocation,cancellation:CancellationToken)->Result<AgentToolResult,AgentEngineError>{
        if invocation.call.call_id.as_ref()=="admitted-call" {self.entered.notify_one();self.release.notified().await;assert!(!cancellation.is_cancelled(),"voice correction must not cancel an already admitted tool");self.settled.store(true,Ordering::Release);}
        EchoTool.invoke(invocation,cancellation).await
    }
}
#[tokio::test]
async fn voice_immediate_waits_for_already_admitted_tool_settlement_without_replay_or_parent_cancel(){
    let scope=VoiceAttemptTest::new_with_tool(false,true);let tools=Arc::new(VoiceAdmittedTool {entered:Notify::new(),release:Notify::new(),settled:std::sync::atomic::AtomicBool::new(false)});
    let parent=CancellationToken::new();let cancel=parent.clone();let run_scope=scope.clone();let run_tools=tools.clone();
    let task=tokio::spawn(async move{run_turn(binding(),Arc::new(VoiceTestModel(run_scope.clone())),run_tools,run_scope.clone(),
        AgentTurnRequest::new(request(),tool_plan(),principal(),0).with_input_port(run_scope.clone()).with_voice_immediate_correction(Arc::new(VoiceTestPort(run_scope))),AgentContextBudget::default(),cancel).await});
    tokio::time::timeout(Duration::from_secs(2),tools.entered.notified()).await.unwrap();scope.correct();
    assert_eq!(scope.requests.lock().unwrap().len(),1);assert!(!tools.settled.load(Ordering::Acquire));assert!(!parent.is_cancelled());tools.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2),async{loop{if scope.requests.lock().unwrap().len()>1{break;}scope.model_opened.notified().await;}}).await.unwrap();
    assert!(tools.settled.load(Ordering::Acquire));let requests=scope.requests.lock().unwrap().clone();
    assert!(requests[1].input.messages.iter().flat_map(|message|&message.content).any(|part|matches!(part,ChatContentPart::ToolResult {call_id,..} if call_id.as_ref()=="admitted-call")),"the actual settled result remains in context");
    parent.cancel();let result=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();assert_eq!(result.tool_call_count,1);
    assert!(!scope.events.lock().unwrap().iter().any(|event|matches!(event,AgentEngineEvent::VoiceModelStepSuperseded {..})),"already settled effects cannot be withdrawn as model proposals");
    crate::AgentImmediateCorrectionPort::quiesce_all(scope.as_ref(),tokio::time::Instant::now()+Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn voice_immediate_late_atomic_tool_denial_withdraws_the_step_without_fake_results_or_new_turn(){
    let mut scope=VoiceAttemptTest::new_with_tool(false,true);Arc::get_mut(&mut scope).unwrap().correct_at_admission=true;
    let parent=CancellationToken::new();let cancel=parent.clone();let run_scope=scope.clone();
    let task=tokio::spawn(async move{run_turn(binding(),Arc::new(VoiceTestModel(run_scope.clone())),Arc::new(EchoTool),run_scope.clone(),
        AgentTurnRequest::new(request(),tool_plan(),principal(),0).with_input_port(run_scope.clone()).with_voice_immediate_correction(Arc::new(VoiceTestPort(run_scope))),AgentContextBudget::default(),cancel).await});
    tokio::time::timeout(Duration::from_secs(2),async{loop{if scope.requests.lock().unwrap().len()>1{break;}scope.model_opened.notified().await;}}).await.unwrap();
    let requests=scope.requests.lock().unwrap().clone();assert_eq!(requests[0].causality.turn_operation_id,requests[1].causality.turn_operation_id);
    assert!(!serde_json::to_string(&requests[1].input.messages).unwrap().contains("admitted-call"));
    let events=scope.events.lock().unwrap().clone();assert!(events.iter().any(|event|matches!(event,AgentEngineEvent::VoiceModelStepSuperseded {..})));
    assert!(!events.iter().any(|event|matches!(event,AgentEngineEvent::ToolStarted {step,..}|AgentEngineEvent::ToolCompleted {step,..} if *step>0)),"an unadmitted proposal has neither an effect nor a fabricated result");
    parent.cancel();let result=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();assert_eq!(result.tool_call_count,0);
    crate::AgentImmediateCorrectionPort::quiesce_all(scope.as_ref(),tokio::time::Instant::now()+Duration::from_secs(1)).await.unwrap();
}
