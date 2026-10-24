//! Retirement after an exact committed terminal, independent of UI history.
//! This owner never sends input, creates a Runtime or resumes a checkpoint.
use super::*;

enum RuntimeRetirement { Retired, Inapplicable }

impl NomiCoreSessionOwner {
    pub(in crate::router) fn schedule_completed_runtime_retirement(
        self: &Arc<Self>, owner_id: String, session: AgentSessionId,
        operation: OperationId, expected_runtime: nomifun_ai_agent::AgentRuntimeHandle,
    ) {
        let owner = Arc::downgrade(self);
        self.background_tasks.spawn(Box::pin(async move {
            let Some(owner) = owner.upgrade() else { return; };
            let mut retry = Duration::from_secs(1);
            let mut workspace_ticket=None;
            loop {
                match owner.retire_completed_runtime_with_workspace_ticket(&owner_id, &session, &operation, &expected_runtime,&mut workspace_ticket).await {
                    Ok(RuntimeRetirement::Retired) => {
                        tracing::info!(session_id=session.as_ref(), operation_id=operation.as_ref(),
                            native_teardown_proven=expected_runtime.teardown_proven(),
                            "completed Runtime retirement owner settled");
                        return;
                    }
                    Ok(RuntimeRetirement::Inapplicable) => return,
                    Err(error) => tracing::warn!(session_id=session.as_ref(),operation_id=operation.as_ref(),%error,
                        "completed Runtime retirement remains unconfirmed; exact owner retained"),
                }
                tokio::select! {
                    _ = owner.background_tasks.shutdown_signal() => return,
                    _ = tokio::time::sleep(retry) => {},
                }
                retry = (retry * 2).min(Duration::from_secs(60));
            }
        }));
    }

    pub(in crate::router) async fn retire_completed_runtime(
        &self, owner_id: &str, session: &AgentSessionId, operation: &OperationId,
        expected_runtime: &nomifun_ai_agent::AgentRuntimeHandle,
    ) -> Result<(), AppError> {
        let mut ticket=None;
        self.retire_completed_runtime_with_workspace_ticket(owner_id,session,operation,expected_runtime,&mut ticket).await.map(|_| ())
    }

