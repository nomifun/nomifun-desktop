//! Supervision of an original Runtime's unfinished cleanup and canonical exit.
//! No input, model call, tool invocation or Attempt can be started here.
use super::*;

#[cfg(test)]
static RETRY_PANICS: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeSet<String>>> = std::sync::OnceLock::new();

impl NomiCoreSessionOwner {
    #[cfg(test)]
    pub(in crate::router) async fn hold_turn_settlement_source_for_test(&self,session:&AgentSessionId,root:&str)
        ->tokio::sync::OwnedRwLockWriteGuard<()> {
        self.session_operation_lock(&Self::turn_settlement_flight_key(session,root)).write_owned().await
    }

    /// Read only the admission still held by this exact original SDK. This
    /// does not expose the registry or select a runtime by Session identity.
    pub(in crate::router) fn exact_runtime_admission_generation(&self,session:&str,
        runtime:&nomifun_ai_agent::AgentRuntimeHandle)->Option<u64> {
        self.runtime_sessions.active_turn_generation_if_matches(session,runtime)
    }

    #[cfg(test)]
    pub(in crate::router) fn inject_settlement_retry_panic(session: &str) {
        RETRY_PANICS.get_or_init(Default::default).lock().unwrap().insert(session.to_owned());
    }
    pub(in crate::router) async fn original_turn_operation_for_settlement(&self, session: &AgentSessionId,
        root: &str) -> Result<OperationId, AppError> {
        let operations: Vec<String> = sqlx::query_scalar(
            "SELECT operation_id FROM agent_turns WHERE session_id=? AND source_message_id=? LIMIT 2",
        ).bind(session.as_ref()).bind(root).fetch_all(&self.pool).await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let [operation] = operations.as_slice() else {
            return Err(AppError::Conflict("settlement source has no unique original Turn".into()));
        };
        Ok(operation.clone().into())
    }

    pub(in crate::router) fn turn_settlement_flight_key(session:&AgentSessionId,root:&str)->String {
        format!("native-turn-settlement-source:{}:{root}",session.as_ref())
    }

    pub(in crate::router) fn schedule_turn_settlement(self: &Arc<Self>, owner_id: String,
        session: AgentSessionId, operation:OperationId, root: String, generation:u64,
        runtime:nomifun_ai_agent::AgentRuntimeHandle,
    ) -> Result<(), AppError> {
        let task=runtime.as_runtime().current_task_identity()
            .ok_or_else(||AppError::Conflict("the original SDK task identity is unavailable".into()))?;
        let key=format!("native-turn-settlement:{}:{}:{generation}",session.as_ref(),operation.as_ref());
        let Ok(flight) = self.session_operation_lock(&key).try_write_owned() else { return Ok(()); };
        self.spawn_turn_settlement(owner_id,session,root,flight,Some((operation,generation)),false,runtime,task,None)
    }

    pub(in crate::router) fn schedule_unclaimed_turn_settlement(self:&Arc<Self>,owner_id:String,
        session:AgentSessionId,root:String,committed:Option<(OperationId,u64)>,
        runtime:nomifun_ai_agent::AgentRuntimeHandle)->Result<(),AppError> {
        // The local task owns this flight even after its registry admission is
        // released. Native generation remains a separate canonical binding.
        let task=runtime.as_runtime().current_task_identity()
            .ok_or_else(||AppError::Conflict("the original SDK task identity is unavailable".into()))?;
        let admitted_generation=self.exact_runtime_admission_generation(session.as_ref(),&runtime);
        let identity=std::ptr::from_ref(runtime.as_runtime()) as *const () as usize;
        let key=format!("native-turn-settlement-preclaim:{}:{root}:{identity:x}:{task:?}",session.as_ref());
        let Ok(flight)=self.session_operation_lock(&key).try_write_owned() else {return Ok(());};
        self.spawn_turn_settlement(owner_id,session,root,flight,committed,true,runtime,task,admitted_generation)
    }

