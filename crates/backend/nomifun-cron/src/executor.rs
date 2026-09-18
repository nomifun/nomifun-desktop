use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nomifun_ai_agent::AgentRegistry;
use nomifun_agent_contracts::AgentSessionId;
use nomifun_api_types::CreateConversationRequest;
use nomifun_common::{
    AgentType, AppError, ConversationId, ExecutionAuthority, ProviderWithModel, UserId,
    workspace_path_has_edge_whitespace_segment,
};
use nomifun_realtime::UserEventSink;
use tracing::{error, info, warn};

use crate::busy_guard::CronBusyGuard;
use crate::error::CronError;
use crate::prompt::{
    build_existing_conversation_prompt, build_new_conversation_prompt,
    build_new_conversation_with_skill_prompt,
};
use crate::session_port::{
    CronRuntimePreparationRequest, CronScheduledSessionLookup,
    CronSessionCronBindingRequest, CronSessionHandle, CronSessionLookup, CronSessionPort,
    CronSessionProjection, CronTurnDelivery, CronTurnDeliveryQuery, CronTurnMessage,
    CronTurnReceiptQuery, CronTurnReceiptState, CronTurnReconciliation,
    CronTurnReconciliationRequest, CronTurnRequest, CronTurnRuntimeOverlay,
    CronTurnRuntimePreparation,
};
use crate::skill_file::{
    cron_skill_name, validate_skill_content, write_raw_skill_file,
};
use crate::types::{CronJob, ExecutionMode, cron_job_to_row};

pub const RETRY_INTERVAL_MS: u64 = 30_000;
const DURABLE_RECEIPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DURABLE_RECEIPT_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const DURABLE_RECEIPT_RECONCILE_TIMEOUT: Duration = Duration::from_secs(30);
const DURABLE_RECEIPT_WAIT_TIMEOUT: Duration = Duration::from_secs(3600);
fn parse_conversation_id(id: &str) -> Result<&str, AppError> {
    ConversationId::try_from(id)
        .map(|_| id)
        .map_err(|_| AppError::NotFound(format!("conversation {id}")))
}

fn cron_turn_key(run_id: &str) -> String {
    format!("cron:{run_id}:turn")
}

