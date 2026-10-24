//! Startup recovery for already accepted canonical Turns. The candidate set
//! is captured before routes open, so ordinary in-flight admissions in this
//! process cannot be mistaken for crashed work. Jobs never create new input.
use super::*;
use super::super::engine_session_host::EngineSessionHost;

impl NomiCoreSessionOwner {
    pub(super) async fn schedule_native_recovery(self: &Arc<Self>, engines: Arc<EngineSessionHost>) -> Result<usize, AppError> {
        let settlement_jobs = self.schedule_native_settlement_recovery().await?;
        self.schedule_requested_native_resumes().await?;
        let candidates: Vec<(String, String)> = sqlx::query_as(
            "SELECT h.session_id,h.active_turn_id FROM agent_session_heads h JOIN agent_sessions s ON s.agent_session_id=h.session_id \
             JOIN agent_turns t ON t.session_id=h.session_id AND t.operation_id=h.active_turn_id \
             WHERE s.state='live' AND h.status IN ('running','reconciliation') AND t.state='running'")
            .fetch_all(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        let mut scheduled = settlement_jobs;
        for (session, operation) in candidates {
            // UI preparation may keep an idle runtime slot. Only a driver or
            // relay holding actual turn admission supersedes cold recovery.
            if self.runtime_sessions.active_turn_generation(&session).is_some() { continue; }
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
                        || owner.runtime_sessions.active_turn_generation(session.as_ref()).is_some() { return; }
                    let until = match store.native_execution_deadline(&session, &operation).await {
                        Ok(until) => until,
                        Err(_) => { drop(owner); tokio::time::sleep(Duration::from_secs(5)).await; continue; }
                    };
                    if until <= now_ms() {
                        match owner.recover_native_turn(engines, &session, &operation, None).await {
                            Ok(()) => return,
                            Err(AppError::ExecutionAdmissionPending) => { failures = 0; }
                            Err(error) => {
                                failures = failures.saturating_add(1);
                                tracing::warn!(session_id = session.as_ref(), error = %error, "native recovery not admitted");
                                if failures >= 3 {
                                    if let Ok(facts) = store.chat_causality_facts(&session, &operation).await {
                                        match store.quarantine_native_recovery(&facts.session.owner_ref, &session, &operation, facts.execution_fence).await {
                                            Ok(quarantined) => {
                                                if quarantined {
                                                    let root = facts.events.iter().find(|event|
                                                        event.kind.0 == "turn/started" && event.correlation_id.as_ref() == operation.as_ref())
                                                        .and_then(|event|facts.event_payloads.get(event.event_id.as_ref()))
                                                        .and_then(|payload|payload.get("source_message_id"))
                                                        .and_then(Value::as_str);
                                                    if let Some(root) = root {
                                                        owner.user_events.send_to_user(&facts.session.owner_ref.principal_id,
                                                            Self::canonical_turn_paused_wire_event(&session,root));
                                                    } else {
                                                        tracing::warn!(session_id=session.as_ref(),
                                                            "native recovery quarantine has no source message for realtime pause notification");
                                                    }
                                                }
                                                return;
                                            }
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

    /// Reattach cleanup/receipt responsibility after restart. A saved failed
    /// run exit is never converted into a new model request or native resume.
    pub(in crate::router) async fn schedule_native_settlement_recovery(self: &Arc<Self>) -> Result<usize,AppError> {
        let mut after = 0;
        let mut scheduled = 0;
        loop {
            let pending = self.canonical.store().pending_native_turn_settlements(after,500).await.map_err(agent_session_store_error)?;
            let Some(last) = pending.last() else { break; };
            after = last.rowid;
            for item in pending {
                let session = item.receipt.session_id().clone();
                let operation = item.receipt.operation_id().clone();
                // The live callback installs its exact-Runtime worker. A cold
                // scanner must not win that flight and then leave for lack of
                // cleanup proof while the SDK believes custody was accepted.
                if self.runtime_sessions.active_turn_generation(session.as_ref()) == Some(item.receipt.generation()) { continue; }
                // Live custody can start before journal claim (generation 0).
                // The source gate bridges that claim only. Final custody is
                // scoped to its native generation, so publication retries for
                // an old pause cannot block a later explicit resume.
                let original=self.canonical.store().native_recovery_facts(&session,&operation).await.map_err(agent_session_store_error)?;
                let root=original.events.iter().find(|event|event.kind.0=="turn/started"&&event.correlation_id.as_ref()==operation.as_ref())
                    .and_then(|event|original.event_payloads.get(event.event_id.as_ref()))
                    .and_then(|payload|payload.get("source_message_id")).and_then(Value::as_str)
                    .ok_or_else(||AppError::Conflict("cold settlement has no original accepted source".into()))?.to_owned();
                let source_key = Self::turn_settlement_flight_key(&session,&root);
                let Ok(source_gate) = self.session_operation_lock(&source_key).try_write_owned() else { continue; };
                let key = format!("native-turn-settlement:{}:{}:{}",session.as_ref(),operation.as_ref(),item.receipt.generation());
                let Ok(flight) = self.session_operation_lock(&key).try_write_owned() else { continue; };
                drop(source_gate);
                let weak = Arc::downgrade(self);
                let tasks = self.background_tasks.clone();
                let task = async move {
                    let _flight = flight;
                    loop {
                        let Some(owner) = weak.upgrade() else { return; };
                        let result = async {
                            let store = owner.canonical.store();
                            if !store.native_turn_settlement_current(&item.receipt).await.map_err(agent_session_store_error)? { return Ok(()); }
                            if !store.native_turn_settlement_cleanup_proven(&item.receipt).await.map_err(agent_session_store_error)? {
                                // A dead process has no live handles to join. Neither an
                                // empty registry nor a returned Effect proves native cleanup.
                                tracing::warn!(session_id=session.as_ref(),operation_id=operation.as_ref(),
                                    "original native cleanup is unproven after restart; durable settlement remains visible for owner reconciliation");
                                return Ok(());
                            }
                            let facts = store.native_recovery_facts(&session,&operation).await.map_err(agent_session_store_error)?;
                            let start = facts.events.iter().find(|event|event.kind.0=="turn/started"&&event.correlation_id.as_ref()==operation.as_ref())
                                .ok_or_else(||AppError::Conflict("cold settlement has no original Turn start".into()))?;
                            let started = facts.event_payloads.get(start.event_id.as_ref())
                                .ok_or_else(||AppError::Conflict("cold settlement has no original start payload".into()))?;
                            let root = started.get("source_message_id").and_then(Value::as_str)
                                .ok_or_else(||AppError::Conflict("cold settlement has no original accepted root".into()))?;
                            let mut context = TurnErrorContext::default();
                            if started.get("resolved_snapshot_ref") == Some(&serde_json::to_value(&facts.session.agent_binding.resolved_snapshot_ref)
                                .map_err(|error|AppError::Internal(error.to_string()))?) {
                                if let Some(projection) = owner.canonical_conversation_projection_readonly(&item.owner.principal_id,&session).await? {
                                    context = TurnErrorContext::from_response(&projection,projection.extra.get("workspace").and_then(Value::as_str));
                                }
                            }
                            if super::super::engine_journal::EngineTurnJournal::settle_cold_native_turn(&store,&item.receipt,&context).await? {
                                owner.publish_settled_turn(&item.owner.principal_id,&session,&operation,root,None).await?;
                            }
                            Ok::<(),AppError>(())
                        }.await;
                        match result {
                            Ok(()) => return,
                            Err(error) => tracing::warn!(session_id=session.as_ref(),operation_id=operation.as_ref(),
                                error=%nomi_redact::redact_secrets_owned(error.to_string()),"cold settlement could not confirm its original receipt"),
                        }
                        drop(owner);
                        tokio::select! { _=tasks.shutdown_signal()=>return, _=tokio::time::sleep(Duration::from_secs(5))=>{} }
                    }
                };
                if self.background_tasks.spawn(Box::pin(task)) { scheduled += 1; }
            }
        }
        Ok(scheduled)
    }

    pub(super) async fn recover_native_turn(self:&Arc<Self>, engines: Arc<EngineSessionHost>, session: &AgentSessionId, operation: &OperationId, expected_generation: Option<u64>) -> Result<(), AppError> {
        // Serialize local attachment flights, while the canonical lease still
        // arbitrates ownership across hosts. This lock is separate from the
        // Session lifecycle read lock used by driver preparation.
        let _recovery_guard = self.session_operation_lock(&format!("native-recovery:{}",session.as_ref())).write_owned().await;
        let _operation_guard = self.session_operation_lock(session.as_ref()).read_owned().await;
        let facts = self.canonical.store().native_recovery_facts(session, operation).await.map_err(agent_session_store_error)?;
        if facts.session.metadata.purpose == nomifun_agent_contracts::SessionPurpose::PluginAuthoring {
            return Err(AppError::Conflict("PLUGIN_AUTHORING_RETIRED: historical authoring execution cannot recover".into()));
        }
        if expected_generation.is_some_and(|generation| generation != facts.execution_generation) { return Ok(()); }
        if facts.head.status != "running" || facts.head.active_turn_id.as_deref() != Some(operation.as_ref()) {
            return Ok(());
        }
        // A different original caller can transfer to the SDK while this
        // observer waits for the recovery gate. Recheck custody under that
        // gate; an Accepted SDK, including a slow pre-attachment Driver, must
        // never receive a second send or lose its writer to unattached cleanup.
        if self.collaboration_has_native_progress_owner_without_input(session.as_ref(),operation.as_ref()).await?
            ||engines.owns_attached_recovery(session,operation,facts.execution_generation).await {
            return Ok(());
        }
        let started = facts.events.iter().find(|event| event.kind.0 == "turn/started" && event.correlation_id.as_ref() == operation.as_ref())
            .ok_or_else(|| AppError::Conflict("recovery Turn start missing".into()))?;
        let root = facts.event_payloads.get(started.event_id.as_ref()).and_then(|value| value.get("source_message_id")).and_then(Value::as_str)
            .ok_or_else(|| AppError::Conflict("recovery source identity missing".into()))?;
        let processing_started_at = canonical_active_turn_runtime(&facts.head, &facts.events)?
            .filter(|(source, _)| source == root)
            .map(|(_, started_at)| started_at)
            .ok_or_else(|| AppError::Conflict("recovery Turn has no authoritative start time".into()))?;
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
        let options = self.runtime_options_for_projection(&owner_id, session, projection).await?;
        let binding = self.official_runtime.get().ok_or_else(|| AppError::Conflict("recovery Runtime not installed".into()))?.binding()?;
        let delivery = SendMessageData { content, msg_id: root.to_owned(), source_message_id: Some(root.to_owned()), files, inject_skills, origin };
        let admitted = engines.read_turn_receipt(&options, &binding, &facts.session.agent_binding.resolved_snapshot_ref, &delivery).await?;
        // Claim before opening/subscribing a new Runtime, so a losing recovery
        // process cannot publish a spurious error over the winner's stream.
        // Retain the claimed writer before any post-claim database operation.
        // A loader failure transfers only this writer's cleanup responsibility;
        // it never grants another send of the original business input.
        let journal = engines.claim_journal(&admitted, tokio_util::sync::CancellationToken::new()).await?;
        drop(_operation_guard);
        let generation = journal.generation();
        let prepared=async {
            #[cfg(test)]
            self.authorized_resume_attachment_cutpoint(session,"claimed").await?;
            journal.refresh_budget().await?;
            journal.restore_public_projection().await
        }.await;
        if let Err(error)=prepared {
            return self.settle_unattached_native_recovery(&journal,&owner_id,session,operation,root,&error).await;
        }
        let cancellation = tokio_util::sync::CancellationToken::new();
        let runtime = match self.runtime_sessions.get_or_create_runtime_for_turn(session.as_ref(), generation, cancellation.clone(), options).await {
            Ok(runtime) => runtime,
            Err(error) => return self.settle_unattached_native_recovery(&journal,&owner_id,session,operation,root,&error).await,
        };
        let stream = runtime.subscribe();
        self.user_events.send_to_user(
            &owner_id,
            Self::canonical_turn_started_wire_event(session, root, processing_started_at),
        );
        self.spawn_canonical_stream_relay(owner_id.clone(), session.clone(), root.to_owned(), journal.response_message_id().await?,
            generation, operation.clone(), cancellation, stream);
        if let Err(error) = runtime.send_message(delivery).await {
            return self.settle_unattached_native_recovery(&journal,&owner_id,session,operation,root,&AppError::Conflict(error.to_string())).await;
        }
        // Accepted means this exact SDK owns execution, even while its Driver
        // is still preparing. Retain the writer until attachment or that SDK's
        // real canonical exit; no deadline can invent a competing outcome.
        loop {
            let settled=tokio::select! {
                biased;
                _ = journal.wait_attached() => return Ok(()),
                _ = self.background_tasks.shutdown_signal()=>return Err(AppError::Conflict("Session owner closed while the original SDK retained its accepted recovery".into())),
                result=runtime.settle_pending_turn(root)=>result,
            };
            let result=async {
                settled?;
                let state=self.canonical.store().native_execution_notification_state(session,operation,generation).await.map_err(agent_session_store_error)?;
                let Some(state)=state else {return Ok(());};
                if state=="running"||state=="paused"&&self.canonical.store().native_pause_state(session,operation).await
                    .map_err(agent_session_store_error)?.is_none_or(|pause|!pause.cleanup_proven) {
                    return Err(AppError::Conflict("accepted SDK has not proven its original canonical exit and cleanup".into()));
                }
                self.publish_settled_turn(&owner_id,session,operation,root,Some(&runtime)).await
            }.await;
            match result {
                Ok(())=>return Ok(()),
                Err(error)=>tracing::warn!(session_id=session.as_ref(),generation,error=%nomi_redact::redact_secrets_owned(error.to_string()),
                    "accepted recovery remains with its exact SDK; only its original settlement is observed"),
            }
            tokio::select! {_=self.background_tasks.shutdown_signal()=>return Err(AppError::Conflict("Session owner closed while observing original SDK settlement".into())),
                _=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }

    async fn settle_unattached_native_recovery(self:&Arc<Self>,journal:&super::super::engine_journal::EngineTurnJournal,
        owner_id:&str,session:&AgentSessionId,operation:&OperationId,root:&str,first_failure:&AppError)->Result<(),AppError> {
        use super::super::engine_journal::UnattachedRecoverySettlement;
        let original=nomi_redact::redact_secrets_owned(first_failure.to_string());
        let mut settled=false;
        loop {
            let result=async {
                if !settled {
                    if self.canonical.store().native_execution_generation(session,operation).await.map_err(agent_session_store_error)?
                        !=journal.generation() {return Ok(());}
                    match journal.fail_unattached_recovery(&original).await? {
                        UnattachedRecoverySettlement::Attached=>return Ok(()),
                        UnattachedRecoverySettlement::Settled=>settled=true,
                    }
                }
                if self.canonical.store().native_execution_notification_state(session,operation,journal.generation()).await
                    .map_err(agent_session_store_error)?.is_none() {return Ok(());}
                self.publish_settled_turn(owner_id,session,operation,root,None).await
            }.await;
            match result {
                Ok(())=>return Ok(()),
                Err(error)=>tracing::warn!(session_id=session.as_ref(),generation=journal.generation(),
                    error=%nomi_redact::redact_secrets_owned(error.to_string()),"original unattached recovery remains owned; only cleanup and its exit are retried"),
            }
            tokio::select! {_=self.background_tasks.shutdown_signal()=>return Err(AppError::Conflict("Session owner is closing with original unattached cleanup retained".into())),
                _=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
}
