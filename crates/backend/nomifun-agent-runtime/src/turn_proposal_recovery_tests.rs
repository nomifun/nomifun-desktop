mod proposal_recovery_tests {
    use super::*;
    use nomifun_chat_model_broker::{ChatUsage, ToolArgumentDiagnosticCategory};

    #[derive(Default)]
    struct Sink {
        events: std::sync::Mutex<Vec<AgentEngineEvent>>,
        reject_write: bool,
        reject_validation_write: bool,
        cancel_on_rejection: Option<CancellationToken>,
    }

    #[async_trait]
    impl AgentEventSink for Sink {
        async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
            if self.reject_validation_write && matches!(event, AgentEngineEvent::ModelResponseValidated { .. }) {
                return Err(AgentEngineError::EventSink("validation write failed".into()));
            }
            if matches!(event, AgentEngineEvent::ModelResponseRejected { .. }) {
                if self.reject_write {
                    return Err(AgentEngineError::EventSink("rejection write failed".into()));
                }
                if let Some(token) = &self.cancel_on_rejection {
                    token.cancel();
                }
            }
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Tools(std::sync::Mutex<Vec<String>>);

    #[async_trait]
    impl AgentToolInvoker for Tools {
        async fn invoke(&self, invocation: AgentToolInvocation, _: CancellationToken)
            -> Result<AgentToolResult, AgentEngineError> {
            if let Some(result) = instruction_result(&invocation) {
                return Ok(result);
            }
            self.0.lock().unwrap().push(invocation.call.call_id.as_ref().into());
            Ok(workspace_result(invocation))
        }
    }

    fn malformed(id: &str, finish_reason: ChatFinishReason) -> Vec<Result<ChatModelEvent, ChatModelError>> {
        let raw = "{\"path\":\"b\"";
        vec![
            Ok(ChatModelEvent::ToolCallDelta {
                call_id: id.into(), name: "read_file".into(), arguments_delta: raw.into(),
            }),
            Ok(ChatModelEvent::ToolCallRejected {
                call_id: id.into(), name: "read_file".into(),
                diagnostic: ToolArgumentDiagnostic {
                    category: ToolArgumentDiagnosticCategory::IncompleteJson,
                    line: Some(1), column: Some(raw.len() as u32), byte_len: raw.len() as u32,
                    digest: digest_bytes(raw.as_bytes()),
                },
            }),
            Ok(ChatModelEvent::Usage { usage: ChatUsage { input_tokens: 10, output_tokens: 4, ..Default::default() } }),
            Ok(ChatModelEvent::Completed { finish_reason }),
        ]
    }

    fn model(steps: Vec<Vec<Result<ChatModelEvent, ChatModelError>>>) -> Arc<ObservingModel> {
        Arc::new(ObservingModel { steps: std::sync::Mutex::new(steps), requests: Default::default() })
    }

    fn checkpoint() -> crate::AgentExecutionCheckpoint {
        crate::AgentExecutionCheckpoint {
            version: 1, binding: binding(), turn_operation_id: request().causality.turn_operation_id,
            active_set_generation: 0, model_steps: 0, tool_call_count: 0, accepted_input_count: 1,
            applied_steering_receipts: vec![], plan: Default::default(), work: Default::default(),
            patch_recovery: Default::default(), segments: None, control_rejections: Default::default(),
            completion_review: Default::default(), pending_user_input: None, awaiting_children: None, collaboration: Default::default(),
        }
    }

    fn checkpoint_prefix(checkpoint: &crate::AgentExecutionCheckpoint) -> Vec<AgentEngineEvent> {
        vec![AgentEngineEvent::TurnStarted {
            binding: checkpoint.binding.clone(), turn_operation_id: checkpoint.turn_operation_id.clone(),
        }, AgentEngineEvent::ExecutionCheckpointSaved {
            step: 0, revision: 1, digest: "c".repeat(64).into(),
        }]
    }

    #[tokio::test]
    async fn mixed_native_batch_is_rejected_before_any_tool_and_corrected_in_same_turn() {
        let mut first = control_step("valid-prefix", "read_file", json!({"path":"a"}));
        first.pop();
        first.extend(malformed("invalid-json", ChatFinishReason::ToolCalls));
        let model = model(vec![first,
            control_step("corrected", "read_file", json!({"path":"b"})), text_step("Done")]);
        let sink = Arc::new(Sink::default());
        let tools = Arc::new(Tools::default());
        let original = request();
        let original_root = original.causality.causation_event_id.clone();
        let requirement = original.input.messages[0].clone();
        let result = run_turn(binding(), model.clone(), tools.clone(), sink.clone(),
            AgentTurnRequest::new(original, tool_plan(), principal(), 0),
            AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(*tools.0.lock().unwrap(), ["corrected"]);
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| request.causality.causation_event_id == original_root));
        assert_ne!(requests[0].causality.operation_id, requests[1].causality.operation_id);
        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::ModelStepStarted {
            step: 2, correction_source: Some(1), ..
        })));
        assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::ModelResponseValidated { step: 2 })));
        assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::Usage { step: 1, .. })));
        assert!(!events.iter().any(|event| matches!(event,
            AgentEngineEvent::ToolStarted { step: 1, .. } | AgentEngineEvent::ToolCompleted { step: 1, .. })));
        let rejection = events.iter().find(|event| matches!(event, AgentEngineEvent::ModelResponseRejected { step: 1, .. })).unwrap();
        let AgentEngineEvent::ModelResponseRejected { discarded_tool_call_ids, feedback: Some(feedback), .. } = rejection else { panic!("complete rejection missing"); };
        assert_eq!(discarded_tool_call_ids, &[ToolCallId::from("valid-prefix"), ToolCallId::from("invalid-json")]);
        assert!(requests[1].input.messages.iter().any(|message| message.content.iter().any(|part|
            matches!(part, ChatContentPart::Text { text } if text == feedback))));
        let mut replayed = Vec::new();
        crate::replay_closed_turn(&mut replayed, requirement, &events).unwrap();
        assert!(replayed.iter().any(|message| message.content.iter().any(|part|
            matches!(part, ChatContentPart::Text { text } if text == feedback))));
        assert!(!replayed.iter().flat_map(|message| &message.content).any(|part| matches!(part,
            ChatContentPart::ToolCall { call_id, .. } if call_id.as_ref() == "valid-prefix")));
    }

    #[tokio::test]
    async fn malformed_arguments_keep_the_existing_bounded_correction_budget() {
        let model = model(vec![malformed("bad-1", ChatFinishReason::ToolCalls),
            malformed("bad-2", ChatFinishReason::ToolCalls), malformed("bad-3", ChatFinishReason::ToolCalls),
            text_step("Must not be requested")]);
        let sink = Arc::new(Sink::default());
        let tools = Arc::new(Tools::default());
        let result = run_turn(binding(), model.clone(), tools.clone(), sink.clone(),
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
            AgentContextBudget::default(), CancellationToken::new()).await;
        assert!(matches!(result, Err(AgentEngineError::TurnFailed(message)) if message.contains("protocol-correction budget")));
        assert_eq!(model.requests.lock().unwrap().len(), 3);
        assert!(tools.0.lock().unwrap().is_empty());
        let continuations = sink.events.lock().unwrap().iter().filter_map(|event|
            if let AgentEngineEvent::ModelResponseRejected { continuation, .. } = event { Some(*continuation) } else { None }).collect::<Vec<_>>();
        assert_eq!(continuations, [true, true, false]);
    }

    #[tokio::test]
    async fn pending_tail_rejection_resumes_with_exactly_one_feedback_and_original_authority() {
        let checkpoint = checkpoint();
        let mut coordinator = crate::protocol_recovery::ProtocolRecovery::default();
        let started = AgentEngineEvent::ModelStepStarted {
            step: 1, operation_id: format!("{}:model:1", checkpoint.turn_operation_id.as_ref()).into(),
            correction_source: None,
        };
        coordinator.observe(&started).unwrap();
        let events = malformed("abandoned", ChatFinishReason::ToolCalls);
        let ChatModelEvent::ToolCallRejected { diagnostic, .. } = events[1].as_ref().unwrap() else { unreachable!() };
        let prepared = coordinator.prepare_rejection(1, vec!["abandoned".into()],
            AgentModelResponseRejection::ToolArguments { diagnostics: vec![AgentRejectedToolProposal {
                call_id: "abandoned".into(), name: "read_file".into(), diagnostic: diagnostic.clone(),
            }] }, Some("read_file".into()), true).unwrap();
        let rejection = prepared.event().clone();
        let feedback = crate::protocol_recovery::rejection_feedback(&rejection).unwrap();
        let recovery = crate::AgentTurnRecovery::new(checkpoint.clone(), 1, 2, checkpoint_prefix(&checkpoint),
            vec![started, AgentEngineEvent::ToolCallDelta {
                step: 1, call_id: "abandoned".into(), name: "read_file".into(), arguments_delta: String::new(),
            }, rejection], vec![]).unwrap();
        let model = model(vec![control_step("resumed", "read_file", json!({"path":"b"})), text_step("Done")]);
        let sink = Arc::new(Sink::default());
        let tools = Arc::new(Tools::default());
        let original_root = request().causality.causation_event_id;
        let result = run_turn(binding(), model.clone(), tools.clone(), sink.clone(),
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0).with_recovery(recovery),
            AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(*tools.0.lock().unwrap(), ["resumed"]);
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests[0].causality.causation_event_id, original_root);
        assert_eq!(requests[0].causality.operation_id.as_ref(), format!("{}:model:2", checkpoint.turn_operation_id.as_ref()));
        assert_eq!(requests[0].input.messages.iter().filter(|message| *message == &feedback).count(), 1);
        assert!(sink.events.lock().unwrap().iter().any(|event| matches!(event,
            AgentEngineEvent::ModelStepStarted { step: 2, correction_source: Some(1), .. })));
    }

    #[tokio::test]
    async fn delivery_review_records_native_parameter_rejection_without_reopening_tools() {
        let mut checkpoint = checkpoint();
        checkpoint.completion_review = crate::AgentCompletionReviewState {
            delivery_pending: true, delivery_used: true, input_revision: 1,
            phase: AgentExecutionPhase::DeliveryReview, ..Default::default()
        };
        let recovery = crate::AgentTurnRecovery::new(checkpoint.clone(), 1, 2,
            checkpoint_prefix(&checkpoint), vec![], vec![]).unwrap();
        let mut invalid_report = malformed("invalid-report", ChatFinishReason::ToolCalls);
        for event in invalid_report.iter_mut().filter_map(|item| item.as_mut().ok()) {
            if let ChatModelEvent::ToolCallDelta { name, .. } | ChatModelEvent::ToolCallRejected { name, .. } = event {
                *name = "report_completion".into();
            }
        }
        let model = model(vec![invalid_report, text_step("Must not run")]);
        let sink = Arc::new(Sink::default());
        let tools = Arc::new(Tools::default());
        let result = run_turn(binding(), model.clone(), tools.clone(), sink.clone(),
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0).with_recovery(recovery),
            AgentContextBudget::default(), CancellationToken::new()).await;
        assert!(matches!(result, Err(AgentEngineError::TurnFailed(message)) if message.contains("invalid native tool arguments")));
        assert_eq!(model.requests.lock().unwrap().len(), 1);
        assert!(tools.0.lock().unwrap().is_empty());
        assert!(sink.events.lock().unwrap().iter().any(|event| matches!(event,
            AgentEngineEvent::ModelResponseRejected {
                continuation: false, rejection: Some(AgentModelResponseRejection::ToolArguments { .. }), ..
            })));
    }

    #[tokio::test]
    async fn failed_rejection_commit_and_cancellation_never_open_a_successor() {
        for reject_write in [true, false] {
            let token = CancellationToken::new();
            let sink = Arc::new(Sink { reject_write,
                cancel_on_rejection: (!reject_write).then(|| token.clone()), ..Default::default() });
            let model = model(vec![malformed("bad", ChatFinishReason::ToolCalls), text_step("Must not run")]);
            let tools = Arc::new(Tools::default());
            let result = run_turn(binding(), model.clone(), tools.clone(), sink,
                AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
                AgentContextBudget::default(), token).await;
            if reject_write { assert!(matches!(result, Err(AgentEngineError::EventSink(_)))); }
            else { assert!(matches!(result.unwrap().terminal, AgentTurnTerminal::Cancelled)); }
            assert_eq!(model.requests.lock().unwrap().len(), 1);
            assert!(tools.0.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn failed_validation_commit_cannot_admit_the_corrected_tool() {
        let model = model(vec![malformed("bad", ChatFinishReason::ToolCalls),
            control_step("corrected", "read_file", json!({"path":"b"})), text_step("Must not run")]);
        let sink = Arc::new(Sink { reject_validation_write: true, ..Default::default() });
        let tools = Arc::new(Tools::default());
        let result = run_turn(binding(), model.clone(), tools.clone(), sink.clone(),
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
            AgentContextBudget::default(), CancellationToken::new()).await;
        assert!(matches!(result, Err(AgentEngineError::EventSink(_))));
        assert_eq!(model.requests.lock().unwrap().len(), 2);
        assert!(tools.0.lock().unwrap().is_empty());
        assert!(!sink.events.lock().unwrap().iter().any(|event| matches!(event,
            AgentEngineEvent::ToolStarted { step: 2, .. } | AgentEngineEvent::ToolCompleted { step: 2, .. })));
    }

    #[tokio::test]
    async fn truncated_cancelled_and_interrupted_responses_do_not_become_syntax_corrections() {
        for finish in [ChatFinishReason::MaxOutputTokens, ChatFinishReason::Cancelled] {
            let model = model(vec![malformed("not-complete", finish), text_step("Done")]);
            let sink = Arc::new(Sink::default());
            let tools = Arc::new(Tools::default());
            let result = run_turn(binding(), model.clone(), tools.clone(), sink.clone(),
                AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
                AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
            assert!(tools.0.lock().unwrap().is_empty());
            assert!(!sink.events.lock().unwrap().iter().any(|event|
                matches!(event, AgentEngineEvent::ModelResponseRejected { .. })));
            assert!(matches!(result.terminal, AgentTurnTerminal::Cancelled) || finish == ChatFinishReason::MaxOutputTokens);
        }
        let mut interrupted = malformed("interrupted", ChatFinishReason::ToolCalls);
        interrupted.pop();
        let model = model(vec![interrupted, text_step("Must not run")]);
        let sink = Arc::new(Sink::default());
        let result = run_turn(binding(), model.clone(), Arc::new(Tools::default()), sink.clone(),
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
            AgentContextBudget::default(), CancellationToken::new()).await;
        assert!(matches!(result, Err(AgentEngineError::ModelStreamEndedWithoutTerminal)));
        assert_eq!(model.requests.lock().unwrap().len(), 1);
        assert!(!sink.events.lock().unwrap().iter().any(|event| matches!(event,
            AgentEngineEvent::ModelResponseRejected { .. })));
    }
}