    async fn retire_completed_runtime_with_workspace_ticket(
        &self,owner_id:&str,session:&AgentSessionId,operation:&OperationId,
        expected_runtime:&nomifun_ai_agent::AgentRuntimeHandle,
        workspace_ticket:&mut Option<crate::router::nomi_core_wave2::WorkspaceHostRetirementTicket>,
    )->Result<RuntimeRetirement,AppError> {
        let engines = self.native_engines.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Runtime retirement has no original resource owner".into()))?;
        if self.runtime_sessions.get_runtime(session.as_ref()).as_ref()
            .is_some_and(|runtime| !std::ptr::eq(runtime.as_runtime(), expected_runtime.as_runtime())) {
            return Ok(RuntimeRetirement::Inapplicable);
        }
        // A delayed terminal relay can reach this owner after the same
        // carrier has accepted a successor. Reject that obsolete request
        // before joining Session background work; the authoritative checks
        // below still repeat under the Session operation fence.
        let observed = match self.canonical.store().head(session).await {
            Ok(head) => head,
            Err(nomifun_agent_session::SessionStoreError::NotFound(_)
                | nomifun_agent_session::SessionStoreError::Deleted(_)) => return Ok(RuntimeRetirement::Inapplicable),
            Err(error) => return Err(agent_session_store_error(error)),
        };
        if observed.active_turn_id.is_some() { return Ok(RuntimeRetirement::Inapplicable); }
        let latest: Option<String> = sqlx::query_scalar(
            "SELECT operation_id FROM agent_turns WHERE session_id=? ORDER BY rowid DESC LIMIT 1",
        ).bind(session.as_ref()).fetch_optional(&self.pool).await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if latest.as_deref() != Some(operation.as_ref()) { return Ok(RuntimeRetirement::Inapplicable); }
        // Natural post-answer work can outlive the main result. Waiting here
        // holds no Session/registry gate and does not delay a successor input.
        tokio::select! {
            _ = self.background_tasks.shutdown_signal() => return Ok(RuntimeRetirement::Inapplicable),
            result = engines.join_completed_runtime_background(session) => result?,
        }
        let _operation_guard = self.session_operation_lock(session.as_ref()).write_owned().await;
        let record = match self.canonical.store().get_live_session(session).await {
            Ok(record) => record,
            // Canonical deletion has its own retained resource owner. Do not
            // retain an obsolete retirement waiter/Arc after that handoff.
            Err(nomifun_agent_session::SessionStoreError::NotFound(_)
                | nomifun_agent_session::SessionStoreError::Deleted(_)) => return Ok(RuntimeRetirement::Inapplicable),
            Err(error) => return Err(agent_session_store_error(error)),
        };
        if record.owner_ref.principal_kind != "user" || record.owner_ref.principal_id != owner_id {
            return Err(AppError::Forbidden("Runtime retirement belongs to another owner".into()));
        }
        let head = self.canonical.store().head(session).await.map_err(agent_session_store_error)?;
        // A paused/recovering Turn retains its canonical active identity.
        if head.active_turn_id.is_some() {
            return Ok(RuntimeRetirement::Inapplicable);
        }
        let latest: Option<String> = sqlx::query_scalar(
            "SELECT operation_id FROM agent_turns WHERE session_id=? ORDER BY rowid DESC LIMIT 1",
        ).bind(session.as_ref()).fetch_optional(&self.pool).await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if latest.as_deref() != Some(operation.as_ref()) { return Ok(RuntimeRetirement::Inapplicable); }
        let receipt = self.canonical.store().read_turn_receipt(session, operation).await.map_err(agent_session_store_error)?;
        if !matches!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Completed
            | nomifun_agent_session::TurnReceiptStatus::Failed | nomifun_agent_session::TurnReceiptStatus::Cancelled) {
            return Ok(RuntimeRetirement::Inapplicable);
        }
        if let Some(active) = self.runtime_sessions.active_turn_generation(session.as_ref()) {
            let original = self.canonical.store().native_execution_generation(session, operation)
                .await.map_err(agent_session_store_error)?;
            if active != original { return Ok(RuntimeRetirement::Inapplicable); }
            // Retry the relay's failed release only for its exact completed
            // generation; the registry independently fences a successor.
            self.runtime_sessions.release_runtime_turn(session.as_ref(), original).await?;
            if self.runtime_sessions.active_turn_generation(session.as_ref()) == Some(original) {
                return Err(AppError::Conflict("completed Runtime admission is still retained".into()));
            }
        }
        if self.canonical.store().has_unsettled_effects(session).await.map_err(agent_session_store_error)?
            && !expected_runtime.teardown_proven() {
            // The exact MCP owner can prove physical closure while retaining
            // Unknown external outcome. This never clears that Effect or
            // grants a new Turn, tool dispatch or checkpoint resume.
            // A concurrent retirement may already have completed this exact
            // Runtime's full teardown and evicted its Kernel weak cache. Its
            // retained successful result is the proof; cache absence is not.
            engines.ensure_completed_effect_resources_closed(session).await?;
        }
        let current = self.runtime_sessions.get_runtime(session.as_ref());
        if current.as_ref().is_some_and(|current| !std::ptr::eq(current.as_runtime(), expected_runtime.as_runtime())) {
            return Ok(RuntimeRetirement::Inapplicable);
        }
        // Quarantined instances are intentionally absent from get_runtime.
        // The registry still owns their exact slot and retry proof below.
        if workspace_ticket.is_none() {*workspace_ticket=engines.capture_completed_workspace_retirement(session)?;}
        engines.retire_completed_browser_resources(owner_id, session, operation).await?;
        // The registry quarantines this original instance until native teardown
        // is proven; Session process owners and user Browser workspaces remain
        // with their existing independent owners.
        if !self.runtime_sessions.retire_runtime_if_matches(session.as_ref(), expected_runtime).await?
            && self.runtime_sessions.get_runtime(session.as_ref()).as_ref()
                .is_some_and(|runtime| std::ptr::eq(runtime.as_runtime(), expected_runtime.as_runtime())) {
            return Err(AppError::Conflict("original Runtime retirement was not confirmed".into()));
        }
        if let Some(ticket)=workspace_ticket.take()
            && let Err(error)=engines.retire_captured_workspace_host(&ticket) {
            tracing::debug!(session_id=session.as_ref(),%error,"native Runtime retired; shared workspace cache cleanup remains owned");
            engines.schedule_workspace_host_retirement(ticket,&self.background_tasks)?;
        }
        Ok(RuntimeRetirement::Retired)
    }
}

