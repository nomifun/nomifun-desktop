//! Formal source owners hand only resource retirement to the existing host.
//! No Runtime acquisition, input delivery, task retry or result rewriting.
use super::*;
use super::super::engine_session_host::EndedBrowserTaskSource;

/// One server-owned source per original accepted input. This private enum is
/// deliberately separate from public message fields and Robot input sources.
pub(super) enum ServerTurnSource {
    Cron(nomifun_agent_contracts::CronRunRuntimeSource),
    Remote(nomifun_agent_contracts::RemoteBindingVersionRef),
}

pub(super) fn validated_remote_runtime_source(
    record: &nomifun_agent_contracts::AgentSessionLiveRecord,
    remote: &NomiRemoteSessionRow, owner_id: &str, session: &AgentSessionId,
) -> Result<nomifun_agent_contracts::RemoteBindingVersionRef, AppError> {
    let provenance = record.remote_binding_provenance.as_ref()
        .ok_or_else(|| AppError::Conflict("Remote input has no original frozen binding provenance".into()))?;
    if record.agent_session_id != *session || record.owner_ref.principal_kind != "user"
        || record.owner_ref.principal_id != owner_id || remote.agent_session_id != session.as_ref()
        || remote.owner_user_id != owner_id || provenance.remote_binding_id.as_ref() != remote.remote_binding_id
        || i64::try_from(provenance.binding_version).ok() != Some(remote.binding_version)
        || source_digest(&serde_json::to_value(&record.agent_binding).map_err(|error| AppError::Internal(error.to_string()))?)? != remote.agent_binding_digest {
        return Err(AppError::Conflict("Remote input differs from its original frozen source".into()));
    }
    Ok(nomifun_agent_contracts::RemoteBindingVersionRef {
        remote_binding_id: provenance.remote_binding_id.clone(), binding_version: provenance.binding_version,
    })
}


pub(super) enum CronSourceResourceState {
    Ended(OperationId),
    ContinuingOrNeverInvoked,
    Pending,
}

