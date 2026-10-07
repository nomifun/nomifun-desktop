use std::{fmt, sync::Arc};

use dashmap::DashMap;
use nomifun_api_types::{
    AttachmentDto, AutoWorkRunState, AutoWorkTargetKind, BoardResponse, CreateRequirementRequest,
    ListRequirementsQuery, Requirement, RequirementStatus, TagBinding, TagBindings, TagSummary,
    UpdateRequirementRequest,
};
use nomifun_common::{
    AppError, AttachmentId, ConversationId, PaginatedResult, RequirementId, TerminalId, UserId, now_ms,
};
use nomifun_db::models::RequirementRowUpdate;
use nomifun_db::{
    IRequirementRepository, ListRequirementsParams, RequirementClaim,
    RequirementClaimResolution,
};
use tracing::warn;

use crate::attachments::AttachmentStore;
use crate::autowork_config::{
    AutoWorkConfig, AutoWorkConfigSnapshot, AutoWorkSessionConfigCommand,
};
use crate::execution_port::{
    AutoWorkBindingLookup, AutoWorkExecutionSource, AutoWorkScheduledSessionLookup,
    AutoWorkSessionConfigPort, PersistedAutoWorkBinding, ScheduledAutoWorkSession,
    autowork_execution_operation_id,
};
use crate::convert::row_to_dto;
use crate::events::RequirementEventEmitter;
use crate::notifier::CompletionNotifier;
use crate::order_key::to_sort_seq;

/// Default claim lease (ms). The AutoWork runner renews well within this window.
pub const DEFAULT_LEASE_MS: i64 = 120_000;
/// Internal AutoWork claim envelope. `claim_generation` is durable and
/// monotonic even when the human-facing retry budget is reset.
#[derive(Clone)]
pub(crate) struct AutoWorkClaim {
    pub requirement: Requirement,
    pub claim_generation: i64,
    pub claim_token: String,
    pub recovered_active: bool,
}

pub(crate) struct AutoWorkDeletePark {
    pub affected: u64,
    pub execution_sources: Vec<AutoWorkExecutionSource>,
}

impl fmt::Debug for AutoWorkClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutoWorkClaim")
            .field("requirement", &self.requirement)
            .field("claim_generation", &self.claim_generation)
            .field("claim_token", &"<redacted>")
            .field("recovered_active", &self.recovered_active)
            .finish()
    }
}

/// Validate an AutoWork target handle in the conversation entity domain.
fn parse_conversation_id(target_id: &str) -> Result<&str, AppError> {
    ConversationId::try_from(target_id)
        .map(|_| target_id)
        .map_err(|_| AppError::NotFound(format!("conversation {target_id}")))
}

fn parse_terminal_id(target_id: &str) -> Result<&str, AppError> {
    TerminalId::try_from(target_id)
        .map(|_| target_id)
        .map_err(|_| AppError::NotFound(format!("terminal {target_id}")))
}

fn validate_requirement_id(id: &str) -> Result<&str, AppError> {
    RequirementId::parse(id)
        .map(|_| id)
        .map_err(|error| AppError::BadRequest(format!("invalid requirement id: {error}")))
}

fn validate_claim_token(token: &str) -> Result<&str, AppError> {
    if token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(token)
    } else {
        Err(AppError::BadRequest(
            "invalid Requirement claim capability".into(),
        ))
    }
}

fn validate_attachment_ids(ids: &[String]) -> Result<(), AppError> {
    for id in ids {
        AttachmentId::try_from(id.as_str())
            .map_err(|error| AppError::BadRequest(format!("invalid attachment id: {error}")))?;
    }
    Ok(())
}

/// Business logic for requirements (CRUD + AutoWork claim/finalize/config).
#[derive(Clone)]
pub struct RequirementService {
    repo: Arc<dyn IRequirementRepository>,
    emitter: RequirementEventEmitter,
    /// Attached for AutoWork config persistence (`extra.autowork`
    /// merge-write) through the current Session owner.
    session_config: Option<Arc<dyn AutoWorkSessionConfigPort>>,
    /// Canonical host projection used for boot-resume/admin enumeration of
    /// persisted Conversation AutoWork schedules.
    scheduled_session_lookup: Option<Arc<dyn AutoWorkScheduledSessionLookup>>,
    /// Fired (detached) after a requirement reaches a terminal state, so a bound
    /// webhook can notify. Optional + non-blocking —a failing webhook never
    /// affects requirement state.
    completion_notifier: Option<Arc<dyn CompletionNotifier>>,
    /// Notified whenever a requirement becomes claimable (created or re-pended),
    /// so idle AutoWork loops wake immediately instead of waiting for their poll
    /// fallback. Attached during assembly to the same `Notify` the AutoWork runner
    /// loops await on. `None` on instances that never drive AutoWork (the sink).
    autowork_waker: Option<Arc<tokio::sync::Notify>>,
    /// Attached for persistent image attachments (bind/copy/delete + AutoWork
    /// workspace staging). `None` on instances that never touch attachments
    /// (e.g. the declaration sink).
    attachments: Option<Arc<AttachmentStore>>,
    /// Serializes owner-scoped config CAS/write commands per target inside this
    /// host process. Conversation storage still performs its own transactional
    /// CAS; terminal storage uses this as its single-writer boundary.
    autowork_config_transitions:
        Arc<DashMap<(AutoWorkTargetKind, String), Arc<tokio::sync::Mutex<()>>>>,
}

impl RequirementService {
    pub fn new(repo: Arc<dyn IRequirementRepository>, emitter: RequirementEventEmitter) -> Self {
        Self {
            repo,
            emitter,
            session_config: None,
            scheduled_session_lookup: None,
            completion_notifier: None,
            autowork_waker: None,
            attachments: None,
            autowork_config_transitions: Arc::new(DashMap::new()),
        }
    }

    /// Attach the typed host-owned Session port for AutoWork configuration.
    pub fn with_session_config_port(
        mut self,
        session: Arc<dyn AutoWorkSessionConfigPort>,
    ) -> Self {
        self.session_config = Some(session);
        self
    }

    /// Attach the canonical host projection for persisted Conversation
    /// AutoWork schedules. This lookup does not own Session/runtime state.
    pub fn with_scheduled_session_lookup(
        mut self,
        lookup: Arc<dyn AutoWorkScheduledSessionLookup>,
    ) -> Self {
        self.scheduled_session_lookup = Some(lookup);
        self
    }

    /// Attach the completion notifier fired on terminal status transitions.
    pub fn with_completion_notifier(mut self, notifier: Arc<dyn CompletionNotifier>) -> Self {
        self.completion_notifier = Some(notifier);
        self
    }

    /// Attach the AutoWork waker. Shared with the runner: transitions that
    /// make a requirement claimable (`create`, re-pend) notify it so idle loops
    /// pick up new work without waiting for their poll fallback.
    pub fn with_autowork_waker(mut self, waker: Arc<tokio::sync::Notify>) -> Self {
        self.autowork_waker = Some(waker);
        self
    }

    /// Attach the attachment store (persistent requirement images).
    pub fn with_attachment_store(mut self, store: Arc<AttachmentStore>) -> Self {
        self.attachments = Some(store);
        self
    }

    /// Attachments of a requirement as DTOs; empty when no store is attached
    /// or on a read failure (display data must not fail the main call).
    async fn load_attachments(&self, requirement_id: &str) -> Vec<AttachmentDto> {
        let Some(store) = &self.attachments else { return Vec::new() };
        match store.list(requirement_id).await {
            Ok(rows) => rows.iter().map(|r| store.to_dto(r)).collect(),
            Err(e) => {
                warn!(error = %e, requirement_id, "failed to load requirement attachments");
                Vec::new()
            }
        }
    }