    fn spawn_turn_settlement(self:&Arc<Self>,owner_id:String,session:AgentSessionId,root:String,
        flight:tokio::sync::OwnedRwLockWriteGuard<()>,mut binding:Option<(OperationId,u64)>,preclaim:bool,
        runtime:nomifun_ai_agent::AgentRuntimeHandle,task:nomifun_ai_agent::AgentRuntimeTurn,
        admitted_generation:Option<u64>)->Result<(),AppError> {
        // The callback transfers the original SDK explicitly. Neither first
        // installation nor a retry may select a replacement by Session id.
        let weak = Arc::downgrade(self);
        let tasks = self.background_tasks.clone();
        if !self.background_tasks.spawn(Box::pin(async move {
            let _flight = flight;
            let mut _generation_flight=None;
            let mut settled=false;
            loop {
                let Some(owner) = weak.upgrade() else { return; };
                let result = std::panic::AssertUnwindSafe(async {
                    #[cfg(test)]
                    {
                        let fail = RETRY_PANICS.get_or_init(Default::default).lock().unwrap().remove(session.as_ref());
                        assert!(!fail, "fixture original settlement retry panic");
                    }
                    let current_task=runtime.as_runtime().current_task_identity()
                        .ok_or_else(||AppError::Conflict("the original SDK task identity is unavailable".into()))?;
                    if current_task!=task {return Ok(());}
                    if preclaim&&_generation_flight.is_none() {
                        // A cold reconciler briefly takes the write side while
                        // acquiring its generation flight. Neither side keeps
                        // this source gate while publishing a settled pause.
                        let source=owner.session_operation_lock(&Self::turn_settlement_flight_key(&session,&root)).read_owned().await;
                        if binding.is_none() {
                            let rows:Vec<(String,i64)>=sqlx::query_as("SELECT t.operation_id,e.seq FROM agent_turns t JOIN agent_events e ON e.session_id=t.session_id AND e.event_id=t.started_event_id WHERE t.session_id=? AND t.source_message_id=? LIMIT 2")
                                .bind(session.as_ref()).bind(&root).fetch_all(&owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
                            let [(operation,generation)]=rows.as_slice() else {return Err(AppError::Conflict("settlement source has no unique original Turn generation".into()));};
                            let generation=admitted_generation.unwrap_or(u64::try_from(*generation)
                                .map_err(|_|AppError::Conflict("settlement generation is invalid".into()))?);
                            binding=Some((operation.clone().into(),generation));
                        }
                        let (operation,generation)=binding.as_ref().expect("resolved above");
                        let key=format!("native-turn-settlement:{}:{}:{generation}",session.as_ref(),operation.as_ref());
                        let Ok(flight)=owner.session_operation_lock(&key).try_write_owned() else {return Ok(());};
                        _generation_flight=Some(flight);
                        drop(source);
                    }
                    let (operation,generation)=binding.as_ref().expect("resolved above");
                    if !settled {
                        // Acquiring the source gate may have waited. Never let
                        // an old callback settle a later task on the same SDK.
                        let current_task=runtime.as_runtime().current_task_identity()
                            .ok_or_else(||AppError::Conflict("the original SDK task identity is unavailable".into()))?;
                        if current_task!=task {return Ok(());}
                        runtime.settle_pending_turn(&root).await?;
                        settled=true;
                    }
                    // A user can resume this exact Turn after its genuine
                    // Pause committed. Re-publication is for the original
                    // generation only and must never settle the new SDK turn.
                    if owner.canonical.store().native_execution_notification_state(&session,operation,*generation).await
                        .map_err(agent_session_store_error)?.is_none() {return Ok(());}
                    owner.publish_settled_turn(&owner_id, &session, &operation, &root, Some(&runtime)).await
                }).catch_unwind().await.unwrap_or_else(|_| Err(AppError::Internal("original Turn settlement retry panicked; original owner retained".into())));
                match result {
                    Ok(()) => return,
                    Err(error) => tracing::warn!(session_id=session.as_ref(),root_message_id=root.as_str(),
                        error=%nomi_redact::redact_secrets_owned(error.to_string()),
                        "original Turn settlement remains owned; no business operation is retried"),
                }
                drop(owner);
                tokio::select! {
                    _ = tasks.shutdown_signal() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        })) {
            return Err(AppError::Conflict("Session owner is closing; original Runtime retains its settlement".into()));
        }
        Ok(())
    }

    pub(in crate::router) async fn publish_settled_turn(self: &Arc<Self>, owner_id: &str, session: &AgentSessionId,
        operation: &OperationId, root: &str, runtime: Option<&nomifun_ai_agent::AgentRuntimeHandle>,
    ) -> Result<(), AppError> {
        use nomifun_agent_session::TurnReceiptStatus;
        use nomifun_ai_agent::protocol::events::{FinishEventData, TurnStopReason};
        let store = self.canonical.store();
        let receipt = store.read_turn_receipt(session, operation).await.map_err(agent_session_store_error)?;
        if receipt.status == TurnReceiptStatus::Running {
            let pause = store.native_pause_state(session, operation).await.map_err(agent_session_store_error)?;
            if pause.is_some_and(|pause| pause.cleanup_proven) {
                self.user_events.send_to_user(owner_id, Self::canonical_turn_paused_wire_event(session, root));
                return Ok(());
            }
            return Err(AppError::Conflict("original Turn has neither a committed exit nor cleanup proof".into()));
        }
        let terminal = receipt.terminal_event.ok_or_else(|| AppError::Conflict("original terminal receipt missing".into()))?;
        let mut event = if receipt.status == TurnReceiptStatus::Failed {
            let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) = terminal.payload else {
                return Err(AppError::Conflict("failed terminal has no canonical error payload".into()));
            };
            AgentStreamEvent::Error(serde_json::from_value(payload.0.get("error").cloned()
                .ok_or_else(|| AppError::Conflict("failed terminal error is missing".into()))?)
                .map_err(|error| AppError::Internal(error.to_string()))?)
        } else {
            let stop_reason = if receipt.status == TurnReceiptStatus::Cancelled { TurnStopReason::Cancelled }
            else {
                let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) = &terminal.payload else {
                    return Err(AppError::Conflict("completed terminal has no canonical outcome payload".into()));
                };
                if payload.0.get("user_input_request").is_some_and(|value| !value.is_null()) { TurnStopReason::InputRequired }
                else {
                    let reason = serde_json::from_value(payload.0.get("finish_reason").cloned()
                        .ok_or_else(|| AppError::Conflict("completed terminal finish reason missing".into()))?)
                        .map_err(|error| AppError::Internal(error.to_string()))?;
                    nomifun_ai_agent::engine_sdk::completed_turn_stop_reason(reason)
                }
            };
            AgentStreamEvent::Finish(FinishEventData { session_id: Some(session.as_ref().to_owned()),
                stop_reason: Some(stop_reason),
                turn_outcome: None })
        };
        let created_at = store.session_created_at(session).await.map_err(agent_session_store_error)?;
        let summary = store.turn_history_for_sources(session, &[root.to_owned()]).await.map_err(agent_session_store_error)?
            .pop().ok_or_else(|| AppError::Conflict("settled Turn result projection missing".into()))?;
        if let Some(summary) = canonical_message_response(session, created_at, summary)
            .map_err(|error| AppError::Conflict(format!("settled Turn projection: {error:?}")))? {
            if let AgentStreamEvent::Finish(data) = &mut event {
                data.turn_outcome = summary.content.get("turn_outcome").filter(|value| !value.is_null())
                    .cloned().map(serde_json::from_value).transpose().map_err(|error| AppError::Internal(error.to_string()))?;
            }
            if let Some(wire) = Self::canonical_terminal_summary_wire_event(session, summary) {
                self.user_events.send_to_user(owner_id, wire);
            }
        }
        self.user_events.send_to_user(owner_id, Self::canonical_turn_completed_wire_event(session, root, &event));
        if let Some(engine) = self.execution_progress.get().and_then(std::sync::Weak::upgrade) {
            engine.notify_source_turn_settled(owner_id, session.as_ref(), operation.as_ref()).await?;
        }
        if let Some(runtime) = runtime {
            self.schedule_completed_runtime_retirement(owner_id.to_owned(), session.clone(), operation.clone(), runtime.clone());
        }
        self.schedule_session_input_drain(owner_id.to_owned(), session.clone(), None);
        Ok(())
    }
}