pub(super) async fn finalized_cron_source_operation(
    pool: &nomifun_db::SqlitePool,
    canonical: &CanonicalAgentSessionOwner,
    request: &nomifun_cron::CronRunResourceRetirement,
) -> Result<CronSourceResourceState, AppError> {
    let Some(status) = request.finalized_status() else {
        return Err(AppError::Conflict("Cron retirement has no immutable original finalization receipt".into()));
    };
    if !matches!(status, "ok" | "error" | "cancelled") {
        return Err(AppError::Conflict("Cron retirement has no formal final task outcome".into()));
    }
    let session = &request.agent_session_id;
    let record = match canonical.store().get_live_session(session).await {
        Ok(record) => record,
        Err(nomifun_agent_session::SessionStoreError::Deleted(_)) => return Ok(CronSourceResourceState::ContinuingOrNeverInvoked),
        Err(error) => return Err(agent_session_store_error(error)),
    };
    if record.owner_ref.principal_kind != "user" || record.owner_ref.principal_id != request.owner_id {
        return Err(AppError::Forbidden("Cron resource retirement belongs to another Session owner".into()));
    }
    // The ticket is minted only by the original NewConversation preparation.
    // Its actual create(run key) result is independently bound to this Session;
    // current mutable cron_jobs.execution_mode is deliberately never consulted.
    let opening_key = format!("user:{}:open:cron:{}:conversation", request.owner_id, request.cron_job_run_id);
    let original_creation: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND producer_id='session_api' \
         AND kind='session/opening' AND kind_version=1 AND idempotency_key=? AND correlation_id=?)",
    ).bind(session.as_ref()).bind(&opening_key).bind(format!("open:{opening_key}"))
        .fetch_one(pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
    if !original_creation {
        return Err(AppError::Conflict("Cron resource retirement has no original per-run Session creation".into()));
    }
    // The original finalizer sealed this tuple before waking its observer.
    // Job/run rows may already have been legally deleted; resource handoff
    // never reinstates their existence as a new scheduling prerequisite.
    let operation = NomiCoreSessionOwner::turn_operation_id(&request.owner_id, session.as_ref(),
        &format!("cron:{}:turn", request.cron_job_run_id));
    let receipt = canonical.store().read_turn_receipt(session, &operation).await.map_err(agent_session_store_error)?;
    if receipt.status == nomifun_agent_session::TurnReceiptStatus::NotFound {
        // Cancellation won before this exact invocation. Another purpose in
        // this historical Session never gains retirement authority from it.
        return Ok(CronSourceResourceState::ContinuingOrNeverInvoked);
    }
    let (_, input) = canonical.store().turn_accepted_input(session, &operation).await.map_err(agent_session_store_error)?;
    ensure_exclusive_source_refs(&input.0)?;
    let Some(value) = input.0.get("cron_runtime_overlay") else {
        return Ok(CronSourceResourceState::ContinuingOrNeverInvoked);
    };
    let source: nomifun_agent_contracts::CronRunRuntimeSource = serde_json::from_value(value.clone())
        .map_err(|error| AppError::Conflict(format!("invalid original Cron input source: {error}")))?;
    if source.cron_job_id != request.cron_job_id || source.cron_job_run_id != request.cron_job_run_id
        || source.agent_session_id != *session || input.0.get("origin").and_then(Value::as_str) != Some("cron") {
        // A valid other input that won the literal idempotency key does not
        // become this run's source through a later error finalization.
        return Ok(CronSourceResourceState::ContinuingOrNeverInvoked);
    }
    if !matches!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Completed
        | nomifun_agent_session::TurnReceiptStatus::Failed | nomifun_agent_session::TurnReceiptStatus::Cancelled) {
        return Ok(CronSourceResourceState::Pending);
    }
    match canonical.store().turn_outcome(session, &operation).await.map_err(agent_session_store_error)? {
        Some(nomifun_agent_contracts::AgentTurnOutcome::Answered { .. }
            | nomifun_agent_contracts::AgentTurnOutcome::Silent { .. }
            | nomifun_agent_contracts::AgentTurnOutcome::Failed { .. }
            | nomifun_agent_contracts::AgentTurnOutcome::Cancelled { .. }) => Ok(CronSourceResourceState::Ended(operation)),
        Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired { .. }) => Ok(CronSourceResourceState::ContinuingOrNeverInvoked),
        _ => Ok(CronSourceResourceState::Pending),
    }
}