    /// Read-only half of conversation AutoWork attachment staging.
    pub(crate) async fn plan_attachments_for_prompt(
        &self,
        req_id: &str,
        workspace: Option<&std::path::Path>,
    ) -> Result<crate::attachments::PromptAttachmentPlan, AppError> {
        match &self.attachments {
            Some(store) => store.plan_for_prompt(req_id, workspace).await,
            None => Ok(crate::attachments::PromptAttachmentPlan::empty()),
        }
    }

    pub(crate) async fn activate_attachment_plan_with_operation_lease(
        &self,
        plan: &crate::attachments::PromptAttachmentPlan,
        operation_lease: Arc<dyn Send + Sync>,
    ) -> Result<(), AppError> {
        match &self.attachments {
            Some(store) => {
                store
                    .activate_prompt_plan_with_operation_lease(plan, operation_lease)
                    .await
            }
            None if plan.attachments.is_empty() => Ok(()),
            None => Err(AppError::Conflict(
                "AutoWork attachment store changed after prompt planning".to_owned(),
            )),
        }
    }

    /// Reconcile crash-safe attachment-delete journals after the caller has
    /// acquired process boot-reconciliation authority.
    pub async fn recover_pending_attachment_deletes(&self) -> Result<(), AppError> {
        if let Some(store) = &self.attachments {
            store.recover_pending_deletes().await?;
        }
        Ok(())
    }

    /// Wake idle AutoWork loops (no-op when no waker is attached). Called after a
    /// requirement becomes `pending` so a bound-but-idle session claims it now.
    fn wake_autowork(&self) {
        if let Some(waker) = &self.autowork_waker {
            waker.notify_waiters();
        }
    }

    /// Expose the repo for the AutoWork runner / sweeper (Phase C).
    pub fn repo(&self) -> &Arc<dyn IRequirementRepository> {
        &self.repo
    }

    pub async fn create(&self, mut req: CreateRequirementRequest) -> Result<Requirement, AppError> {
        let new_attachments = std::mem::take(&mut req.attachments);
        if req.title.trim().is_empty() {
            return Err(AppError::BadRequest("title must not be empty".into()));
        }
        if req.tag.trim().is_empty() {
            return Err(AppError::BadRequest("tag must not be empty".into()));
        }
        let now = now_ms();
        let order_key = req.order_key.unwrap_or_default();
        let status = req.status.unwrap_or(RequirementStatus::Pending);
        if status == RequirementStatus::InProgress {
            return Err(AppError::BadRequest(
                "in_progress is internal execution authority and cannot be created directly"
                    .into(),
            ));
        }
        let new_row = nomifun_db::models::NewRequirementRow {
            title: req.title,
            content: req.content,
            tag: req.tag,
            sort_seq: to_sort_seq(&order_key),
            order_key,
            status: status.as_db().to_string(),
            // `priority` is a reserved technical column. V3 ordering is
            // exclusively defined by `order_key`; callers cannot supply a
            // second ordering representation.
            priority: 0,
            completion_note: None,
            owner_conversation_id: None,
            owner_terminal_id: None,
            active_turn_started_at: None,
            lease_expires_at: None,
            started_at: None,
            completed_at: None,
            attempt_count: 0,
            created_by: req.created_by.unwrap_or_else(|| "user".to_string()),
            extra: "{}".to_string(),
            created_at: now,
            updated_at: now,
        };
        let row = self.repo.insert(&new_row).await?;
        let mut dto = row_to_dto(&row);
        if !new_attachments.is_empty() {
            let Some(store) = &self.attachments else {
                let _ = self.repo.delete(&row.requirement_id).await;
                return Err(AppError::Internal("attachment store not attached".into()));
            };
            match store
                .ingest(&row.requirement_id, &new_attachments, Some(&row.created_by))
                .await
            {
                Ok(rows) => dto.attachments = rows.iter().map(|r| store.to_dto(r)).collect(),
                Err(e) => {
                    // Keep create atomic for the caller: drop the row we just inserted.
                    if let Err(de) = self.repo.delete(&row.requirement_id).await {
                        warn!(error = %de, requirement_id = row.requirement_id, "rollback after attachment ingest failure failed");
                    }
                    return Err(e);
                }
            }
        }
        self.emitter.emit_created(&dto);
        // A freshly-created pending requirement is claimable now —wake idle loops.
        if dto.status == RequirementStatus::Pending {
            self.wake_autowork();
        }
        Ok(dto)
    }