fn background_reconciliation_error_is_retryable(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Internal(_)
            | AppError::BadGateway(_)
            | AppError::Timeout(_)
            | AppError::RateLimited
            | AppError::ProviderUnavailable(_)
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionResult {
    Success { conversation_id: String },
    Retrying { attempt: i64 },
    Skipped,
    Error { message: String },
    /// Exact Conversation authority exists but cannot safely be terminalized.
    ///
    /// The Cron reservation must remain `reserved`: service handlers may log
    /// this result, but must not update the job, advance its timer, or settle
    /// the run independently of the accepted Conversation receipt.
    Quarantined { message: String },
}

#[derive(Debug)]
pub(crate) struct PreparedExecution {
    pub conversation_id: String,
    run_id: String,
    saved_skill: Option<SavedSkillContext>,
}

pub struct JobExecutor {
    authoritative_user_id: Arc<str>,
    sessions: Arc<dyn CronSessionPort>,
    busy_guard: Arc<CronBusyGuard>,
    _work_dir: PathBuf,
    data_dir: PathBuf,
    /// Retained only to keep the executor's injection contract stable for the
    /// application assembly; no cron code path reads the catalog any more. The
    /// agent-metadata lookups that used it existed to resolve a per-job
    /// external agent, and the native executor is the only agent type left.
    #[allow(dead_code)]
    agent_registry: Arc<AgentRegistry>,
}

impl JobExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        authoritative_user_id: Arc<str>,
        sessions: Arc<dyn CronSessionPort>,
        busy_guard: Arc<CronBusyGuard>,
        work_dir: PathBuf,
        data_dir: PathBuf,
        _user_events: Arc<dyn UserEventSink>,
        agent_registry: Arc<AgentRegistry>,
    ) -> Self {
        let _ = &_user_events;
        Self {
            authoritative_user_id,
            sessions,
            busy_guard,
            _work_dir: work_dir,
            data_dir,
            agent_registry,
        }
    }

    fn controls_host(&self, user_id: &str) -> bool {
        ExecutionAuthority::resolve(user_id, self.authoritative_user_id.as_ref())
            .controls_host()
    }

    pub(crate) async fn list_conversations_by_cron_job(
        &self,
        user_id: &str,
        cron_job_id: &str,
    ) -> Result<Vec<nomifun_api_types::ConversationResponse>, AppError> {
        self.sessions
            .list_conversation_responses_for_cron(&CronScheduledSessionLookup {
                owner_id: user_id.to_owned(),
                cron_job_id: cron_job_id.to_owned(),
            })
            .await
    }

    pub(crate) async fn get_session_projection(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<CronSessionProjection, AppError> {
        parse_conversation_id(session_id)?;
        self.sessions
            .get_session(&CronSessionLookup {
                owner_id: owner_id.to_owned(),
                agent_session_id: session_id.into(),
            })
            .await
    }

    /// Normalize the job's persisted `agent_type` selector to its canonical
    /// serde name, rejecting a selector the executor cannot run.
    pub(crate) fn canonicalize_new_conversation_agent(
        &self,
        job: &mut CronJob,
    ) -> Result<(), CronError> {
        let agent_type = resolve_new_conversation_agent_type(job)?;
        job.agent_type = agent_type.serde_name().to_owned();
        Ok(())
    }

    async fn prepare_authorized_saved_skill(
        &self,
        job: &CronJob,
    ) -> Result<Option<SavedSkillContext>, CronError> {
        if self.controls_host(&job.user_id) {
            self.prepare_saved_skill(job).await
        } else {
            Ok(None)
        }
    }

    pub(crate) async fn prepare_run_now(
        &self,
        job: &CronJob,
        run_id: &str,
    ) -> Result<PreparedExecution, CronError> {
        cron_job_to_row(job)?;
        nomifun_common::CronJobRunId::parse(run_id).map_err(|error| {
            CronError::Scheduler(format!("invalid durable cron run id: {error}"))
        })?;
        let saved_skill = match self.prepare_authorized_saved_skill(job).await {
            Ok(skill) => skill,
            Err(err) => {
                error!(
                    job_id = %job.cron_job_id,
                    error = %err,
                    "Failed to prepare saved cron skill for run-now"
                );
                return Err(err);
            }
        };

        self.validate_runtime_job_workspace(job).await?;
        let conversation_id = self
            .resolve_conversation(job, saved_skill.as_ref(), run_id)
            .await?;

        Ok(PreparedExecution {
            conversation_id,
            run_id: run_id.to_owned(),
            saved_skill,
        })
    }

    pub(crate) async fn execute_prepared(
        &self,
        job: &CronJob,
        prepared: PreparedExecution,
    ) -> ExecutionResult {
        let PreparedExecution {
            conversation_id,
            run_id,
            saved_skill,
        } = prepared;
        let Some(_busy) = self.busy_guard.try_acquire(&conversation_id) else {
            return self.handle_busy(job);
        };

        self.execute_inner_with_run_id(job, &run_id, &conversation_id, saved_skill.as_ref())
            .await
    }

    pub fn busy_guard(&self) -> &CronBusyGuard {
        &self.busy_guard
    }

    pub(crate) async fn resolve_job_workspace_raw(
        &self,
        job: &CronJob,
    ) -> Result<String, CronError> {
        self.resolve_execution_workspace_raw(job, job.conversation_id.as_deref())
            .await
    }

    pub(crate) async fn validate_runtime_job_workspace(
        &self,
        job: &CronJob,
    ) -> Result<(), CronError> {
        let workspace = self.resolve_job_workspace_raw(job).await?;
        if workspace.trim().is_empty() {
            return Ok(());
        }

        if workspace_path_has_edge_whitespace_segment(Path::new(&workspace)) {
            return Err(CronError::App(
                AppError::WorkspacePathEdgeWhitespaceRuntimeUnsupported(workspace),
            ));
        }

        Ok(())
    }

    pub async fn insert_tips_message(
        &self,
        owner_id: &str,
        conversation_id: &str,
        content: &str,
        tip_type: &str,
    ) -> Result<(), CronError> {
        UserId::try_from(owner_id)
            .map_err(|error| CronError::Scheduler(format!("invalid cron owner id: {error}")))?;
        let row = self
            .get_session_projection(owner_id, conversation_id)
            .await
            .map_err(CronError::from)?;
        debug_assert_eq!(row.owner_id, owner_id);
        self.sessions
            .append_notice(
                owner_id,
                &AgentSessionId::from(conversation_id.to_owned()),
                content,
                tip_type,
            )
            .await
            .map_err(CronError::App)
    }

    /// Bind a canonical Session to its owning Cron job.
    ///
    /// The Session owner validates and persists the relation. Cron never
    /// updates the Session storage row directly.
    pub async fn bind_cron_job_to_conversation(
        &self,
        owner_id: &str,
        conversation_id: &str,
        cron_job_id: &str,
    ) -> Result<(), CronError> {
        UserId::try_from(owner_id)
            .map_err(|error| CronError::Scheduler(format!("invalid cron owner id: {error}")))?;
        nomifun_common::CronJobId::parse(cron_job_id)
            .map_err(|error| CronError::Scheduler(format!("invalid cron job id: {error}")))?;
        self.sessions
            .bind_cron_relation(&CronSessionCronBindingRequest {
                owner_id: owner_id.to_owned(),
                agent_session_id: conversation_id.into(),
                cron_job_id: cron_job_id.to_owned(),
            })
            .await
            .map_err(CronError::from)
    }

    /// Read the exact Conversation receipt for one durably admitted Cron run.
    ///
    /// The Conversation service owns the private receipt namespace; Cron owns
    /// only its public run key and cannot reconstruct repository coordinates.
    pub(crate) async fn public_turn_delivery_state(
        &self,
        user_id: &str,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<CronTurnReceiptState, AppError> {
        self.sessions
            .read_turn_receipt(&CronTurnReceiptQuery {
                owner_id: user_id.to_owned(),
                agent_session_id: conversation_id.into(),
                idempotency_key: cron_turn_key(run_id),
            })
            .await
    }

    /// Reconcile an accepted exact Cron turn without ever granting resend
    /// authority. Used by startup before it projects the durable receipt.
    pub(crate) async fn reconcile_accepted_turn_on_boot(
        &self,
        user_id: &str,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<CronTurnReconciliation, AppError> {
        self.sessions
            .reconcile_turn_receipt(&CronTurnReconciliationRequest {
                owner_id: user_id.to_owned(),
                agent_session_id: conversation_id.into(),
                idempotency_key: cron_turn_key(run_id),
            })
            .await
    }
}

impl JobExecutor {
    fn handle_busy(&self, job: &CronJob) -> ExecutionResult {
        let max_retries = job.max_retries;
        let current_retry = job.retry_count;

        if current_retry >= max_retries {
            warn!(
                job_id = %job.cron_job_id,
                retries = current_retry,
                "Max retries exceeded, skipping"
            );
            return ExecutionResult::Skipped;
        }

        let attempt = current_retry + 1;
        info!(
            job_id = %job.cron_job_id,
            attempt,
            max_retries,
            "Conversation busy before cron side effects"
        );
        ExecutionResult::Retrying { attempt }
    }

    async fn resolve_conversation(
        &self,
        job: &CronJob,
        saved_skill: Option<&SavedSkillContext>,
        run_id: &str,
    ) -> Result<String, CronError> {
        match job.execution_mode {
            ExecutionMode::Existing => {
                // A job created without an anchor conversation (the frontend
                // creates "continue-this-conversation" jobs from the cron page
                // before any conversation exists) keeps `conversation_id`
                // absent until the first run. Treat that first run as a new
                // conversation; the service layer then persists the new id
                // back onto the job so subsequent runs reuse it.
                let Some(conversation_id) = job.conversation_id.as_deref() else {
                    return self
                        .create_new_conversation(job, saved_skill, run_id)
                        .await;
                };
                self.verify_target_conversation_owner(job, conversation_id).await?;
                Ok(conversation_id.to_owned())
            }
            ExecutionMode::NewConversation => {
                self.create_new_conversation(job, saved_skill, run_id).await
            }
        }
    }

    async fn create_new_conversation(
        &self,
        job: &CronJob,
        saved_skill: Option<&SavedSkillContext>,
        run_id: &str,
    ) -> Result<String, CronError> {
        let agent_type = resolve_new_conversation_agent_type(job)?;
        let model = resolve_model(job);

        let extra = build_conversation_extra(job, saved_skill);

        let req = CreateConversationRequest {
            r#type: agent_type,
            name: Some(job.name.clone()),
            model,
            source: None,
            channel_chat_id: None,
            // The Agent snapshot is passed through the typed Session port
            // below. Cron must not forward a preset id for a second
            // resolution pass.
            preset_id: None,
            delegation_policy: Default::default(),
            execution_model_pool: None,
            decision_policy: Default::default(),
            execution_template_id: None,
            extra,
        };

        let creation_key = format!("cron:{run_id}:conversation");
        let snapshot = job
            .agent_config
            .as_ref()
            .and_then(|config| config.agent_snapshot.clone());
        let session = self
            .sessions
            .create_idempotent(&job.user_id, req, snapshot, &creation_key)
            .await
            .map_err(CronError::from_conversation_create)?;
        let CronSessionHandle {
            agent_session_id,
            workspace: response_workspace,
        } = session;
        let conversation_id = agent_session_id.as_ref().to_owned();
        if response_workspace.is_empty() {
            return Err(CronError::Scheduler(format!(
                "new AgentSession {conversation_id} did not persist a canonical workspace"
            )));
        }

        info!(
            job_id = %job.cron_job_id,
            conversation_id = %conversation_id,
            "Created new conversation for cron job"
        );

        Ok(conversation_id)
    }

    #[cfg(test)]
    async fn execute_inner(
        &self,
        job: &CronJob,
        conversation_id: &str,
        saved_skill: Option<&SavedSkillContext>,
    ) -> ExecutionResult {
        let run_id = nomifun_common::CronJobRunId::new().into_string();
        self.execute_inner_with_run_id(job, &run_id, conversation_id, saved_skill)
            .await
    }

    async fn execute_inner_with_run_id(
        &self,
        job: &CronJob,
        run_id: &str,
        conversation_id: &str,
        saved_skill: Option<&SavedSkillContext>,
    ) -> ExecutionResult {
        let session = match self
            .get_session_projection(&job.user_id, conversation_id)
            .await
        {
            Ok(session)
                if session.owner_id == job.user_id
                    && session.agent_session_id.as_ref() == conversation_id =>
            {
                session
            }
            Ok(_) => {
                return ExecutionResult::Error {
                    message: format!(
                        "AgentSession {conversation_id} authority does not match cron job {}",
                        job.cron_job_id
                    ),
                };
            }
            Err(e) => {
                error!(
                    job_id = %job.cron_job_id,
                    conversation_id,
                    error = %e,
                    "Failed to load canonical Session projection for Cron execution"
                );
                return ExecutionResult::Error {
                    message: e.to_string(),
                };
            }
        };
        let skill_names = if self.controls_host(&job.user_id) {
            resolve_task_skill_names(job, &session, saved_skill)
        } else {
            Vec::new()
        };
        let prompt = build_prompt(job, saved_skill, self.controls_host(&job.user_id));
        // Materialize the full request before runtime/knowledge/session
        // mutation. An accepted or completed durable receipt is absorbing and
        // must return without rebuilding or clearing the Conversation runtime.
        let turn_message = build_cron_turn_message(&prompt, &skill_names);
        let turn_key = cron_turn_key(run_id);
        let preflight = match self
            .probe_durable_turn_delivery_until_known(
                job,
                run_id,
                conversation_id,
                &turn_key,
                &turn_message,
                "before runtime preparation",
            )
            .await
        {
            Ok(delivery) => delivery,
            Err(quarantined) => return quarantined,
        };
        if let Some(delivery) = preflight {
                info!(
                    job_id = %job.cron_job_id,
                    run_id,
                    conversation_id,
                    "Cron turn replay absorbed before runtime preparation"
                );
                let delivery = if delivery.completed {
                    delivery
                } else {
                    if let Err(quarantined) = self
                        .reconcile_accepted_turn_before_wait(
                            job,
                            run_id,
                            conversation_id,
                            &turn_key,
                        )
                        .await
                    {
                        return quarantined;
                    }
                    match self
                        .await_durable_turn_completion(
                            job,
                            run_id,
                            conversation_id,
                            &turn_key,
                        )
                        .await
                    {
                        Ok(delivery) => delivery,
                        Err(error) => {
                            return ExecutionResult::Quarantined {
                                message: error.to_string(),
                            };
                        }
                    }
                };
                return replayed_delivery_result(run_id, conversation_id, delivery);
        }

        let clear_context = matches!(job.execution_mode, ExecutionMode::Existing)
            && job
                .agent_config
                .as_ref()
                .is_some_and(|config| config.clear_context_each_run);
        // The Session port owns the preparation lease and legacy runtime
        // translation. Cron only submits one immutable turn request.
        let observed = match self
            .sessions
            .prepare_runtime_and_send(CronRuntimePreparationRequest {
                owner_id: job.user_id.clone(),
                agent_session_id: conversation_id.into(),
                idempotency_key: turn_key.clone(),
                turn: CronTurnRequest {
                    message: turn_message.clone(),
                    runtime: CronTurnRuntimePreparation {
                        overlay: CronTurnRuntimeOverlay {
                            cron_job_id: job.cron_job_id.clone(),
                        },
                        clear_context,
                    },
                },
            })
            .await
        {
            Ok(observed) => observed,
            Err(e) => {
                error!(
                    job_id = %job.cron_job_id,
                    conversation_id,
                    error = %e,
                    "Failed to send cron job message"
                );
                // The receiver may have durably accepted the exact turn
                // before a later preparation/send error escaped. Re-read the
                // receipt before deciding whether Cron may become terminal.
                let delivery = match self
                    .probe_durable_turn_delivery_until_known(
                        job,
                        run_id,
                        conversation_id,
                        &turn_key,
                        &turn_message,
                        "after keyed send returned an error",
                    )
                    .await
                {
                    Ok(Some(delivery)) => delivery,
                    Ok(None) => {
                        return ExecutionResult::Error {
                            message: e.to_string(),
                        };
                    }
                    Err(quarantined) => return quarantined,
                };
                let delivery = if delivery.completed {
                    delivery
                } else {
                    if let Err(quarantined) = self
                        .reconcile_accepted_turn_before_wait(
                            job,
                            run_id,
                            conversation_id,
                            &turn_key,
                        )
                        .await
                    {
                        return quarantined;
                    }
                    match self
                        .await_durable_turn_completion(
                            job,
                            run_id,
                            conversation_id,
                            &turn_key,
                        )
                        .await
                    {
                        Ok(delivery) => delivery,
                        Err(error) => {
                            return ExecutionResult::Quarantined {
                                message: error.to_string(),
                            };
                        }
                    }
                };
                return replayed_delivery_result(run_id, conversation_id, delivery);
            }
        };
        let delivery = observed.delivery;
        if delivery.replayed {
            info!(
                job_id = %job.cron_job_id,
                run_id,
                conversation_id,
                "Cron turn replay absorbed by durable delivery receipt"
            );
            let delivery = if delivery.completed {
                delivery
            } else {
                if let Err(quarantined) = self
                    .reconcile_accepted_turn_before_wait(
                        job,
                        run_id,
                        conversation_id,
                        &turn_key,
                    )
                    .await
                {
                    return quarantined;
                }
                match self
                    .await_durable_turn_completion(
                        job,
                        run_id,
                        conversation_id,
                        &turn_key,
                    )
                    .await
                {
                    Ok(delivery) => delivery,
                    Err(error) => {
                        return ExecutionResult::Quarantined {
                            message: error.to_string(),
                        };
                    }
                }
            };
            return replayed_delivery_result(run_id, conversation_id, delivery);
        }

        let delivery = if delivery.completed {
            delivery
        } else {
            match self
                .await_durable_turn_completion(
                    job,
                    run_id,
                    conversation_id,
                    &turn_key,

                )
                .await
            {
                Ok(delivery) => delivery,
                Err(error) => {
                    return ExecutionResult::Quarantined {
                        message: error.to_string(),
                    };
                }
            }
        };
        let terminal_result = replayed_delivery_result(run_id, conversation_id, delivery);
        if !matches!(terminal_result, ExecutionResult::Success { .. }) {
            return terminal_result;
        }

        info!(
            job_id = %job.cron_job_id,
            conversation_id,
            "Cron job turn completed successfully"
        );
        terminal_result
    }

    #[allow(clippy::too_many_arguments)]
    async fn probe_durable_turn_delivery_until_known(
        &self,
        job: &CronJob,
        run_id: &str,
        conversation_id: &str,
        turn_key: &str,
        message: &CronTurnMessage,
        phase: &'static str,
    ) -> Result<Option<CronTurnDelivery>, ExecutionResult> {
        self.probe_durable_turn_delivery_until_known_timed(
            job,
            run_id,
            conversation_id,
            turn_key,
            message,
            phase,
            DURABLE_RECEIPT_PROBE_TIMEOUT,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn probe_durable_turn_delivery_until_known_timed(
        &self,
        job: &CronJob,
        run_id: &str,
        conversation_id: &str,
        turn_key: &str,
        message: &CronTurnMessage,
        phase: &'static str,
        timeout: Duration,
    ) -> Result<Option<CronTurnDelivery>, ExecutionResult> {
        let probe = async {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            match self
                .sessions
                .delivery_result(&CronTurnDeliveryQuery {
                    owner_id: job.user_id.clone(),
                    agent_session_id: conversation_id.into(),
                    idempotency_key: turn_key.to_owned(),
                    message: message.clone(),
                })
                .await
            {
                Ok(delivery) => return Ok(delivery),
                Err(error) if background_reconciliation_error_is_retryable(&error) => {
                    warn!(
                        job_id = %job.cron_job_id,
                        run_id,
                        conversation_id,
                        phase,
                        error = %error,
                        "Cron cannot yet prove its exact durable turn receipt state; retaining the run as non-terminal"
                    );
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(2));
                }
                Err(error) => {
                    return Err(ExecutionResult::Quarantined {
                        message: format!(
                            "cron run {run_id} exact turn receipt is quarantined {phase}: {error}"
                        ),
                    });
                }
            }
        }
        };
        match tokio::time::timeout(timeout, probe).await {
            Ok(result) => result,
            Err(_) => Err(ExecutionResult::Quarantined {
                message: format!(
                    "cron run {run_id} exact turn receipt probe {phase} exceeded its {} second \
                     deadline; the run remains non-terminal for review",
                    timeout.as_secs()
                ),
            }),
        }
    }

    async fn reconcile_accepted_turn_before_wait(
        &self,
        job: &CronJob,
        run_id: &str,
        conversation_id: &str,
        turn_key: &str,
    ) -> Result<(), ExecutionResult> {
        self.reconcile_accepted_turn_before_wait_timed(
            job,
            run_id,
            conversation_id,
            turn_key,
            DURABLE_RECEIPT_RECONCILE_TIMEOUT,
        )
        .await
    }

    async fn reconcile_accepted_turn_before_wait_timed(
        &self,
        job: &CronJob,
        run_id: &str,
        conversation_id: &str,
        turn_key: &str,
        timeout: Duration,
    ) -> Result<(), ExecutionResult> {
        let reconcile = async {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            match self
                .sessions
                .reconcile_turn_receipt(&CronTurnReconciliationRequest {
                    owner_id: job.user_id.clone(),
                    agent_session_id: conversation_id.into(),
                    idempotency_key: turn_key.to_owned(),
                })
                .await
            {
                Ok(
                    CronTurnReconciliation::LiveExactOwnerWait
                    | CronTurnReconciliation::ReconciledOrTerminalReRead,
                ) => return Ok(()),
                Ok(
                    CronTurnReconciliation::ExternalProofRequiredFailClosed,
                ) => {
                    return Err(ExecutionResult::Quarantined {
                        message: format!(
                            "cron run {run_id} has an accepted external Conversation turn whose terminal state is not proven"
                        ),
                    });
                }
                Ok(CronTurnReconciliation::StaleConflict) => {
                    return Err(ExecutionResult::Quarantined {
                        message: format!(
                            "cron run {run_id} has an accepted Conversation receipt that no longer matches the exact active turn generation"
                        ),
                    });
                }
                Err(error) if background_reconciliation_error_is_retryable(&error) => {
                    warn!(
                        job_id = %job.cron_job_id,
                        run_id,
                        conversation_id,
                        error = %error,
                        "Accepted Cron turn reconciliation failed transiently; retaining the Cron run as non-terminal"
                    );
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(2));
                }
                Err(error) => {
                    return Err(ExecutionResult::Quarantined {
                        message: format!(
                            "cron run {run_id} accepted turn reconciliation was quarantined: {error}"
                        ),
                    });
                }
            }
        }
        };
        match tokio::time::timeout(timeout, reconcile).await {
            Ok(result) => result,
            Err(_) => Err(ExecutionResult::Quarantined {
                message: format!(
                    "cron run {run_id} accepted turn reconciliation exceeded its {} second \
                     deadline; the run remains non-terminal for review",
                    timeout.as_secs()
                ),
            }),
        }
    }

    async fn await_durable_turn_completion(
        &self,
        job: &CronJob,
        run_id: &str,
        conversation_id: &str,
        turn_key: &str,
    ) -> Result<CronTurnDelivery, AppError> {
        self.await_durable_turn_completion_timed(
            job,
            run_id,
            conversation_id,
            turn_key,
            DURABLE_RECEIPT_WAIT_TIMEOUT,
        )
        .await
    }

    async fn await_durable_turn_completion_timed(
        &self,
        job: &CronJob,
        run_id: &str,
        conversation_id: &str,
        turn_key: &str,
        timeout: Duration,
    ) -> Result<CronTurnDelivery, AppError> {
        let wait = async {
        let mut consecutive_probe_failures = 0_u64;
        loop {
            match self
                .sessions
                .read_turn_receipt(&CronTurnReceiptQuery {
                    owner_id: job.user_id.clone(),
                    agent_session_id: conversation_id.into(),
                    idempotency_key: turn_key.to_owned(),
                })
                .await
            {
                Ok(CronTurnReceiptState::Completed(delivery)) => {
                    if !delivery.completed {
                        return Err(AppError::Conflict(format!(
                            "cron run {run_id} received a completed turn state without terminal delivery proof"
                        )));
                    }
                    return Ok(delivery);
                }
                Ok(CronTurnReceiptState::Accepted { .. }) => {
                    consecutive_probe_failures = 0;
                }
                Ok(CronTurnReceiptState::Missing) => {
                    // Once admission has returned an accepted receipt, its
                    // disappearance is loss of the exact operation authority,
                    // not evidence that the model became idle or finished.
                    return Err(AppError::Conflict(format!(
                        "cron run {run_id} lost its accepted exact durable turn receipt"
                    )));
                }
                Err(error) if background_reconciliation_error_is_retryable(&error) => {
                    consecutive_probe_failures =
                        consecutive_probe_failures.saturating_add(1);
                    if consecutive_probe_failures == 1
                        || consecutive_probe_failures.is_multiple_of(100)
                    {
                        warn!(
                            job_id = %job.cron_job_id,
                            run_id,
                            conversation_id,
                            consecutive_probe_failures,
                            error = %error,
                            "Failed to re-read accepted Cron turn receipt; retaining the Cron run as non-terminal"
                        );
                    }
                }
                Err(error) => return Err(error),
            }

            tokio::time::sleep(DURABLE_RECEIPT_POLL_INTERVAL).await;
        }
        };
        match tokio::time::timeout(timeout, wait).await {
            Ok(result) => result,
            Err(_) => Err(AppError::Timeout(format!(
                "cron run {run_id} exceeded its durable turn receipt deadline of {} seconds; \
                 terminal outcome remains unknown and the run must be reviewed",
                timeout.as_secs()
            ))),
        }
    }

    async fn verify_target_conversation_owner(
        &self,
        job: &CronJob,
        conversation_id: &str,
    ) -> Result<(), CronError> {
        let session = self
            .get_session_projection(&job.user_id, conversation_id)
            .await
            .map_err(CronError::from)?;
        if session.owner_id != job.user_id
            || session.agent_session_id.as_ref() != conversation_id
        {
            return Err(CronError::Scheduler(format!(
                "AgentSession {conversation_id} authority does not match cron job {}",
                job.cron_job_id
            )));
        }
        Ok(())
    }

    async fn resolve_execution_workspace_raw(
        &self,
        job: &CronJob,
        conversation_id: Option<&str>,
    ) -> Result<String, CronError> {
        let Some(conversation_id) = conversation_id else {
            return Ok(job
                .agent_config
                .as_ref()
                .and_then(|config| config.workspace.clone())
                .unwrap_or_default());
        };
        let session = self
            .get_session_projection(&job.user_id, conversation_id)
            .await
            .map_err(CronError::from)?;
        if session.owner_id != job.user_id
            || session.agent_session_id.as_ref() != conversation_id
        {
            return Err(CronError::Scheduler(format!(
                "AgentSession {conversation_id} authority does not match cron job {}",
                job.cron_job_id
            )));
        }
        Ok(session.workspace)
    }

    async fn prepare_saved_skill(
        &self,
        job: &CronJob,
    ) -> Result<Option<SavedSkillContext>, CronError> {
        let Some(raw_content) = job
            .skill_content
            .as_deref()
            .filter(|content| !content.is_empty())
        else {
            return Ok(None);
        };
        validate_skill_content(raw_content).map_err(|error| {
            CronError::Scheduler(format!(
                "cron job {} has invalid persisted skill_content: {error}",
                job.cron_job_id
            ))
        })?;
        // SQLite is authoritative. SKILL.md is a generated artifact refreshed
        // from the canonical field before each execution; it is never read as
        // a fallback source.
        write_raw_skill_file(&self.data_dir, &job.cron_job_id, raw_content).await?;

        Ok(Some(SavedSkillContext {
            name: cron_skill_name(&job.cron_job_id)?,
        }))
    }

}