impl NomiCoreSessionOwner {
    pub(super) fn retain_finalized_cron_browser_retirement(
        &self, request: nomifun_cron::CronRunResourceRetirement,
    ) -> Result<(), AppError> {
        nomifun_common::UserId::parse(&request.owner_id).map_err(|error| AppError::BadRequest(error.to_string()))?;
        nomifun_common::CronJobId::parse(&request.cron_job_id).map_err(|error| AppError::BadRequest(error.to_string()))?;
        nomifun_common::CronJobRunId::parse(&request.cron_job_run_id).map_err(|error| AppError::BadRequest(error.to_string()))?;
        // An owner that never installed the native host never acquired Agent
        // Browser authority. User-created workspaces remain with their own
        // owner; do not create a carrier or a permanent retry to inspect them.
        let Some(engines) = self.native_engines.get().cloned() else { return Ok(()); };
        let pool = self.pool.clone();
        let canonical = self.canonical.clone();
        let tasks = self.background_tasks.clone();
        let original_tasks = tasks.clone();
        if !original_tasks.spawn(Box::pin(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                let result = async {
                    match finalized_cron_source_operation(&pool, &canonical, &request).await? {
                        CronSourceResourceState::ContinuingOrNeverInvoked => Ok(true),
                        CronSourceResourceState::Pending => Ok(false),
                        CronSourceResourceState::Ended(operation) => {
                            let engines = engines.upgrade()
                                .ok_or_else(|| AppError::Conflict("Cron Browser retirement has no original host owner".into()))?;
                            engines.retire_ended_source_task_resources(&request.owner_id, &request.agent_session_id, &operation,
                                EndedBrowserTaskSource::CronNewConversation).await
                        }
                    }
                }.await;
                match result {
                    Ok(true) => return,
                    Ok(false) => {},
                    Err(error) => tracing::warn!(run_id=%request.cron_job_run_id,session_id=request.agent_session_id.as_ref(),%error,
                        "original Cron Browser retirement remains owned"),
                }
                tokio::select! {
                    _ = tasks.shutdown_signal() => return,
                    _ = tokio::time::sleep(backoff) => {},
                }
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        })) {
            return Err(AppError::Conflict("Cron Browser retirement owner is closing; application Browser shutdown retains its resources".into()));
        }
        Ok(())
    }

    /// This runs in the original Remote single-flight source owner. Durable
    /// cancellation is already committed; only resource handoff is retried.
    pub(super) async fn finish_cancelled_remote_browser_retirement(
        &self, repository: &Arc<dyn IRemoteBindingRepository>, owner_id: &str,
        session: &AgentSessionId, key: &str, target: &RemoteCancelTarget,
    ) {
        let mut backoff = Duration::from_secs(1);
        loop {
            match self.retire_cancelled_remote_browser_once(repository, owner_id, session, key, target).await {
                Ok(true) => return,
                Ok(false) => {},
                Err(error) => tracing::warn!(session_id=session.as_ref(),%error,
                    "original cancelled Remote Browser retirement remains owned"),
            }
            tokio::select! {
                _ = self.background_tasks.shutdown_signal() => return,
                _ = tokio::time::sleep(backoff) => {},
            }
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    }

    pub(super) async fn retire_cancelled_remote_browser_once(
        &self, repository: &Arc<dyn IRemoteBindingRepository>, owner_id: &str,
        session: &AgentSessionId, key: &str, target: &RemoteCancelTarget,
    ) -> Result<bool, AppError> {
        let Some(original_host) = self.native_engines.get() else { return Ok(true); };
        match self.canonical.store().get_live_session(session).await {
            Ok(_) => {},
            // Canonical deletion already owns this exact Session's resources.
            Err(nomifun_agent_session::SessionStoreError::Deleted(_)) => return Ok(true),
            Err(error) => return Err(agent_session_store_error(error)),
        }
        if !cancelled_remote_source_proven(&self.canonical, repository, owner_id, session, key, target).await? {
            return Ok(false);
        }
        if !exact_remote_cancellation_stopped(self, session, target).await? { return Ok(false); }
        let engines = original_host.upgrade()
            .ok_or_else(|| AppError::Conflict("Remote Browser retirement has no original host owner".into()))?;
        let Some(purpose) = engines.source_browser_task_purpose(owner_id, session).await? else { return Ok(true); };
        // A prior Browser purpose may belong to this closed source, but the
        // cancel target alone cannot confer that ownership. Another legitimate
        // input in the Session remains with its own owner and creates no retry.
        match remote_browser_purpose_state(&self.canonical, repository, owner_id, session, &purpose, target).await? {
            RemotePurposeResourceState::OtherSource => return Ok(true),
            RemotePurposeResourceState::Pending => return Ok(false),
            RemotePurposeResourceState::Ended => {},
        }
        engines.retire_ended_source_task_resources(owner_id, session, &purpose,
            EndedBrowserTaskSource::CancelledRemoteSession).await
    }
}

fn ensure_exclusive_source_refs(input: &Value) -> Result<(), AppError> {
    if ["cron_runtime_overlay", "remote_runtime_source", "input_source"].into_iter()
        .filter(|field| input.get(*field).is_some()).count() > 1 {
        return Err(AppError::Conflict("canonical input has conflicting server-owned sources".into()));
    }
    Ok(())
}

