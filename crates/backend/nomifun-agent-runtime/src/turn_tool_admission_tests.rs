struct ResultBudgetJournal {
    allow_first: bool,
    denied: CancellationToken,
    events: std::sync::Mutex<Vec<AgentEngineEvent>>,
    checkpoints: std::sync::Mutex<Vec<crate::AgentExecutionCheckpoint>>,
}

#[async_trait]
impl AgentEventSink for ResultBudgetJournal {
    fn supports_checkpoints(&self) -> bool { true }

    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    async fn admit_tool(&self, event: AgentEngineEvent) -> Result<crate::AgentToolAdmission, AgentEngineError> {
        let AgentEngineEvent::ToolStarted { call_id, .. } = &event else {
            return Err(AgentEngineError::InvalidContract("test admission requires ToolStarted".into()));
        };
        if self.allow_first && call_id.as_ref() == "call-1" {
            self.emit(event).await?;
            return Ok(crate::AgentToolAdmission::Admitted);
        }
        self.denied.cancel();
        Ok(crate::AgentToolAdmission::JournalBudget)
    }

    async fn execution_pressure(&self) -> Result<crate::AgentExecutionPressure, AgentEngineError> {
        Ok(crate::AgentExecutionPressure { renew_window:false,
            stop:self.denied.is_cancelled().then_some(crate::AgentExecutionStopReason::TurnJournalBudget) })
    }

    async fn save_checkpoint(&self, checkpoint: crate::AgentExecutionCheckpoint)
        -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
        checkpoint.validate()?;
        let mut checkpoints = self.checkpoints.lock().unwrap();
        let revision = checkpoints.len() as u64 + 1;
        let digest = nomifun_agent_contracts::digest_payload(&serde_json::to_value(&checkpoint).unwrap()).unwrap();
        let mut events = self.events.lock().unwrap();
        events.push(AgentEngineEvent::ExecutionCheckpointSaved { step:checkpoint.model_steps, revision, digest:digest.clone() });
        checkpoints.push(checkpoint);
        Ok(Some(crate::AgentCheckpointReceipt { revision, digest, through_seq:events.len() as u64 }))
    }
}

struct ReservedResultOwner {
    calls: std::sync::Mutex<Vec<String>>,
    wait_for_budget_refusal: bool,
    denied: CancellationToken,
    content: String,
}

#[async_trait]
impl AgentToolInvoker for ReservedResultOwner {
    async fn invoke(&self, invocation: AgentToolInvocation, _:CancellationToken) -> Result<AgentToolResult, AgentEngineError> {
        if let Some(result) = instruction_result(&invocation) { return Ok(result); }
        self.calls.lock().unwrap().push(invocation.call.call_id.as_ref().to_owned());
        if self.wait_for_budget_refusal { self.denied.cancelled().await; }
        Ok(AgentToolResult::text(invocation.call.call_id, json!({
            "path":invocation.call.arguments.0["path"], "content":self.content,
            "offset":0,"total_bytes":self.content.len(),"eof":true,"next_offset":null,
            "sha256":nomifun_agent_contracts::digest_bytes(self.content.as_bytes()),
        }).to_string(), false))
    }
}

#[tokio::test]
async fn journal_budget_refusal_preserves_admitted_results_and_pauses_without_invoking_or_retrying_held_calls() {
    for parallel in [false, true] {
        for allow_first in [false, true] {
            let denied = CancellationToken::new();
            let journal = Arc::new(ResultBudgetJournal { allow_first, denied:denied.clone(),
                events:Default::default(), checkpoints:Default::default() });
            let owner = Arc::new(ReservedResultOwner { calls:Default::default(),
                wait_for_budget_refusal:parallel, denied, content:"EXACT_OWNER_RESULT😀\\\"\n".repeat(4000) });
            let mut calls = Vec::new();
            for index in 1..=3 {
                let mut call = control_step(&format!("call-{index}"), "read_file", json!({"path":"a"}));
                call.pop();
                calls.extend(call);
            }
            calls.push(Ok(ChatModelEvent::Completed { finish_reason:ChatFinishReason::ToolCalls }));
            let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![calls]), requests:Default::default() });
            let plan = AgentToolPlan::new([tool_binding("read_file", "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, parallel)]).unwrap();
            let outcome = tokio::time::timeout(Duration::from_secs(5), run_turn(binding(), model.clone(), owner.clone(), journal.clone(),
                AgentTurnRequest::new(request(), plan, principal(), 0).with_max_model_steps(2)
                    .with_execution_segments(crate::AgentSegmentPolicy { max_segments:2, max_no_progress_segments:2 }),
                AgentContextBudget::default(), CancellationToken::new())).await.unwrap().unwrap();
            assert!(matches!(outcome.terminal, AgentTurnTerminal::Paused { reason } if reason=="EXECUTION_TURN_JOURNAL_BUDGET"));
            assert_eq!(model.requests.lock().unwrap().len(), 1, "budget refusal cannot buy another model attempt");
            let expected_calls = if allow_first { vec!["call-1".to_owned()] } else { vec![] };
            assert_eq!(*owner.calls.lock().unwrap(), expected_calls);
            let events = journal.events.lock().unwrap();
            let admitted = events.iter().filter_map(|event| match event {
                AgentEngineEvent::ToolStarted { step:1, call_id, .. } => Some(call_id.as_ref().to_owned()), _=>None,
            }).collect::<Vec<_>>();
            assert_eq!(admitted, expected_calls);
            assert_eq!(events.iter().filter(|event| matches!(event,AgentEngineEvent::PublicOutputCommitted { step:1,.. })).count(), usize::from(allow_first));
            let mut held = 0;
            for event in &*events {
                if let AgentEngineEvent::ToolCompleted { step:1, result, runtime_hold } = event {
                    if allow_first && result.call_id.as_ref()=="call-1" {
                        assert_eq!(*runtime_hold, None);
                        assert!(!result.is_error);
                        let output:serde_json::Value = serde_json::from_str(&result.output_text()).unwrap();
                        assert_eq!(output["content"], owner.content, "the admitted owner's complete result survives a sibling refusal");
                    } else {
                        assert_eq!(*runtime_hold, Some(crate::RuntimeToolHoldReason::RuntimeGate));
                        assert!(result.is_error);
                        let refusal:serde_json::Value = serde_json::from_str(&result.output_text()).unwrap();
                        assert_eq!(refusal["status"], "not_executed");
                        assert_eq!(refusal["code"], "NATIVE_JOURNAL_BUDGET");
                        held += 1;
                    }
                }
            }
            assert_eq!(held, if allow_first { 2 } else { 3 });
            if parallel {
                assert!(events.iter().any(|event| matches!(event,AgentEngineEvent::ToolResultsOrdered { step:1,call_ids }
                    if call_ids.iter().map(|id|id.as_ref()).collect::<Vec<_>>()==["call-1","call-2","call-3"])),
                    "budget refusals retain the original model proposal order beside admitted observations");
            }
            assert!(!events.iter().any(|event| matches!(event,AgentEngineEvent::ExecutionSegmentRenewed {..})), "the same budget is never silently increased");
            let checkpoints = journal.checkpoints.lock().unwrap();
            let checkpoint = checkpoints.last().unwrap();
            assert_eq!(checkpoint.work.failed_commands, 0);
            assert_eq!(checkpoint.work.successful_commands, 0);
            assert_eq!(checkpoint.work.failed_tools, held, "preserve the existing deferred-feedback count without claiming owner failures");
        }
    }
}