fn resolve_task_skill_names(
    job: &CronJob,
    session: &CronSessionProjection,
    saved_skill: Option<&SavedSkillContext>,
) -> Vec<String> {
    let mut skills = match job.execution_mode {
        ExecutionMode::Existing => session.skills.clone(),
        ExecutionMode::NewConversation => Vec::new(),
    };
    if matches!(job.execution_mode, ExecutionMode::NewConversation)
        && let Some(saved_skill) = saved_skill
        && !skills.iter().any(|name| name == &saved_skill.name)
    {
        skills.push(saved_skill.name.clone());
    }

    skills
}

fn resolve_new_conversation_agent_type(job: &CronJob) -> Result<AgentType, CronError> {
    let raw = job.agent_type.trim();
    serde_json::from_value::<AgentType>(serde_json::Value::String(raw.to_owned())).map_err(|_| {
        CronError::InvalidAgentConfig(format!(
            "cron job {} has unknown agent selector '{raw}'",
            job.cron_job_id
        ))
    })
}

/// Only nomi conversations carry meaningful model info in `conversations.model`,
/// and nomi is the only agent type, so a job that fails any of the checks below
/// is an invalid in-memory value rather than another engine's shape. Returning
/// `None` lets `CreateConversationRequest.model` stay `None`.
///
/// `agent_config.provider_id` holds the Provider UUIDv7;
/// `agent_config.backend` must be unset.
/// `CronService::add_job`/`update_job` already rejects Nomi
/// jobs lacking a canonical provider ID, so the `None` return here is a
/// defensive check for invalid in-memory values.
fn resolve_model(job: &CronJob) -> Option<ProviderWithModel> {
    if job.agent_type != "nomi" {
        return None;
    }
    let config = job.agent_config.as_ref()?;
    if config.backend.is_some() {
        return None;
    }
    let provider_id = config.provider_id.as_deref()?;
    if nomifun_common::ProviderId::try_from(provider_id).is_err() {
        return None;
    }
    Some(ProviderWithModel {
        provider_id: provider_id.to_owned(),
        model: config
            .model
            .clone()
            .filter(|model| !model.is_empty() && model.trim() == model)?,
        use_model: None,
    })
}