fn source_digest(value: &Value) -> Result<String, AppError> {
    nomifun_agent_contracts::digest_payload(value).map(|digest| digest.as_ref().to_owned())
        .map_err(|error| AppError::Internal(error.to_string()))
}

pub(super) async fn cancelled_remote_source_proven(
    canonical: &CanonicalAgentSessionOwner, repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str, session: &AgentSessionId, key: &str, target: &RemoteCancelTarget,
) -> Result<bool, AppError> {
    let record = canonical.store().get_live_session(session).await.map_err(agent_session_store_error)?;
    if record.owner_ref.principal_kind != "user" || record.owner_ref.principal_id != owner_id {
        return Err(AppError::Forbidden("Remote resource retirement belongs to another owner".into()));
    }
    let remote = repository.get_session(owner_id, session.as_ref()).await?
        .ok_or_else(|| AppError::Conflict("Remote resource retirement lost its original Session source".into()))?;
    if !matches!(remote.state.as_str(), "cancelled" | "failed") { return Ok(false); }
    let provenance = record.remote_binding_provenance.as_ref()
        .ok_or_else(|| AppError::Conflict("Remote retirement has no original frozen binding provenance".into()))?;
    if remote.agent_session_id != session.as_ref() || remote.owner_user_id != owner_id
        || remote.binding_version != target.expected_binding_version
        || provenance.remote_binding_id.as_ref() != remote.remote_binding_id
        || i64::try_from(provenance.binding_version).ok() != Some(remote.binding_version)
        || source_digest(&serde_json::to_value(&record.agent_binding).map_err(|error| AppError::Internal(error.to_string()))?)? != remote.agent_binding_digest {
        return Err(AppError::Conflict("Remote Browser retirement differs from its original frozen source".into()));
    }
    let digest = source_digest(&json!({"idempotency_key":key}))?;
    let request_digest = source_digest(&json!({"operation":"remote.cancel","agent_session_id":session,"idempotency_key":key}))?;
    let Some(requested) = repository.find_event_by_operation_key(owner_id, session.as_ref(), "session/cancel-requested", &digest).await? else { return Ok(false); };
    let Some(cancelled) = repository.find_event_by_operation_key(owner_id, session.as_ref(), "session/cancelled", &digest).await? else { return Ok(false); };
    if cancelled.seq <= requested.seq { return Ok(false); }
    let requested: Value = serde_json::from_str(&requested.payload_json).map_err(|error| AppError::Internal(error.to_string()))?;
    let cancelled_value: Value = serde_json::from_str(&cancelled.payload_json).map_err(|error| AppError::Internal(error.to_string()))?;
    let captured: RemoteCancelTarget = serde_json::from_value(json!({
        "expected_operation_id":requested.get("expected_operation_id"),
        "expected_head_seq":requested.get("expected_head_seq"),
        "expected_binding_version":requested.get("expected_binding_version"),
    })).map_err(|error| AppError::Conflict(format!("invalid original Remote cancellation target: {error}")))?;
    Ok(captured == *target && requested.get("request_digest").and_then(Value::as_str) == Some(request_digest.as_str())
        && cancelled_value.get("request_digest").and_then(Value::as_str) == Some(request_digest.as_str())
        && cancelled_value.get("outcome").and_then(Value::as_str) == Some("cancelled")
        && (remote.state != "failed" || cancelled_value.get("preserved_failure").and_then(Value::as_bool) == Some(true)))
}

pub(super) enum RemotePurposeResourceState { OtherSource, Pending, Ended }