    pub async fn get(&self, id: &str) -> Result<Requirement, AppError> {
        let id = validate_requirement_id(id)?;
        let row = self
            .repo
            .get_by_requirement_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("requirement {id}")))?;
        let mut dto = row_to_dto(&row);
        dto.attachments = self.load_attachments(id).await;
        Ok(dto)
    }

    pub async fn list(&self, query: &ListRequirementsQuery) -> Result<PaginatedResult<Requirement>, AppError> {
        if let Some(conversation_id) = query.conversation_id.as_deref() {
            parse_conversation_id(conversation_id)?;
        }
        let page = query.page.unwrap_or(1).max(1);
        let page_size = query.page_size.unwrap_or(20).clamp(1, 200);
        let params = ListRequirementsParams {
            tag: query.tag.clone(),
            status: query.status.map(|s| s.as_db().to_string()),
            owner_conversation_id: query.conversation_id.clone(),
            // The public list query has no kind filter —`conversation_id` here
            // is a UI filter that historically meant the conversation domain.
            owner_terminal_id: None,
            q: query.q.clone(),
            order_by: query.order_by.clone(),
            order: query.order.clone(),
            page: Some(page),
            page_size: Some(page_size),
        };
        let (rows, total) = self.repo.list(&params).await?;
        let items: Vec<Requirement> = rows.iter().map(row_to_dto).collect();
        let has_more = (page as u64) * (page_size as u64) < total;
        Ok(PaginatedResult { items, total, has_more })
    }

    pub async fn update(&self, id: &str, req: UpdateRequirementRequest) -> Result<Requirement, AppError> {
        let id = validate_requirement_id(id)?;
        validate_attachment_ids(&req.remove_attachment_ids)?;
        // Ensure it exists for a clean 404 (update() also returns NotFound).
        let original_row = self
            .repo
            .get_by_requirement_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("requirement {id}")))?;
        if req.title.as_deref().is_some_and(|title| title.trim().is_empty()) {
            return Err(AppError::BadRequest("title must not be empty".into()));
        }
        if req.tag.as_deref().is_some_and(|tag| tag.trim().is_empty()) {
            return Err(AppError::BadRequest("tag must not be empty".into()));
        }

        // Attachment changes first —ingest BEFORE remove. Ingest is the only
        // high-failure-probability step (validation, the temp source may already
        // be cleaned) and is all-or-nothing, so a failure here leaves the row and
        // its existing attachments completely untouched. Remove afterwards only
        // deletes DB rows + best-effort files and practically cannot fail; in the
        // extreme case it does, the freshly-ingested attachments are kept and a
        // retry of the same update converges (remove skips already-gone ids).
        let attachments_changed = !req.remove_attachment_ids.is_empty() || !req.add_attachments.is_empty();
        if attachments_changed {
            let Some(store) = &self.attachments else {
                return Err(AppError::Internal("attachment store not attached".into()));
            };
            store.ingest(id, &req.add_attachments, None).await?;
            store.remove(id, &req.remove_attachment_ids).await?;
        }

        let requested_status = req.status;
        let requested_note = req.completion_note;
        let mut params = RequirementRowUpdate {
            title: req.title,
            content: req.content,
            tag: req.tag,
            status: None,
            completion_note: if requested_status.is_none() {
                requested_note.clone().map(Some)
            } else {
                None
            },
            ..Default::default()
        };
        if let Some(ok) = req.order_key {
            params.sort_seq = Some(to_sort_seq(&ok));
            params.order_key = Some(ok);
        }
        // Attachment-only update: every row field is None, so repo.update would
        // early-return on its empty SET list and leave updated_at stale while we
        // still emit `requirement.updated`. Force the SQL path with an equal-value
        // field —repo.update stamps updated_at itself.
        let metadata_changed = params.title.is_some()
            || params.content.is_some()
            || params.tag.is_some()
            || params.completion_note.is_some()
            || params.order_key.is_some();
        let mut persisted_row = original_row;
        if metadata_changed {
            persisted_row = self.repo.update(id, &params).await?;
        } else if attachments_changed && requested_status.is_none() {
            persisted_row = self.repo.touch_updated_at(id, now_ms()).await?;
        }
        let mut dto = match requested_status {
            Some(status) => self.set_status(id, status, requested_note).await?,
            None => row_to_dto(&persisted_row),
        };
        dto.attachments = self.load_attachments(id).await;
        self.emitter.emit_updated(&dto);
        Ok(dto)
    }

    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        let id = validate_requirement_id(id)?;
        let prepared = match &self.attachments {
            Some(store) => Some((Arc::clone(store), store.prepare_delete_all(id).await?)),
            None => None,
        };
        if let Err(error) = self.repo.delete(id).await {
            if let Some((store, prepared)) = prepared
                && let Err(restore_error) = store.restore_prepared_delete(prepared).await
            {
                warn!(
                    error = %restore_error,
                    requirement_id = id,
                    "attachment rollback failed after requirement delete transaction failed"
                );
            }
            return Err(error.into());
        }
        if let Some((store, prepared)) = prepared {
            store.finish_prepared_delete(prepared).await;
        }
        self.emitter.emit_deleted(id);
        Ok(())
    }

    /// Delete many requirements by id. Missing ids are skipped (not an error).
    /// Returns the number actually deleted; emits `requirement.deleted` per row.
    pub async fn delete_many(&self, ids: &[String]) -> Result<u64, AppError> {
        for id in ids {
            validate_requirement_id(id)?;
        }
        let mut deleted = 0u64;
        for id in ids {
            let prepared = match &self.attachments {
                Some(store) => Some((Arc::clone(store), store.prepare_delete_all(id).await?)),
                None => None,
            };
            match self.repo.delete(id).await {
                Ok(()) => {
                    if let Some((store, prepared)) = prepared {
                        store.finish_prepared_delete(prepared).await;
                    }
                    self.emitter.emit_deleted(id);
                    deleted += 1;
                }
                Err(nomifun_db::DbError::NotFound(_)) => {
                    if let Some((store, prepared)) = prepared {
                        store.restore_prepared_delete(prepared).await?;
                    }
                }
                Err(error) => {
                    if let Some((store, prepared)) = prepared
                        && let Err(restore_error) = store.restore_prepared_delete(prepared).await
                    {
                        warn!(
                            error = %restore_error,
                            requirement_id = id,
                            "attachment rollback failed after batch requirement delete failed"
                        );
                    }
                    return Err(error.into());
                }
            }
        }
        Ok(deleted)
    }

    pub async fn tags(&self) -> Result<Vec<TagSummary>, AppError> {
        let counts = self.repo.tag_status_counts().await?;
        let mut summaries: Vec<TagSummary> = Vec::new();
        for (tag, status, count) in counts {
            let entry = match summaries.iter_mut().find(|s| s.tag == tag) {
                Some(e) => e,
                None => {
                    summaries.push(TagSummary {
                        tag: tag.clone(),
                        ..Default::default()
                    });
                    summaries.last_mut().unwrap()
                }
            };
            match status.as_str() {
                "pending" => entry.pending += count,
                "in_progress" => entry.in_progress += count,
                "done" => entry.done += count,
                "failed" => entry.failed += count,
                "cancelled" => entry.cancelled += count,
                "needs_review" => entry.needs_review += count,
                _ => {}
            }
            entry.total += count;
        }
        // Annotate AutoWork pause state per tag (tag count is small).
        for summary in &mut summaries {
            if let Some(st) = self.repo.get_tag_state(&summary.tag).await? {
                summary.paused = st.is_paused();
                summary.paused_reason = if st.is_paused() { st.paused_reason } else { None };
            }
        }
        Ok(summaries)
    }

    pub async fn board(&self, tag: &str) -> Result<BoardResponse, AppError> {
        let rows = self.repo.list_by_tag(tag).await?;
        let mut board = BoardResponse {
            tag: tag.to_string(),
            pending: Vec::new(),
            in_progress: Vec::new(),
            done: Vec::new(),
            failed: Vec::new(),
            cancelled: Vec::new(),
            needs_review: Vec::new(),
        };
        for row in &rows {
            let dto = row_to_dto(row);
            match RequirementStatus::from_db(&row.status) {
                RequirementStatus::Pending => board.pending.push(dto),
                RequirementStatus::InProgress => board.in_progress.push(dto),
                RequirementStatus::Done => board.done.push(dto),
                RequirementStatus::Failed => board.failed.push(dto),
                RequirementStatus::Cancelled => board.cancelled.push(dto),
                RequirementStatus::NeedsReview => board.needs_review.push(dto),
            }
        }
        Ok(board)
    }

    /// Test-only shorthand. Production claim allocation is deliberately
    /// available only through [`Self::claim_next_for_runner`], which returns
    /// the exact generation and opaque capability needed to close the turn.
    #[cfg(test)]
    pub(crate) async fn claim_next(
        &self,
        tag: &str,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        lease_ms: i64,
    ) -> Result<Option<Requirement>, AppError> {
        Ok(self
            .claim_next_for_runner(tag, owner_id, kind, lease_ms)
            .await?
            .map(|claim| claim.requirement))
    }

    /// Runner-only claim boundary that carries the durable generation used to
    /// namespace the Conversation delivery receipt. A restarted runner may get
    /// the same active claim and therefore the same generation.
    pub(crate) async fn claim_next_for_runner(
        &self,
        tag: &str,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        lease_ms: i64,
    ) -> Result<Option<AutoWorkClaim>, AppError> {
        let (owner_conversation_id, owner_terminal_id) = match kind {
            AutoWorkTargetKind::Conversation => (Some(parse_conversation_id(owner_id)?), None),
            AutoWorkTargetKind::Terminal => (None, Some(parse_terminal_id(owner_id)?)),
        };
        let claim = self
            .repo
            .claim_next_for_runner(
                tag,
                owner_conversation_id,
                owner_terminal_id,
                lease_ms,
                now_ms(),
            )
            .await?;
        self.finish_runner_claim(claim)
    }

    /// Recover only an existing active claim. Unlike
    /// [`Self::claim_next_for_runner`], this never allocates a pending
    /// requirement. Terminal loops call it before the PTY liveness gate so a
    /// pre-restart injection with unknown outcome is parked instead of waiting
    /// for a relaunch and then being injected again.
    pub(crate) async fn recover_active_claim_for_runner(
        &self,
        tag: &str,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        lease_ms: i64,
    ) -> Result<Option<AutoWorkClaim>, AppError> {
        let (owner_conversation_id, owner_terminal_id) = match kind {
            AutoWorkTargetKind::Conversation => (Some(parse_conversation_id(owner_id)?), None),
            AutoWorkTargetKind::Terminal => (None, Some(parse_terminal_id(owner_id)?)),
        };
        let claim = self
            .repo
            .recover_active_claim_for_runner(
                tag,
                owner_conversation_id,
                owner_terminal_id,
                lease_ms,
                now_ms(),
            )
            .await?;
        self.finish_runner_claim(claim)
    }

    fn finish_runner_claim(
        &self,
        claim: Option<RequirementClaim>,
    ) -> Result<Option<AutoWorkClaim>, AppError> {
        let Some(claim) = claim else {
            return Ok(None);
        };
        let claim_token = claim.row.claim_token.clone().ok_or_else(|| {
            AppError::Internal(format!(
                "active Requirement {} has no durable claim capability",
                claim.row.requirement_id
            ))
        })?;
        validate_claim_token(&claim_token).map_err(|_| {
            AppError::Internal(format!(
                "active Requirement {} has an invalid durable claim capability",
                claim.row.requirement_id
            ))
        })?;
        let requirement = row_to_dto(&claim.row);
        self.emitter.emit_status_changed(&requirement);
        Ok(Some(AutoWorkClaim {
            requirement,
            claim_generation: claim.row.claim_generation,
            claim_token,
            recovered_active: claim.recovered_active,
        }))
    }

    /// Agent-facing claim hides the opaque claim capability while binding the
    /// durable generation to the authenticated AgentSession. Later status
    /// Actions reload that capability from the owner row; it never crosses a
    /// model-visible schema.
    pub async fn claim_next_for_agent_session(
        &self,
        tag: &str,
        agent_session_id: &str,
    ) -> Result<Option<Requirement>, AppError> {
        let claim = self
            .claim_next_for_runner(
                tag,
                agent_session_id,
                AutoWorkTargetKind::Conversation,
                DEFAULT_LEASE_MS,
            )
            .await?;
        Ok(claim.map(|claim| claim.requirement))
    }

    pub async fn set_status_for_agent_session(
        &self,
        requirement_id: &str,
        agent_session_id: &str,
        status: RequirementStatus,
        completion_note: Option<String>,
    ) -> Result<Requirement, AppError> {
        let requirement_id = validate_requirement_id(requirement_id)?;
        let agent_session_id = parse_conversation_id(agent_session_id)?;
        let row = self
            .repo
            .get_by_requirement_id(requirement_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("requirement {requirement_id}")))?;
        if row.status == RequirementStatus::InProgress.as_db()
            && row.owner_conversation_id.as_deref() == Some(agent_session_id)
        {
            let token = row.claim_token.as_deref().ok_or_else(|| {
                AppError::Conflict("active Requirement has no exact claim capability".into())
            })?;
            if status == RequirementStatus::Pending {
                let Some(requirement) = self
                    .release_claim_exact_requirement(
                        requirement_id,
                        agent_session_id,
                        row.claim_generation,
                        token,
                    )
                    .await?
                else {
                    return Err(AppError::Conflict(
                        "Requirement claim changed before it could be released".into(),
                    ));
                };
                return Ok(requirement);
            }
            return self
                .resolve_claim_verdict_exact(
                    requirement_id,
                    row.claim_generation,
                    token,
                    agent_session_id,
                    AutoWorkTargetKind::Conversation,
                    status,
                    completion_note,
                )
                .await?
                .ok_or_else(|| {
                    AppError::Conflict(
                        "Requirement claim changed before its status receipt committed".into(),
                    )
                });
        }
        self.update(
            requirement_id,
            UpdateRequirementRequest {
                title: None,
                content: None,
                tag: None,
                order_key: None,
                status: Some(status),
                completion_note,
                add_attachments: Vec::new(),
                remove_attachment_ids: Vec::new(),
            },
        )
        .await
    }

    /// Renew the lease for `id` held by `owner_id` in the requested owner domain.
    /// Returns whether a row matched.
    pub async fn renew_lease(
        &self,
        id: &str,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        expected_generation: i64,
        expected_claim_token: &str,
        lease_ms: i64,
    ) -> Result<bool, AppError> {
        let id = validate_requirement_id(id)?;
        let expected_claim_token = validate_claim_token(expected_claim_token)?;
        let owners = match kind {
            AutoWorkTargetKind::Conversation => (Some(parse_conversation_id(owner_id)?), None),
            AutoWorkTargetKind::Terminal => (None, Some(parse_terminal_id(owner_id)?)),
        };
        Ok(self
            .repo
            .renew_lease(
                id,
                owners.0,
                owners.1,
                expected_generation,
                expected_claim_token,
                lease_ms,
                now_ms(),
            )
            .await?)
    }

    /// Verify one capability against the exact active durable claim. This is a
    /// preflight only; the eventual verdict still uses the repository's exact
    /// generation CAS to close the check/write race.
    pub async fn verify_active_claim_exact(
        &self,
        id: &str,
        expected_generation: i64,
        expected_claim_token: &str,
        owner_conversation_id: Option<&str>,
        owner_terminal_id: Option<&str>,
    ) -> Result<bool, AppError> {
        let id = validate_requirement_id(id)?;
        let expected_claim_token = validate_claim_token(expected_claim_token)?;
        let owners = match (owner_conversation_id, owner_terminal_id) {
            (Some(conversation_id), None) => {
                (Some(parse_conversation_id(conversation_id)?), None)
            }
            (None, Some(terminal_id)) => (None, Some(parse_terminal_id(terminal_id)?)),
            _ => {
                return Err(AppError::BadRequest(
                    "an exact Requirement capability must name exactly one owner domain".into(),
                ));
            }
        };
        let Some(row) = self.repo.get_by_requirement_id(id).await? else {
            return Ok(false);
        };
        Ok(row.status == "in_progress"
            && row.claim_generation == expected_generation
            && row.claim_token.as_deref() == Some(expected_claim_token)
            && row.owner_conversation_id.as_deref() == owners.0
            && row.owner_terminal_id.as_deref() == owners.1)
    }

    /// The user manually cancelled an AutoWork-driven turn —treat it as an
    /// explicit "stop working on this" signal, NOT a failed attempt:
    /// 1. pause the tag (reason `user_interrupted`, resumable from the UI) so
    ///    the persistent loop does not immediately re-claim and re-inject the
    ///    same requirement —the historical "I paused it and seconds later it
    ///    was running again";
    /// 2. release the claim back to `pending` WITHOUT consuming an attempt.
    /// Ordered pause-first so the release's wake cannot race a re-claim (the
    /// claim SQL skips paused tags). Best-effort on the pause write: a failure
    /// must not block the claim release.
    /// Release only the exact durable conversation claim generation. A late
    /// stop from an older runner cannot unclaim a newer turn owned by the same
    /// conversation.
    pub async fn release_claim_exact(
        &self,
        id: &str,
        conversation_id: &str,
        expected_generation: i64,
        expected_claim_token: &str,
    ) -> Result<bool, AppError> {
        Ok(self
            .release_claim_exact_requirement(
                id,
                conversation_id,
                expected_generation,
                expected_claim_token,
            )
            .await?
            .is_some())
    }

    async fn release_claim_exact_requirement(
        &self,
        id: &str,
        conversation_id: &str,
        expected_generation: i64,
        expected_claim_token: &str,
    ) -> Result<Option<Requirement>, AppError> {
        let id = validate_requirement_id(id)?;
        let conversation_id = parse_conversation_id(conversation_id)?;
        let expected_claim_token = validate_claim_token(expected_claim_token)?;
        let released = self
            .repo
            .abandon_claim_before_admission_exact(
                id,
                Some(conversation_id),
                None,
                expected_generation,
                expected_claim_token,
                now_ms(),
            )
            .await?;
        let requirement = released.map(|updated| {
            let requirement = row_to_dto(&updated);
            self.emitter.emit_status_changed(&requirement);
            self.wake_autowork();
            requirement
        });
        Ok(requirement)
    }

    /// Pause without changing the active claim. The runner must use durable
    /// delivery evidence to choose exact-unclaim versus exact NeedsReview.
    pub async fn pause_for_user_interrupt(&self, id: &str, tag: &str) -> Result<(), AppError> {
        self.pause_for_execution_attention(id, tag, "user_interrupted")
            .await
    }

    /// Pause queue selection after AgentExecution reaches a durable attention
    /// or failure boundary. AgentExecution, not Requirement, owns retry.
    pub async fn pause_for_execution_attention(
        &self,
        id: &str,
        tag: &str,
        reason: &str,
    ) -> Result<(), AppError> {
        let id = validate_requirement_id(id)?;
        if !matches!(
            reason,
            "user_interrupted" | "user_action_required" | "execution_failed"
        ) {
            return Err(AppError::BadRequest(
                "unknown AutoWork execution attention reason".to_owned(),
            ));
        }
        match self.repo.pause_tag(tag, reason, Some(id), now_ms()).await {
            Ok(()) => self.emitter.emit_tag_paused(&nomifun_api_types::TagPausedPayload {
                tag: tag.to_string(),
                reason: reason.to_string(),
                requirement_id: Some(id.to_owned()),
            }),
            Err(e) => warn!(
                tag,
                requirement_id = id,
                error = %e,
                "Failed to pause tag for AgentExecution attention"
            ),
        }
        Ok(())
    }

    /// Agent/user self-update: set status, with timestamps. `done` sets
    /// `completed_at` (+ optional note); `failed` records the note. Idempotent on
    /// any terminal state (re-setting the same status is a no-op). Rejects
    /// transitions out of a terminal state (`done`/`failed`/`cancelled`).
    pub async fn set_status(
        &self,
        id: &str,
        status: RequirementStatus,
        note: Option<String>,
    ) -> Result<Requirement, AppError> {
        let id = validate_requirement_id(id)?;
        if status == RequirementStatus::InProgress {
            return Err(AppError::BadRequest(
                "requirements may enter in_progress only through the atomic AutoWork claim allocator"
                    .into(),
            ));
        }
        let row = self
            .repo
            .get_by_requirement_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("requirement {id}")))?;

        // Idempotent: re-setting the current status is a no-op (covers done->done,
        // failed->failed, cancelled->cancelled, and avoids a duplicate WS event).
        if row.status == status.as_db() {
            return Ok(row_to_dto(&row));
        }

        if status == RequirementStatus::Pending {
            if !matches!(row.status.as_str(), "failed" | "needs_review") {
                return Err(AppError::BadRequest(format!(
                    "requirement {id} cannot be explicitly requeued from {}",
                    row.status
                )));
            }
            let updated = self
                .repo
                .requeue_for_resume_exact(
                    id,
                    &row.status,
                    row.claim_generation,
                    false,
                    now_ms(),
                )
                .await?
                .ok_or_else(|| {
                    AppError::Conflict(format!(
                        "requirement {id} changed while applying an explicit requeue"
                    ))
                })?;
            let dto = row_to_dto(&updated);
            self.emitter.emit_status_changed(&dto);
            self.wake_autowork();
            return Ok(dto);
        }

        if row.status == "in_progress" {
            return Err(AppError::BadRequest(
                "active Requirement verdicts require the exact internal claim capability".into(),
            ));
        }

        // A terminal requirement is frozen: reject transitions out of done/failed/
        // cancelled. (Re-running a requirement means creating a new one.)
        if matches!(row.status.as_str(), "done" | "failed" | "cancelled") {
            return Err(AppError::BadRequest(format!(
                "requirement {id} is {} and cannot transition to {}",
                row.status,
                status.as_db()
            )));
        }

        let now = now_ms();
        let writes_note = matches!(
            status,
            RequirementStatus::Done
                | RequirementStatus::Failed
                | RequirementStatus::NeedsReview
        );
        let updated = match self
            .repo
            .transition_status_if_current(
                id,
                &row.status,
                status.as_db(),
                writes_note,
                note.as_deref(),
                status == RequirementStatus::InProgress,
                status == RequirementStatus::Done,
                now,
            )
            .await?
        {
            Some(updated) => updated,
            None => {
                let current = self
                    .repo
                    .get_by_requirement_id(id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("requirement {id}")))?;
                if current.status == status.as_db() {
                    return Ok(row_to_dto(&current));
                }
                if matches!(current.status.as_str(), "done" | "failed" | "cancelled") {
                    return Err(AppError::BadRequest(format!(
                        "requirement {id} is {} and cannot transition to {}",
                        current.status,
                        status.as_db()
                    )));
                }
                return Err(AppError::Conflict(format!(
                    "requirement {id} changed from {} to {} while applying {}",
                    row.status,
                    current.status,
                    status.as_db()
                )));
            }
        };
        let dto = row_to_dto(&updated);
        self.emitter.emit_status_changed(&dto);

        // Fire the completion notifier on terminal transitions. The early-returns
        // above guarantee this is a genuine change out of a non-terminal state, so
        // this runs at most once per requirement. Detached + best-effort: a slow or
        // failing webhook must never block or fail the status transition.
        // `NeedsReview` is included because it is exactly a "human, please look"
        // signal worth notifying on, even though it is not a frozen terminal state.
        if matches!(
            status,
            RequirementStatus::Done | RequirementStatus::Failed | RequirementStatus::NeedsReview
        ) && let Some(notifier) = &self.completion_notifier
        {
            let notifier = notifier.clone();
            let row = updated.clone();
            tokio::spawn(async move {
                notifier.notify_completion(&row).await;
            });
        }
        Ok(dto)
    }

    /// Convenience: mark done with a completion note.
    pub async fn complete(&self, id: &str, completion_note: Option<String>) -> Result<Requirement, AppError> {
        let id = validate_requirement_id(id)?;
        self.set_status(id, RequirementStatus::Done, completion_note).await
    }

    /// Broadcast an AutoWork state change (used by the routes layer).
    pub fn emit_autowork_state(&self, state: &nomifun_api_types::AutoWorkState) {
        self.emitter.emit_autowork_changed(state);
    }

    fn autowork_config_transition(
        &self,
        kind: AutoWorkTargetKind,
        target_id: &str,
    ) -> Arc<tokio::sync::Mutex<()>> {
        self.autowork_config_transitions
            .entry((kind, target_id.to_owned()))
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Persist one owner-scoped AgentSession AutoWork queue binding.
    pub async fn save_autowork_config(
        &self,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        target_id: &str,
        config: AutoWorkConfig,
        expected_revision: &str,
        operation_id: Option<&str>,
    ) -> Result<AutoWorkConfigSnapshot, AppError> {
        let owner_id = UserId::parse(owner_id)
            .map_err(|error| AppError::Forbidden(format!("invalid caller identity: {error}")))?;
        if kind != AutoWorkTargetKind::Conversation {
            return Err(AppError::BadRequest(
                "Terminal AutoWork was retired; bind an AgentPreset Session instead".to_owned(),
            ));
        }
        let canonical = AutoWorkConfig::normalize(
            config.enabled,
            config.tag.as_deref(),
            config.max_requirements,
        )?;
        if canonical != config {
            return Err(AppError::BadRequest(
                "AutoWork config must use its canonical normalized tag".to_owned(),
            ));
        }
        if expected_revision.trim().is_empty() {
            return Err(AppError::Conflict(
                "AutoWork config write requires an expected revision".to_owned(),
            ));
        }
        if operation_id.is_some_and(|operation_id| operation_id.trim().is_empty()) {
            return Err(AppError::BadRequest(
                "AutoWork config operation identity must not be empty".to_owned(),
            ));
        }
        let target_id = parse_conversation_id(target_id)?;
        let transition = self.autowork_config_transition(kind, target_id);
        let _transition_guard = transition.lock().await;
        let Some(session_config) = &self.session_config else {
            return Err(AppError::Internal(
                "AutoWork AgentSession config port not attached".into(),
            ));
        };
        let snapshot = session_config
            .save_config(AutoWorkSessionConfigCommand {
                owner_id: owner_id.as_str().to_owned(),
                session_id: target_id.to_owned(),
                config: config.clone(),
                expected_revision: expected_revision.to_owned(),
                operation_id: operation_id.map(str::to_owned),
            })
            .await?;
        if snapshot.config != config
            || (operation_id.is_some() && snapshot.operation_id.as_deref() != operation_id)
        {
            return Err(AppError::Conflict(
                "AgentSession owner returned a different AutoWork config".to_owned(),
            ));
        }
        Ok(snapshot)
    }

    /// Read one owner-scoped AgentSession AutoWork queue binding.
    pub async fn read_autowork_config_snapshot(
        &self,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        target_id: &str,
    ) -> Result<AutoWorkConfigSnapshot, AppError> {
        let owner_id = UserId::parse(owner_id)
            .map_err(|error| AppError::Forbidden(format!("invalid caller identity: {error}")))?;
        if kind != AutoWorkTargetKind::Conversation {
            return Err(AppError::BadRequest(
                "Terminal AutoWork was retired; bind an AgentPreset Session instead".to_owned(),
            ));
        }
        let Some(session_config) = &self.session_config else {
            return Err(AppError::Internal(
                "AutoWork AgentSession config port not attached".into(),
            ));
        };
        let snapshot = session_config
            .read_config(owner_id.as_str(), parse_conversation_id(target_id)?)
            .await?;
        let canonical = AutoWorkConfig::normalize(
            snapshot.config.enabled,
            snapshot.config.tag.as_deref(),
            snapshot.config.max_requirements,
        )
        .map_err(|error| {
            AppError::Conflict(format!(
                "AgentSession owner returned invalid AutoWork config for {target_id}: {error}"
            ))
        })?;
        if canonical != snapshot.config {
            return Err(AppError::Conflict(format!(
                "AgentSession owner returned non-canonical AutoWork config for {target_id}"
            )));
        }
        AutoWorkConfigSnapshot::new(
            snapshot.config,
            snapshot.revision,
            snapshot.operation_id,
        )
    }

    /// Project an explicit durable verdict onto exactly one AutoWork claim
    /// generation. A late receipt/runner from generation N cannot overwrite a
    /// manual requeue or active generation N+1.
    pub async fn resolve_claim_verdict_exact(
        &self,
        id: &str,
        expected_generation: i64,
        expected_claim_token: &str,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        status: RequirementStatus,
        note: Option<String>,
    ) -> Result<Option<Requirement>, AppError> {
        let id = validate_requirement_id(id)?;
        let expected_claim_token = validate_claim_token(expected_claim_token)?;
        let owners = match kind {
            AutoWorkTargetKind::Conversation => (Some(parse_conversation_id(owner_id)?), None),
            AutoWorkTargetKind::Terminal => (None, Some(parse_terminal_id(owner_id)?)),
        };
        let note = note.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
        let resolution = match status {
            RequirementStatus::Done => RequirementClaimResolution::Done {
                completion_note: note,
            },
            RequirementStatus::Failed => RequirementClaimResolution::Failed {
                completion_note: note,
            },
            RequirementStatus::NeedsReview => RequirementClaimResolution::NeedsReview {
                completion_note: note,
            },
            RequirementStatus::Cancelled => RequirementClaimResolution::Cancelled {
                completion_note: note,
            },
            RequirementStatus::Pending | RequirementStatus::InProgress => {
                return Err(AppError::BadRequest(format!(
                    "exact AutoWork verdict cannot resolve a claim to {}",
                    status.as_db()
                )));
            }
        };

        let Some(updated) = self
            .repo
            .resolve_claim_exact(
                id,
                expected_generation,
                expected_claim_token,
                owners.0,
                owners.1,
                &resolution,
                now_ms(),
            )
            .await?
        else {
            return Ok(None);
        };

        let dto = row_to_dto(&updated);
        self.emitter.emit_status_changed(&dto);
        if matches!(
            dto.status,
            RequirementStatus::Done
                | RequirementStatus::Failed
                | RequirementStatus::Cancelled
                | RequirementStatus::NeedsReview
        ) && let Some(notifier) = &self.completion_notifier
        {
            let notifier = notifier.clone();
            let row = updated.clone();
            tokio::spawn(async move {
                notifier.notify_completion(&row).await;
            });
        }
        Ok(Some(dto))
    }

    /// Confirm that a concurrent writer already committed the exact terminal
    /// verdict. This read cannot retry, requeue, or create another execution.
    pub async fn confirm_claim_verdict_exact(
        &self,
        id: &str,
        expected_generation: i64,
        expected_claim_token: &str,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        status: RequirementStatus,
    ) -> Result<bool, AppError> {
        let id = validate_requirement_id(id)?;
        let expected_claim_token = validate_claim_token(expected_claim_token)?;
        let owners = match kind {
            AutoWorkTargetKind::Conversation => (Some(parse_conversation_id(owner_id)?), None),
            AutoWorkTargetKind::Terminal => (None, Some(parse_terminal_id(owner_id)?)),
        };
        let Some(row) = self.repo.get_by_requirement_id(id).await? else {
            return Ok(false);
        };
        Ok(row.claim_generation == expected_generation
            && row.claim_token.as_deref() == Some(expected_claim_token)
            && row.owner_conversation_id.as_deref() == owners.0
            && row.owner_terminal_id.as_deref() == owners.1
            && row.status == status.as_db())
    }

    /// Whether `tag` is currently paused (AutoWork halted for it).
    pub async fn is_tag_paused(&self, tag: &str) -> Result<bool, AppError> {
        Ok(self.repo.is_tag_paused(tag).await?)
    }

    /// Read the durable pause projection used by both REST snapshots and live
    /// AutoWork status events. A missing tag-state row is the canonical active
    /// state; stale reasons are never exposed after resume.
    pub async fn tag_pause_state(&self, tag: &str) -> Result<(bool, Option<String>), AppError> {
        let Some(state) = self.repo.get_tag_state(tag).await? else {
            return Ok((false, None));
        };
        if state.is_paused() {
            Ok((true, state.paused_reason))
        } else {
            Ok((false, None))
        }
    }

    /// Resume a paused tag. Optionally re-queue specific failed requirements back
    /// to `pending` (clearing their consumed attempts) so they retry from
    /// scratch. Wakes idle AutoWork loops so the tag's work resumes immediately.
    pub async fn resume_tag(&self, tag: &str, requeue_ids: &[String]) -> Result<(), AppError> {
        for id in requeue_ids {
            validate_requirement_id(id)?;
        }
        for updated in self
            .repo
            .resume_tag_with_requeues(tag, requeue_ids, now_ms())
            .await?
        {
            self.emitter.emit_status_changed(&row_to_dto(&updated));
        }
        // Tag is active again (+ any requeued rows are pending) —wake idle loops.
        self.wake_autowork();
        Ok(())
    }

    /// Resume a tag because AutoWork was explicitly (re-)ENABLED on a session
    /// bound to it. A paused tag (prior `requirement_failed`, or a deleted-session
    /// cascade) otherwise silently blocks EVERY conversation bound to the same tag
    /// —the user toggles AutoWork on and nothing happens, with no per-conversation
    /// indication that the shared tag is paused (the recurring "nothing runs"
    /// trap).
    ///
    /// An explicit enable unpauses the tag and refreshes retry budgets for
    /// `failed`/`pending` work. An `in_progress` row is different: process or
    /// lease loss cannot prove that its model/tool/PTY side effects never
    /// started, so it is parked in `needs_review` with its owner and durable
    /// claim generation intact. Rows already parked for review and terminal
    /// rows (`done` / `cancelled`) are left untouched.
    pub async fn resume_tag_for_enable(&self, tag: &str) -> Result<(), AppError> {
        if !self.repo.is_tag_paused(tag).await? {
            return Ok(());
        }
        let note = "AutoWork was re-enabled while this durable claim had an unknown \
                    execution outcome; it was not executed again.";
        for updated in self
            .repo
            .resume_tag_for_enable_atomic(tag, note, now_ms())
            .await?
        {
            self.emitter.emit_status_changed(&row_to_dto(&updated));
        }
        self.wake_autowork();
        Ok(())
    }


    /// WITHOUT consuming an attempt (the turn never ran). Wakes loops to retry.
    pub async fn unclaim_busy(
        &self,
        id: &str,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        expected_generation: i64,
        expected_claim_token: &str,
    ) -> Result<bool, AppError> {
        let id = validate_requirement_id(id)?;
        let expected_claim_token = validate_claim_token(expected_claim_token)?;
        let owners = match kind {
            AutoWorkTargetKind::Conversation => (Some(parse_conversation_id(owner_id)?), None),
            AutoWorkTargetKind::Terminal => (None, Some(parse_terminal_id(owner_id)?)),
        };
        let abandoned = self
            .repo
            .abandon_claim_before_admission_exact(
                id,
                owners.0,
                owners.1,
                expected_generation,
                expected_claim_token,
                now_ms(),
            )
            .await?;
        if let Some(updated) = &abandoned {
            self.emitter.emit_status_changed(&row_to_dto(updated));
            self.wake_autowork();
        }
        Ok(abandoned.is_some())
    }

    /// Reconcile every Requirement bound to a now-deleted session.
    ///
    /// The owner columns intentionally have no cross-table FK, so a deleted
    /// conversation/terminal does not cascade-clear them. Without this hook a requirement claimed by a
    /// since-deleted session would keep a dangling owner and, if it was
    /// `in_progress`, sit orphaned until the lease sweeper happened to run.
    ///
    /// Inactive rows are detached. An `in_progress` or already parked
    /// `needs_review` row retains its typed owner as durable evidence and is
    /// parked in `needs_review`, preserving its capability, claim generation
    /// and effects-start timestamp: deletion cannot prove that the prior
    /// model/PTY turn had not already crossed its irreversible boundary. It
    /// must never make that work claimable again.
    ///
    /// CALL SITE (Phase 3/4 wiring): invoke from the conversations + terminal
    /// deletion paths (`nomifun-conversation` / `nomifun-terminal` delete) with
    /// the deleted session id AND its domain. Exposed here so the deletion path
    /// can call it without the DB layer (the FK that would have cascaded does
    /// not exist).
    ///
    /// SECURITY (spec §2.2): the query is scoped to the owner column for the
    /// requested domain. Clearing a conversation can never release terminal
    /// work, and vice versa.
    pub async fn clear_owner_for_session(
        &self,
        session_id: &str,
        kind: AutoWorkTargetKind,
    ) -> Result<u64, AppError> {
        Ok(self
            .park_owner_for_session_delete(session_id, kind)
            .await?
            .affected)
    }

    pub(crate) async fn park_owner_for_session_delete(
        &self,
        session_id: &str,
        kind: AutoWorkTargetKind,
    ) -> Result<AutoWorkDeletePark, AppError> {
        let session_id = match kind {
            AutoWorkTargetKind::Conversation => parse_conversation_id(session_id)?,
            AutoWorkTargetKind::Terminal => parse_terminal_id(session_id)?,
        };
        let note = format!(
            "{} session {} was deleted while AutoWork could still be executing; active claims were parked for review and their typed owner/generation evidence was retained.",
            kind.as_str(),
            session_id
        );
        let rows = self
            .repo
            .detach_owner_for_session(
                (kind == AutoWorkTargetKind::Conversation).then_some(session_id),
                (kind == AutoWorkTargetKind::Terminal).then_some(session_id),
                &note,
                now_ms(),
            )
            .await?;
        for row in &rows {
            self.emitter.emit_status_changed(&row_to_dto(row));
        }
        let affected = rows.len() as u64;
        let execution_sources = rows
            .into_iter()
            .filter(|row| row.status == RequirementStatus::NeedsReview.as_db())
            .map(|row| {
                let claim_token = row.claim_token.ok_or_else(|| {
                    AppError::Conflict(format!(
                        "parked Requirement {} has no exact claim capability",
                        row.requirement_id
                    ))
                })?;
                validate_claim_token(&claim_token)?;
                Ok(AutoWorkExecutionSource {
                    operation_id: autowork_execution_operation_id(
                        &row.requirement_id,
                        row.claim_generation,
                        &claim_token,
                    ),
                    requirement_id: row.requirement_id,
                    claim_generation: row.claim_generation,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(AutoWorkDeletePark {
            affected,
            execution_sources,
        })
    }

    /// Enumerate validated, enabled AutoWork bindings from the compatible
    /// persisted Conversation/Terminal representations.
    ///
    /// This is the Requirement-side implementation of the typed boot-resume
    /// lookup contract. Storage details stay here; callers never parse
    /// Conversation `extra` or terminal JSON.
    async fn collect_enabled_autowork_bindings(
        &self,
        user_id: &str,
    ) -> Result<Vec<PersistedAutoWorkBinding>, AppError> {
        let user_id = UserId::parse(user_id)
            .map_err(|error| AppError::Forbidden(format!("invalid caller identity: {error}")))?;
        let user_id = user_id.as_str();
        let mut bindings = Vec::new();

        if let Some(lookup) = &self.scheduled_session_lookup {
            let scan = lookup.list_enabled_scheduled_sessions(user_id).await?;
            for issue in scan.quarantined {
                warn!(
                    target_id = issue.target_id.as_deref().unwrap_or("<unknown>"),
                    code = issue.code,
                    detail = issue.detail,
                    "Quarantined malformed Conversation AutoWork binding"
                );
            }
            for scheduled in scan.sessions {
                match scheduled_session_binding(scheduled) {
                    Ok(binding) => bindings.push(binding),
                    Err(error) => warn!(
                        error = %error,
                        "Quarantined invalid typed Conversation AutoWork binding"
                    ),
                }
            }
        } else if self.session_config.is_some() {
            return Err(AppError::Conflict(
                "AutoWork scheduled-session lookup is not wired by the canonical Session facade"
                    .to_owned(),
            ));
        }

        Ok(bindings)
    }

    /// Enumerate AutoWork tag/session bindings for `user_id`, grouped by tag.
    ///
    /// The public API projection is derived from the same typed lookup used by
    /// boot resume, so admin listing and runtime startup cannot disagree about
    /// persisted binding semantics.
    pub async fn tag_bindings(&self, user_id: &str) -> Result<Vec<TagBindings>, AppError> {
        let mut by_tag: std::collections::BTreeMap<String, Vec<TagBinding>> =
            std::collections::BTreeMap::new();
        for binding in self.collect_enabled_autowork_bindings(user_id).await? {
            by_tag
                .entry(binding.tag.clone())
                .or_default()
                .push(TagBinding {
                    kind: binding.kind,
                    target_id: binding.target_id,
                    name: binding.display_name,
                    run_state: AutoWorkRunState::Idle,
                });
        }
        Ok(by_tag
            .into_iter()
            .map(|(tag, bindings)| TagBindings { tag, bindings })
            .collect())
    }
}

fn scheduled_session_binding(
    scheduled: ScheduledAutoWorkSession,
) -> Result<PersistedAutoWorkBinding, AppError> {
    let session_id = parse_conversation_id(&scheduled.session_id)?;
    let config = AutoWorkConfig::normalize(
        true,
        Some(&scheduled.tag),
        scheduled.max_requirements,
    )
    .map_err(|error| {
        AppError::Conflict(format!(
            "AutoWork scheduled session {session_id} has invalid config: {error}"
        ))
    })?;
    let snapshot = AutoWorkConfigSnapshot::new(
        config,
        scheduled.config_revision,
        None,
    )?;
    Ok(PersistedAutoWorkBinding {
        kind: AutoWorkTargetKind::Conversation,
        target_id: session_id.to_owned(),
        display_name: if scheduled.display_name.trim().is_empty() {
            session_id.to_owned()
        } else {
            scheduled.display_name
        },
        tag: snapshot
            .config
            .tag
            .expect("enabled canonical AutoWork config has a tag"),
        max_requirements: snapshot.config.max_requirements,
        config_revision: snapshot.revision,
    })
}

#[async_trait::async_trait]
impl AutoWorkBindingLookup for RequirementService {
    async fn list_enabled_autowork_bindings(
        &self,
        owner_id: &str,
    ) -> Result<Vec<PersistedAutoWorkBinding>, AppError> {
        self.collect_enabled_autowork_bindings(owner_id).await
    }
}

/// Terminal-delete hook (spec §9.B) for the typed Terminal owner domain.
/// Wired in `nomifun-app` via
/// `TerminalService::with_delete_hook`.
#[async_trait::async_trait]
impl nomifun_common::OnTerminalDelete for RequirementService {
    async fn on_terminal_deleted(&self, _user_id: &str, terminal_id: &str) {
        if let Err(e) = self
            .clear_owner_for_session(&terminal_id, AutoWorkTargetKind::Terminal)
            .await
        {
            warn!(
                terminal_id,
                error = %nomifun_common::ErrorChain(&e),
                "failed to reconcile Requirement owner on terminal delete"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::CreateRequirementRequest;
    use nomifun_db::{SqliteRequirementRepository, init_database_memory};
    use nomifun_realtime::UserEventSink;

    #[derive(Default)]
    struct NoopBroadcaster;

    impl UserEventSink for NoopBroadcaster {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    async fn service_with_session() -> (RequirementService, String) {
        let db = init_database_memory().await.unwrap();
        let owner_id = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
        let session_id = ConversationId::new().into_string();
        sqlx::query(
            "INSERT INTO agent_sessions (\
                agent_session_id, owner_ref_json, state, title, archived, pinned, \
                agent_binding_json, next_seq, created_at\
             ) VALUES (?1, json_object('principal_kind','user','principal_id',?2), \
                       'live', 'Requirement AgentSession', 0, 0, '{}', 1, 0)",
        )
        .bind(&session_id)
        .bind(&owner_id)
        .execute(db.pool())
        .await
        .unwrap();
        let repo: Arc<dyn IRequirementRepository> =
            Arc::new(SqliteRequirementRepository::new(db.pool().clone()));
        (
            RequirementService::new(
                repo,
                RequirementEventEmitter::new(
                    Arc::new(NoopBroadcaster),
                    Arc::from(owner_id.as_str()),
                ),
            ),
            session_id,
        )
    }

    async fn pending_requirement(service: &RequirementService, tag: &str) -> Requirement {
        service
            .create(CreateRequirementRequest {
                title: "Implement the exact requirement".to_owned(),
                content: "Use the canonical AgentExecution path.".to_owned(),
                tag: tag.to_owned(),
                order_key: None,
                status: None,
                created_by: None,
                attachments: Vec::new(),
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn execution_receipt_resolves_exact_claim_without_requirement_retry() {
        let (service, session_id) = service_with_session().await;
        let requirement = pending_requirement(&service, "execution").await;
        let claim = service
            .claim_next_for_runner(
                "execution",
                &session_id,
                AutoWorkTargetKind::Conversation,
                DEFAULT_LEASE_MS,
            )
            .await
            .unwrap()
            .unwrap();

        let resolved = service
            .resolve_claim_verdict_exact(
                &requirement.requirement_id,
                claim.claim_generation,
                &claim.claim_token,
                &session_id,
                AutoWorkTargetKind::Conversation,
                RequirementStatus::Done,
                Some("AgentExecution completed".to_owned()),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.status, RequirementStatus::Done);
        assert!(
            service
                .confirm_claim_verdict_exact(
                    &requirement.requirement_id,
                    claim.claim_generation,
                    &claim.claim_token,
                    &session_id,
                    AutoWorkTargetKind::Conversation,
                    RequirementStatus::Done,
                )
                .await
                .unwrap()
        );
        assert!(
            service
                .claim_next_for_runner(
                    "execution",
                    &session_id,
                    AutoWorkTargetKind::Conversation,
                    DEFAULT_LEASE_MS,
                )
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn execution_failure_is_terminal_until_explicit_user_action() {
        let (service, session_id) = service_with_session().await;
        let requirement = pending_requirement(&service, "failure").await;
        let claim = service
            .claim_next_for_runner(
                "failure",
                &session_id,
                AutoWorkTargetKind::Conversation,
                DEFAULT_LEASE_MS,
            )
            .await
            .unwrap()
            .unwrap();
        service
            .resolve_claim_verdict_exact(
                &requirement.requirement_id,
                claim.claim_generation,
                &claim.claim_token,
                &session_id,
                AutoWorkTargetKind::Conversation,
                RequirementStatus::Failed,
                Some("canonical execution failed".to_owned()),
            )
            .await
            .unwrap()
            .unwrap();

        let final_requirement = service.get(&requirement.requirement_id).await.unwrap();
        assert_eq!(final_requirement.status, RequirementStatus::Failed);
        assert_eq!(final_requirement.attempt_count, 1);
    }

    #[tokio::test]
    async fn tag_pause_projection_clears_reason_after_resume() {
        let (service, _) = service_with_session().await;
        assert_eq!(service.tag_pause_state("release").await.unwrap(), (false, None));

        service
            .repo()
            .pause_tag("release", "execution_failed", None, now_ms())
            .await
            .unwrap();
        assert_eq!(
            service.tag_pause_state("release").await.unwrap(),
            (true, Some("execution_failed".to_owned()))
        );

        service.resume_tag("release", &[]).await.unwrap();
        assert_eq!(service.tag_pause_state("release").await.unwrap(), (false, None));
    }
}
