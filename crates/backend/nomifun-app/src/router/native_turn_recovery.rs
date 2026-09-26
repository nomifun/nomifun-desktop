//! Startup recovery for already accepted canonical Turns. The candidate set
//! is captured before routes open, so ordinary in-flight admissions in this
//! process cannot be mistaken for crashed work. Jobs never create new input.
use super::*;
use super::super::engine_session_host::EngineSessionHost;

impl NomiCoreSessionOwner {
    pub(super) async fn schedule_native_recovery(self: &Arc<Self>, engines: Arc<EngineSessionHost>) -> Result<usize, AppError> {
        let candidates: Vec<(String, String)> = sqlx::query_as(
            "SELECT h.session_id,h.active_turn_id FROM agent_session_heads h JOIN agent_sessions s ON s.agent_session_id=h.session_id \
             JOIN agent_turns t ON t.session_id=h.session_id AND t.operation_id=h.active_turn_id \
             WHERE s.state='live' AND h.status IN ('running','reconciliation') AND t.state='running'")
            .fetch_all(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        let mut scheduled = 0;
        for (session, operation) in candidates {
            if self.runtime_sessions.get_runtime(&session).is_some() { continue; }
            let owner = Arc::downgrade(self);
            let engines = Arc::downgrade(&engines);
            let task = async move {
                let session = AgentSessionId::from(session);
                let operation = OperationId::from(operation);
                let mut failures = 0u8;
                loop {
                    let (Some(owner), Some(engines)) = (owner.upgrade(), engines.upgrade()) else { return; };
                    let store = owner.canonical.store();
                    let receipt = match store.read_turn_receipt(&session, &operation).await {
                        Ok(receipt) => receipt,
                        Err(nomifun_agent_session::SessionStoreError::Deleted(_) | nomifun_agent_session::SessionStoreError::NotFound(_)) => return,
                        Err(_) => { drop(owner); tokio::time::sleep(Duration::from_secs(5)).await; continue; }
                    };
                    if receipt.status != nomifun_agent_session::TurnReceiptStatus::Running
                        || owner.runtime_sessions.get_runtime(session.as_ref()).is_some() { return; }
                    let until = match store.native_execution_deadline(&session, &operation).await {
                        Ok(until) => until,
                        Err(_) => { drop(owner); tokio::time::sleep(Duration::from_secs(5)).await; continue; }
                    };
                    if until <= now_ms() {
                        match owner.recover_native_turn(engines, &session, &operation, None).await {
                            Ok(()) => return,
                            Err(error) => {
                                failures = failures.saturating_add(1);
                                tracing::warn!(session_id = session.as_ref(), error = %error, "native recovery not admitted");
                                if failures >= 3 {
                                    if let Ok(facts) = store.chat_causality_facts(&session, &operation).await {
                                        match store.quarantine_native_recovery(&facts.session.owner_ref, &session, &operation, facts.execution_fence).await {
                                            Ok(_) => return,
                                            Err(nomifun_agent_session::SessionStoreError::ExecutionLeaseActive | nomifun_agent_session::SessionStoreError::ExecutionFenced) => failures = 0,
                                            Err(error) => tracing::warn!(session_id=session.as_ref(), error=%error, "native recovery quarantine not committed; retrying safely"),
                                        }
                                    }
                                }
                            }
                        }
                    }
                    drop(owner);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            };
            if self.background_tasks.spawn(Box::pin(task)) { scheduled += 1; }
        }
        Ok(scheduled)
    }

    pub(super) async fn recover_native_turn(&self, engines: Arc<EngineSessionHost>, session: &AgentSessionId, operation: &OperationId, expected_generation: Option<u64>) -> Result<(), AppError> {
        // Serialize local attachment flights, while the canonical lease still
        // arbitrates ownership across hosts. This lock is separate from the
        // Session lifecycle read lock used by driver preparation.
        let _recovery_guard = self.session_operation_lock(&format!("native-recovery:{}",session.as_ref())).write_owned().await;
        let _operation_guard = self.session_operation_lock(session.as_ref()).read_owned().await;
        let facts = self.canonical.store().native_recovery_facts(session, operation).await.map_err(agent_session_store_error)?;
        if expected_generation.is_some_and(|generation| generation != facts.execution_generation) { return Ok(()); }
        if facts.head.status != "running" || facts.head.active_turn_id.as_deref() != Some(operation.as_ref()) {
            return Ok(());
        }
        let started = facts.events.iter().find(|event| event.kind.0 == "turn/started" && event.correlation_id.as_ref() == operation.as_ref())
            .ok_or_else(|| AppError::Conflict("recovery Turn start missing".into()))?;
        let root = facts.event_payloads.get(started.event_id.as_ref()).and_then(|value| value.get("source_message_id")).and_then(Value::as_str)
            .ok_or_else(|| AppError::Conflict("recovery source identity missing".into()))?;
        let input = facts.event_payloads.get(root).ok_or_else(|| AppError::Conflict("recovery source input missing".into()))?;
        let content = input.get("content").and_then(Value::as_str).ok_or_else(|| AppError::Conflict("recovery source text is missing".into()))?.to_owned();
        let files = super::super::runtime_attachments::references(input)?;
        let inject_skills = super::super::runtime_attachments::selected_skills(input)?;
        let origin = match input.get("origin") {
            None | Some(Value::Null) => None, Some(Value::String(value)) => Some(value.clone()),
            _ => return Err(AppError::Conflict("recovery source origin is invalid".into())),
        };
        let owner_id = facts.session.owner_ref.principal_id.clone();
        if facts.session.owner_ref.principal_kind != "user" { return Err(AppError::Conflict("unsupported recovery principal".into())); }
        let projection = self.canonical_conversation_projection(&owner_id, session).await?
            .ok_or_else(|| AppError::NotFound("recovery Session missing".into()))?;
        let (options, _) = runtime_options_from_session(&owner_id, projection, None)?;
        let binding = self.official_runtime.get().ok_or_else(|| AppError::Conflict("recovery Runtime not installed".into()))?.binding()?;
        let delivery = SendMessageData { content, msg_id: root.to_owned(), source_message_id: Some(root.to_owned()), files, inject_skills, origin };
        let admitted = engines.read_turn_receipt(&options, &binding, &facts.session.agent_binding.resolved_snapshot_ref, &delivery).await?;
        // Claim before opening/subscribing a new Runtime, so a losing recovery
        // process cannot publish a spurious error over the winner's stream.
        let journal = engines.open_journal(&admitted, tokio_util::sync::CancellationToken::new()).await?;
        let generation = journal.generation();
        let cancellation = tokio_util::sync::CancellationToken::new();
        let runtime = match self.runtime_sessions.get_or_create_runtime_for_turn(session.as_ref(), generation, cancellation.clone(), options).await {
            Ok(runtime) => runtime,
            Err(error) => { let _ = journal.fail_unattached_recovery().await; return Err(error); }
        };
        let mut monitor = runtime.subscribe();
        let stream = runtime.subscribe();
        self.user_events.send_to_user(&owner_id, Self::canonical_turn_started_wire_event(session, root));
        self.spawn_canonical_stream_relay(owner_id, session.clone(), root.to_owned(), journal.response_message_id().await?,
            generation, operation.clone(), cancellation, stream);
        if let Err(error) = runtime.send_message(delivery).await {
            let _ = journal.fail_unattached_recovery().await;
            return Err(AppError::Conflict(error.to_string()));
        }
        // Keep the preclaimed writer alive until the actual Driver owns it.
        let attachment_deadline = tokio::time::sleep(Duration::from_secs(30));
        tokio::pin!(attachment_deadline);
        loop {
            tokio::select! {
                biased;
                _ = journal.wait_attached() => return Ok(()),
                _ = &mut attachment_deadline => {
                    if journal.is_attached() { return Ok(()); }
                    let _ = journal.fail_unattached_recovery().await;
                    return Err(AppError::Conflict("recovery Runtime did not attach before its deadline".into()));
                }
                event = monitor.recv() => match event {
                    Ok(AgentStreamEvent::Error(_) | AgentStreamEvent::Finish(_)) | Err(broadcast::error::RecvError::Closed) => {
                        let _ = journal.fail_unattached_recovery().await;
                        return Err(AppError::Conflict("recovery Runtime ended before attachment".into()));
                    }
                    _ => {}
                }
            }
        }
    }
}