pub(super) async fn remote_browser_purpose_state(
    canonical: &CanonicalAgentSessionOwner, repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str, session: &AgentSessionId, purpose: &OperationId, target: &RemoteCancelTarget,
) -> Result<RemotePurposeResourceState, AppError> {
    let (_, input) = canonical.store().turn_accepted_input(session, purpose).await.map_err(agent_session_store_error)?;
    // The source author is the private Remote dispatcher, never origin text,
    // operation spelling, latest Turn, or the mutable Remote event projection.
    ensure_exclusive_source_refs(&input.0)?;
    let Some(value) = input.0.get("remote_runtime_source") else { return Ok(RemotePurposeResourceState::OtherSource); };
    let actual: nomifun_agent_contracts::RemoteBindingVersionRef = serde_json::from_value(value.clone())
        .map_err(|error| AppError::Conflict(format!("invalid original Remote input source: {error}")))?;
    let remote = repository.get_session(owner_id, session.as_ref()).await?
        .ok_or_else(|| AppError::Conflict("Remote resource retirement lost its original Session source".into()))?;
    let record = canonical.store().get_live_session(session).await.map_err(agent_session_store_error)?;
    let expected = validated_remote_runtime_source(&record, &remote, owner_id, session)?;
    if actual != expected || i64::try_from(actual.binding_version).ok() != Some(target.expected_binding_version) {
        return Ok(RemotePurposeResourceState::OtherSource);
    }
    let receipt = canonical.store().read_turn_receipt(session, purpose).await.map_err(agent_session_store_error)?;
    Ok(if matches!(receipt.status, nomifun_agent_session::TurnReceiptStatus::Completed
        | nomifun_agent_session::TurnReceiptStatus::Failed | nomifun_agent_session::TurnReceiptStatus::Cancelled) {
        RemotePurposeResourceState::Ended
    } else { RemotePurposeResourceState::Pending })
}

pub(super) fn retain_remote_source_browser_retirement(
    state: &NomiCoreAgentApiState, owner_id: &str, session: &AgentSessionId, key: &str, target: &RemoteCancelTarget,
) -> Result<(), NomiCoreApiError> {
    retain_remote_source_browser_retirement_with_owner(&state.remote_runtime, state.session_owner.clone(),
        state.remote_repository.clone(), owner_id, session, key, target)
}

pub(super) fn retain_remote_source_browser_retirement_with_owner(
    coordinator: &super::super::remote_runtime::NomiCoreRemoteRuntimeCoordinator,
    owner: Arc<NomiCoreSessionOwner>, repository: Arc<dyn IRemoteBindingRepository>,
    owner_id: &str, session: &AgentSessionId, key: &str, target: &RemoteCancelTarget,
) -> Result<(), NomiCoreApiError> {
    let task_key = format!("remote-source-browser-retirement:{}:{}", session.as_ref(), remote_operation_key_digest(key)?);
    let owner_id = owner_id.to_owned(); let session = session.clone(); let key = key.to_owned(); let target = target.clone();
    match coordinator.retain_resource_cleanup_once(task_key, move || async move {
        owner.finish_cancelled_remote_browser_retirement(&repository, &owner_id, &session, &key, &target).await;
    }) {
        Ok(()) | Err(super::super::remote_runtime::RemoteDetachedMutationAdmissionError::AlreadyRunning) => Ok(()),
        Err(error) => Err(AppError::Conflict(format!("original Remote Browser retirement owner is closing: {error:?}")).into()),
    }
}

/// Called inside the original retained cancellation owner. No await separates
/// the durable result from registering its resource responsibility.
pub(super) async fn commit_remote_cancellation_and_handoff(
    state: NomiCoreAgentApiState, owner: AuthenticatedOwner, session: AgentSessionId,
    key: String, target: RemoteCancelTarget, current: NomiRemoteSessionRow, request_digest: String,
) -> Result<NomiRemoteSessionRow, NomiCoreApiError> {
    let current = if current.state == "failed" {
        append_remote_event_once(&state.remote_repository, owner.as_ref(), session.as_ref(), "session/cancelled", Some(&key),
            json!({"outcome":"cancelled","request_digest":request_digest,"preserved_failure":true})).await?;
        current
    } else {
        transition_remote_state(&state, &owner, &session, current, "cancelled", &key,
            json!({"outcome":"cancelled","request_digest":request_digest})).await?
    };
    retain_remote_source_browser_retirement(&state, owner.as_ref(), &session, &key, &target)?;
    Ok(current)
}