impl NomiCoreAgentApiState {
    /// Retry the already-owned retirement/deletion only. This cannot reopen a
    /// Turn, acquire a Runtime, acknowledge unknown effects or dispose of an
    /// ordinary live workspace.
    pub(in crate::router) async fn retry_session_resources(&self, owner_id: &str, session_id: &str,
        expected_head_seq: u64) -> Result<(), NomiCoreApiError> {
        let session = parse_agent_session_id(session_id)?;
        let state: Option<String> = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
            .bind(session.as_ref()).fetch_optional(&self.session_owner.pool).await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let principal = PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
        if state.as_deref() == Some("deleting") {
            let _gate = self.session_owner.session_operation_lock(session.as_ref()).write_owned().await;
            let deleting = self.session_owner.canonical.store().get_deleting_session(&session).await.map_err(agent_session_store_error)?;
            if deleting.owner_ref != principal { return Err(AppError::Forbidden("Session cleanup belongs to another owner".into()).into()); }
            // Deleting has already been authorized above. The public head
            // reader is live-only and cannot authorize this retained cleanup.
            let sequence:Option<i64>=sqlx::query_scalar("SELECT last_seq FROM agent_session_heads WHERE session_id=?")
                .bind(session.as_ref()).fetch_optional(&self.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
            if sequence.and_then(|value|u64::try_from(value).ok()) != Some(expected_head_seq) {
                return Err(AppError::Conflict("Session cleanup cursor changed or is unavailable".into()).into());
            }
            nomifun_common::UserId::parse(owner_id).map_err(|error|AppError::Forbidden(error.to_string()))?;
            execute_nomi_core_agent_session_delete_held(self.clone(), AuthenticatedOwner(UserId::from(owner_id.to_owned())), session.clone(),
                format!("activity-delete-cleanup:{}:{expected_head_seq}",session.as_ref())).await?;
            return Ok(());
        }
        let observed = self.session_owner.canonical.get(&principal,&session).await?;
        if observed.head.last_seq != expected_head_seq || observed.head.active_turn_id.is_some() {
            return Err(AppError::Conflict("Session cleanup no longer targets an inactive exact head".into()).into());
        }
        let operation: Option<String> = sqlx::query_scalar("SELECT operation_id FROM agent_turns WHERE session_id=? ORDER BY rowid DESC LIMIT 1")
            .bind(session.as_ref()).fetch_optional(&self.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let operation = OperationId::from(operation.ok_or_else(||AppError::Conflict("Session has no committed task retirement to retry".into()))?);
        if let Some(runtime) = self.session_owner.runtime_sessions.get_runtime(session.as_ref()) {
            self.session_owner.retire_completed_runtime(owner_id,&session,&operation,&runtime).await?;
        } else {
            let engines = self.session_owner.native_engines.get().and_then(std::sync::Weak::upgrade)
                .ok_or_else(||AppError::Conflict("Original resource cleanup owner is unavailable".into()))?;
            engines.join_completed_runtime_background(&session).await?;
            let _gate = self.session_owner.session_operation_lock(session.as_ref()).write_owned().await;
            let current = self.session_owner.canonical.get(&principal,&session).await?;
            if current.head.last_seq != expected_head_seq || current.head.active_turn_id.is_some() {
                return Err(AppError::Conflict("Session changed during resource cleanup wait".into()).into());
            }
            let receipt = self.session_owner.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error)?;
            if !matches!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Completed
                |nomifun_agent_session::TurnReceiptStatus::Failed|nomifun_agent_session::TurnReceiptStatus::Cancelled) {
                return Err(AppError::Conflict("Original task resource settlement is still unconfirmed".into()).into());
            }
            if self.session_owner.canonical.store().has_unsettled_effects(&session).await.map_err(agent_session_store_error)? {
                engines.ensure_completed_effect_resources_closed(&session).await?;
            }
            // A quarantined original is intentionally hidden by get_runtime.
            // Its retained nonce, rather than absence from the healthy cache,
            // identifies the cleanup to retry after natural background drain.
            let carrier=self.session_owner.runtime_sessions.observe_runtime_carrier(session.as_ref());
            if carrier.teardown_pending {
                let instance=carrier.instance_id.ok_or_else(||AppError::Conflict("Original Runtime cleanup identity is unconfirmed".into()))?;
                if self.session_owner.runtime_sessions.request_runtime_stop(session.as_ref(),&instance,
                    carrier.active_generation.or(carrier.constructing_generation))? {
                    self.session_owner.runtime_sessions.finish_runtime_stop(session.as_ref(),&instance).await?;
                }
            }
            engines.retire_completed_browser_resources(owner_id,&session,&operation).await?;
        }
        Ok(())
    }
}