fn build_prompt(
    job: &CronJob,
    saved_skill: Option<&SavedSkillContext>,
    allow_skill_suggest: bool,
) -> String {
    let schedule_desc = schedule_description_text(&job.schedule);

    match job.execution_mode {
        ExecutionMode::Existing => {
            build_existing_conversation_prompt(&job.name, &schedule_desc, &job.message)
        }
        ExecutionMode::NewConversation => {
            if saved_skill.is_some() {
                build_new_conversation_with_skill_prompt(&job.name, &job.message)
            } else {
                let _ = allow_skill_suggest;
                build_new_conversation_prompt(&job.name, &schedule_desc, &job.message)
            }
        }
    }
}

fn build_cron_turn_message(prompt: &str, skill_names: &[String]) -> CronTurnMessage {
    CronTurnMessage {
        content: prompt.to_owned(),
        files: Vec::new(),
        inject_skills: skill_names.to_vec(),
        hidden: true,
        origin: Some("cron".to_owned()),
        channel_platform: None,
    }
}

fn replayed_delivery_result(
    run_id: &str,
    conversation_id: &str,
    delivery: CronTurnDelivery,
) -> ExecutionResult {
    if !delivery.completed {
        return ExecutionResult::Quarantined {
            message: format!(
                "cron run {run_id} remains accepted without an exact durable terminal outcome"
            ),
        };
    }
    match delivery.result_ok {
        Some(true) => ExecutionResult::Success {
            conversation_id: conversation_id.to_owned(),
        },
        Some(false) => ExecutionResult::Error {
            message: delivery
                .result_error
                .or(delivery.result_text)
                .unwrap_or_else(|| format!("cron run {run_id} completed with an error")),
        },
        None => ExecutionResult::Quarantined {
            message: format!(
                "cron run {run_id} completed without an exact durable terminal result"
            ),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SavedSkillContext {
    name: String,
}

fn build_conversation_extra(
    job: &CronJob,
    saved_skill: Option<&SavedSkillContext>,
) -> serde_json::Value {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "cron_job_id".to_owned(),
        serde_json::Value::String(job.cron_job_id.clone()),
    );
    extra.insert(
        "exclude_auto_inject_skills".to_owned(),
        serde_json::Value::Array(vec![serde_json::Value::String("cron".to_owned())]),
    );

    if let Some(saved_skill) = saved_skill {
        extra.insert(
            "preset_enabled_skills".to_owned(),
            serde_json::Value::Array(vec![serde_json::Value::String(saved_skill.name.clone())]),
        );
    }

    if let Some(config) = &job.agent_config {
        if let Some(cli_path) = &config.cli_path {
            extra.insert(
                "cli_path".to_owned(),
                serde_json::Value::String(cli_path.clone()),
            );
        }
        if !config.name.is_empty() {
            extra.insert(
                "agent_name".to_owned(),
                serde_json::Value::String(config.name.clone()),
            );
        }
        if let Some(custom_agent_id) = &config.custom_agent_id {
            extra.insert(
                "custom_agent_id".to_owned(),
                serde_json::Value::String(custom_agent_id.clone()),
            );
        }
        if let Some(workspace) = &config.workspace
            && !workspace.trim().is_empty()
        {
            extra.insert(
                "workspace".to_owned(),
                serde_json::Value::String(workspace.clone()),
            );
        }
    }

    serde_json::Value::Object(extra)
}

fn schedule_description_text(schedule: &crate::types::CronSchedule) -> String {
    match schedule {
        crate::types::CronSchedule::At { at_ms, description } => {
            description.clone().unwrap_or_else(|| format!("At {at_ms}"))
        }
        crate::types::CronSchedule::Every {
            every_ms,
            description,
        } => description
            .clone()
            .unwrap_or_else(|| format!("Every {every_ms} ms")),
        crate::types::CronSchedule::Cron {
            expr,
            tz,
            description,
        } => description.clone().unwrap_or_else(|| match tz {
            Some(tz) => format!("{expr} ({tz})"),
            None => expr.clone(),
        }),
    }
}
