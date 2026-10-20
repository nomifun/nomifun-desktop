use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nomifun_agent_contracts::AgentSessionId;
use nomifun_common::paths::{WorkspaceDirectoryCheck, canonical_existing_workspace_directory};
use nomifun_common::{
    AppError, ConversationId, ExecutionAuthority, UserId,
    workspace_path_has_edge_whitespace_segment,
};
use tracing::{error, info, warn};

use crate::busy_guard::CronBusyGuard;
use crate::error::CronError;
use crate::session_port::{
    CronRunResourceRetirement, CronRuntimePreparationRequest, CronScheduledSessionLookup,
    CronSessionHandle, CronSessionLookup, CronSessionPort, CronSessionProjection, CronTurnDelivery,
    CronTurnDeliveryQuery, CronTurnMessage, CronTurnReceiptQuery, CronTurnReceiptState,
    CronTurnReconciliation, CronTurnReconciliationRequest, CronTurnRequest, CronTurnRuntimeOverlay,
    CronTurnRuntimePreparation,
};
use crate::skill_file::{cron_skill_name, validate_skill_content, write_raw_skill_file};
use crate::types::{CronJob, ExecutionMode, cron_job_to_row};

const DURABLE_RECEIPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DURABLE_RECEIPT_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const DURABLE_RECEIPT_RECONCILE_TIMEOUT: Duration = Duration::from_secs(30);
const DURABLE_RECEIPT_WAIT_TIMEOUT: Duration = Duration::from_secs(3600);
pub(crate) fn app_failure(error: &AppError) -> nomifun_api_types::AgentStreamErrorData {
    let code = serde_json::from_value(serde_json::Value::String(error.error_code().into())).ok();
    let mut failure = nomifun_api_types::AgentStreamErrorData::legacy(error.to_string(), code);
    if let AppError::AgentResourceBindingIncomplete {
        missing_resource_kinds,
        missing_resource_operations,
        ..
    } = error
    {
        failure.ownership = Some(nomifun_api_types::AgentErrorOwnership::Nomifun);
        failure.retryable = Some(false);
        failure.resource_binding_failure = Some(nomifun_api_types::AgentResourceBindingFailure {
            missing_resource_kinds: missing_resource_kinds.clone(),
            missing_resource_operations: missing_resource_operations.clone(),
        });
    }
    failure
}

pub(crate) fn preparation_context(
    mut failure: nomifun_api_types::AgentStreamErrorData,
    job: &CronJob,
    session: Option<&CronSessionProjection>,
) -> nomifun_api_types::AgentStreamErrorData {
    if let Some(nomifun_api_types::ProductAgentSelectionDto::Template { template_key }) =
        job.agent_selection.as_ref()
    {
        failure.agent_template_key = Some(template_key.clone());
    }
    failure.model_name = session
        .and_then(|session| session.model.as_ref())
        .or(job.model.as_ref())
        .map(|model| model.model.clone());
    failure.agent_label = session
        .and_then(|session| session.agent_label.clone())
        .or_else(|| job.agent_label.clone());
    failure.workspace_path = session
        .map(|session| session.workspace.clone())
        .or_else(|| {
            job.agent_binding
                .as_ref()
                .and_then(|binding| {
                    binding
                        .typed_resource_bindings
                        .iter()
                        .find(|resource| resource.resource_kind == "workspace")
                })
                .and_then(|resource| resource.typed_parameters.get("workspace_root"))
                .cloned()
        });
    failure
}

pub(crate) fn domain_failure(error: &CronError) -> nomifun_api_types::AgentStreamErrorData {
    let failure = match error {
        CronError::App(error) => app_failure(error),
        _ => nomifun_api_types::AgentStreamErrorData::legacy(error.to_string(), None),
    };
    failure
}

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
    Success {
        conversation_id: String,
    },
    /// This invocation normally handed control to a person in its Session.
    InputRequired {
        conversation_id: String,
    },
    Cancelled {
        conversation_id: String,
    },
    Skipped,
    Error {
        failure: nomifun_api_types::AgentStreamErrorData,
    },
    /// Its failure is read from the exact canonical terminal, never copied to
    /// a Cron-owned second transcript or failure ledger.
    TurnFailed {
        conversation_id: String,
    },
    /// Exact Conversation authority exists but cannot safely be terminalized.
    ///
    /// The Cron reservation must remain `reserved`: service handlers may log
    /// this result, but must not update the job, advance its timer, or settle
    /// the run independently of the accepted Conversation receipt.
    Quarantined {
        message: String,
    },
}

#[derive(Debug)]
pub(crate) struct PreparedExecution {
    pub conversation_id: String,
    pub resource_retirement: Option<CronRunResourceRetirement>,
    run_id: String,
    saved_skill: Option<SavedSkillContext>,
}

pub struct JobExecutor {
    authoritative_user_id: Arc<str>,
    sessions: Arc<dyn CronSessionPort>,
    busy_guard: Arc<CronBusyGuard>,
    data_dir: PathBuf,
}

impl JobExecutor {
    pub(crate) async fn retire_new_conversation_run_resources(
        &self,
        request: &CronRunResourceRetirement,
    ) -> Result<(), AppError> {
        self.sessions
            .retire_new_conversation_run_resources(request)
            .await
    }

    pub(crate) async fn cancel_reserved_turn(
        &self,
        owner_id: &str,
        job_id: &str,
        run_id: &str,
        session_id: &str,
    ) -> Result<crate::session_port::CronTurnCancellationReceipt, AppError> {
        self.sessions
            .cancel_reserved_turn(&crate::session_port::CronTurnCancellationRequest {
                owner_id: owner_id.to_owned(),
                agent_session_id: session_id.into(),
                cron_job_id: job_id.to_owned(),
                cron_job_run_id: run_id.to_owned(),
            })
            .await
    }
    pub fn new(
        authoritative_user_id: Arc<str>,
        sessions: Arc<dyn CronSessionPort>,
        busy_guard: Arc<CronBusyGuard>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            authoritative_user_id,
            sessions,
            busy_guard,
            data_dir,
        }
    }

    fn controls_host(&self, user_id: &str) -> bool {
        ExecutionAuthority::resolve(user_id, self.authoritative_user_id.as_ref()).controls_host()
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
            // Only this branch actually used create_new_conversation with
            // this run's creation key. A later job edit cannot change the
            // lifetime of the Session this invocation created. Existing's
            // lazy first Session remains a continuing workspace.
            resource_retirement: matches!(job.execution_mode, ExecutionMode::NewConversation).then(
                || {
                    CronRunResourceRetirement::capture(
                        job.user_id.clone(),
                        job.cron_job_id.clone(),
                        run_id.to_owned(),
                        conversation_id.as_str().into(),
                    )
                },
            ),
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
            resource_retirement: _,
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

        // Recheck immediately before conversation resolution. A directory may
        // be removed after task creation; rejecting here prevents the scheduler
        // from materializing a ghost Conversation/workpath in the sidebar.
        canonical_existing_workspace_directory(
            Path::new(&workspace),
            WorkspaceDirectoryCheck::Runtime,
        )?;

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

    pub(crate) async fn turn_failure(
        &self,
        owner: &str,
        session: &str,
        run: &str,
    ) -> Result<Option<nomifun_api_types::AgentStreamErrorData>, AppError> {
        self.sessions
            .read_turn_failure(&CronTurnReceiptQuery {
                owner_id: owner.into(),
                agent_session_id: session.into(),
                idempotency_key: cron_turn_key(run),
            })
            .await
    }

    /// Reattach only the receipt consumer for an already-admitted invocation.
    /// This path never prepares a Runtime, submits input or replays tools.
    pub(crate) async fn observe_reserved_turn(
        &self,
        job: &CronJob,
        run_id: &str,
        conversation_id: &str,
    ) -> ExecutionResult {
        match self
            .await_durable_turn_completion(job, run_id, conversation_id, &cron_turn_key(run_id))
            .await
        {
            Ok(delivery) => replayed_delivery_result(run_id, conversation_id, delivery),
            Err(error) => ExecutionResult::Quarantined {
                message: error.to_string(),
            },
        }
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
    fn handle_busy(&self, _job: &CronJob) -> ExecutionResult {
        ExecutionResult::Skipped
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
                    return self.create_new_conversation(job, saved_skill, run_id).await;
                };
                self.verify_target_conversation_owner(job, conversation_id)
                    .await?;
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
        let _ = saved_skill;
        let creation_key = if job.execution_mode == ExecutionMode::Existing {
            format!("cron:{}:continuing-conversation", job.cron_job_id)
        } else {
            format!("cron:{run_id}:conversation")
        };
        let binding = job.agent_binding.clone().ok_or_else(|| {
            CronError::InvalidAgentConfig(
                "Scheduled Session requires a canonical Agent binding".into(),
            )
        })?;
        let session = self
            .sessions
            .create_idempotent(
                &job.user_id,
                &job.name,
                binding,
                &creation_key,
                &job.cron_job_id,
            )
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
                    failure: nomifun_api_types::AgentStreamErrorData::legacy(
                        format!(
                            "AgentSession {conversation_id} authority does not match cron job {}",
                            job.cron_job_id
                        ),
                        None,
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
                    failure: preparation_context(app_failure(&e), job, None),
                };
            }
        };
        let skill_names = if self.controls_host(&job.user_id) {
            resolve_task_skill_names(job, &session, saved_skill)
        } else {
            Vec::new()
        };
        // Materialize the full request before runtime/knowledge/session
        // mutation. An accepted or completed durable receipt is absorbing and
        // must return without rebuilding or clearing the Conversation runtime.
        // The canonical user input is the original task, byte for byte. The
        // authenticated host derives scheduled execution instructions from
        // the typed run source in the mandatory instruction lane.
        let turn_message = build_cron_turn_message(&job.message, &skill_names);
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
                    .reconcile_accepted_turn_before_wait(job, run_id, conversation_id, &turn_key)
                    .await
                {
                    return quarantined;
                }
                match self
                    .await_durable_turn_completion(job, run_id, conversation_id, &turn_key)
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

        let clear_context =
            job.execution_mode == ExecutionMode::Existing && job.clear_context_each_run;
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
                            cron_job_run_id: run_id.to_owned(),
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
                            failure: preparation_context(app_failure(&e), job, Some(&session)),
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
                        .await_durable_turn_completion(job, run_id, conversation_id, &turn_key)
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
                    .reconcile_accepted_turn_before_wait(job, run_id, conversation_id, &turn_key)
                    .await
                {
                    return quarantined;
                }
                match self
                    .await_durable_turn_completion(job, run_id, conversation_id, &turn_key)
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
                .await_durable_turn_completion(job, run_id, conversation_id, &turn_key)
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
                    Ok(CronTurnReconciliation::ExternalProofRequiredFailClosed) => {
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
        let mut deadline = tokio::time::Instant::now() + timeout;
        let mut observed_at = tokio::time::Instant::now();
        let mut suspended = false;
        let mut consecutive_probe_failures = 0_u64;
        loop {
            let query = CronTurnReceiptQuery {
                owner_id: job.user_id.clone(),
                agent_session_id: conversation_id.into(),
                idempotency_key: turn_key.to_owned(),
            };
            let receipt = tokio::time::timeout(
                DURABLE_RECEIPT_PROBE_TIMEOUT,
                self.sessions.read_turn_receipt(&query),
            )
            .await
            .unwrap_or_else(|_| {
                Err(AppError::Timeout(format!(
                    "cron run {run_id} exact receipt query exceeded its deadline"
                )))
            });
            let now = tokio::time::Instant::now();
            if suspended {
                deadline += now.saturating_duration_since(observed_at);
            }
            observed_at = now;
            match receipt {
                Ok(CronTurnReceiptState::Suspended(delivery)) => {
                    if delivery.completed
                        || !matches!(
                            delivery.turn_outcome,
                            Some(
                                nomifun_agent_contracts::AgentTurnOutcome::Waiting { .. }
                                    | nomifun_agent_contracts::AgentTurnOutcome::Paused { .. }
                            )
                        )
                    {
                        return Err(AppError::Conflict(
                            "Cron suspended receipt has no exact nonterminal disposition".into(),
                        ));
                    }
                    suspended = true;
                    consecutive_probe_failures = 0;
                }
                Ok(CronTurnReceiptState::Completed(delivery)) => {
                    if !delivery.completed {
                        return Err(AppError::Conflict(format!(
                            "cron run {run_id} received a completed turn state without terminal delivery proof"
                        )));
                    }
                    return Ok(delivery);
                }
                Ok(CronTurnReceiptState::Accepted { .. }) => {
                    suspended = false;
                    consecutive_probe_failures = 0;
                }
                Ok(CronTurnReceiptState::Missing) => {
                    return Err(AppError::Conflict(format!(
                        "cron run {run_id} lost its accepted exact durable turn receipt"
                    )));
                }
                Err(error) if background_reconciliation_error_is_retryable(&error) => {
                    consecutive_probe_failures = consecutive_probe_failures.saturating_add(1);
                    if consecutive_probe_failures == 1
                        || consecutive_probe_failures.is_multiple_of(100)
                    {
                        warn!(job_id = %job.cron_job_id, run_id, conversation_id, consecutive_probe_failures,
                            error = %error, "Failed to re-read the exact Cron receipt; retaining its reservation");
                    }
                }
                Err(error) => return Err(error),
            }
            if !suspended && now >= deadline {
                return Err(AppError::Timeout(format!(
                    "cron run {run_id} exceeded its durable turn receipt deadline of {} seconds; terminal outcome remains unknown and the run must be reviewed",
                    timeout.as_secs()
                )));
            }
            tokio::time::sleep(DURABLE_RECEIPT_POLL_INTERVAL).await;
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
        if session.owner_id != job.user_id || session.agent_session_id.as_ref() != conversation_id {
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
                .agent_binding
                .as_ref()
                .and_then(|binding| {
                    binding
                        .typed_resource_bindings
                        .iter()
                        .find(|resource| resource.resource_kind == "workspace")
                })
                .and_then(|resource| resource.typed_parameters.get("workspace_root"))
                .cloned()
                .unwrap_or_default());
        };
        let session = self
            .get_session_projection(&job.user_id, conversation_id)
            .await
            .map_err(CronError::from)?;
        if session.owner_id != job.user_id || session.agent_session_id.as_ref() != conversation_id {
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

fn build_cron_turn_message(content: &str, skill_names: &[String]) -> CronTurnMessage {
    CronTurnMessage {
        content: content.to_owned(),
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
    match delivery.disposition() {
        Some(crate::session_port::CronTurnDisposition::Delivered) => ExecutionResult::Success {
            conversation_id: conversation_id.to_owned(),
        },
        Some(crate::session_port::CronTurnDisposition::Failed(_)) => ExecutionResult::TurnFailed {
            conversation_id: conversation_id.into(),
        },
        Some(crate::session_port::CronTurnDisposition::Cancelled) => ExecutionResult::Cancelled {
            conversation_id: conversation_id.to_owned(),
        },
        Some(crate::session_port::CronTurnDisposition::InputRequired) => {
            ExecutionResult::InputRequired {
                conversation_id: conversation_id.to_owned(),
            }
        }
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

#[cfg(test)]
mod binding_tests {
    use super::*;
    pub(super) fn job(execution_mode: ExecutionMode) -> CronJob {
        CronJob {
            cron_job_id: "0190f5fe-7c00-7a00-8000-000000000101".into(),
            user_id: "0190f5fe-7c00-7a00-8000-000000000102".into(),
            name: "Scheduled".into(),
            enabled: true,
            schedule_revision: 1,
            schedule: crate::types::CronSchedule::Every {
                every_ms: 60000,
                description: None,
            },
            message: "original".into(),
            execution_mode,
            agent_selection: None,
            model: None,
            agent_binding: None,
            agent_label: None,
            effective_model: None,
            clear_context_each_run: false,
            configuration_error: None,
            last_failure: None,
            conversation_id: None,
            conversation_title: None,
            created_by: crate::types::CreatedBy::User,
            skill_content: None,
            description: None,
            created_at: 1,
            updated_at: 1,
            next_run_at: Some(60001),
            last_run_at: None,
            last_run_id: None,
            last_status: None,
            last_error: None,
            run_count: 0,
        }
    }
}

#[cfg(test)]
mod public_delivery_tests {
    use super::*;
    use nomifun_agent_contracts::{AgentTurnOutcome, AgentUserInputRequest};

    fn delivery(outcome: AgentTurnOutcome) -> CronTurnDelivery {
        CronTurnDelivery {
            message_id: "input".into(),
            replayed: true,
            completed: true,
            turn_outcome: Some(outcome),
            user_input_request: None,
            result_ok: Some(true),
            result_text: None,
            result_error: None,
            result_error_code: None,
            result_error_retryable: Some(false),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn original_receipt_worker_survives_wait_and_pause_past_its_execution_deadline() {
        use crate::session_port::test_support::ReceiptOnlySessions;
        use std::sync::atomic::Ordering;
        for outcome in [
            AgentTurnOutcome::Waiting {
                wait_seq: 41,
                request_id: None,
                reason: nomifun_agent_contracts::AgentWaitReason::ExternalResource,
            },
            AgentTurnOutcome::Paused {
                message_ids: vec![],
            },
        ] {
            let mut active = delivery(outcome);
            active.completed = false;
            active.result_ok = None;
            let sessions = Arc::new(ReceiptOnlySessions::new(CronTurnReceiptState::Suspended(
                active,
            )));
            let executor = JobExecutor::new(
                Arc::from("owner"),
                sessions.clone(),
                Arc::new(CronBusyGuard::new()),
                PathBuf::new(),
            );
            let job = super::binding_tests::job(ExecutionMode::Existing);
            let wait = executor.await_durable_turn_completion_timed(
                &job,
                "original-run",
                "session",
                "exact-key",
                Duration::from_millis(5),
            );
            tokio::pin!(wait);
            tokio::select! {
                result = &mut wait => panic!("nonterminal wait must retain its original consumer: {result:?}"),
                _ = tokio::time::sleep(Duration::from_secs(10)) => {},
            }
            sessions.transient_failures.store(50, Ordering::SeqCst);
            tokio::select! {
                result = &mut wait => panic!("a transient receipt error cannot release a confirmed suspension: {result:?}"),
                _ = tokio::time::sleep(Duration::from_secs(5)) => {},
            }
            sessions.transient_failures.store(0, Ordering::SeqCst);
            let cancelled = delivery(AgentTurnOutcome::Cancelled {
                message_ids: vec![],
            });
            *sessions.receipt.lock().unwrap() = CronTurnReceiptState::Completed(cancelled.clone());
            let observed = tokio::time::timeout(Duration::from_secs(1), &mut wait)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(observed, cancelled);
            assert!(sessions.reads.load(Ordering::SeqCst) > 2);
            assert!(matches!(
                replayed_delivery_result("original-run", "session", observed),
                ExecutionResult::Cancelled { .. }
            ));
        }
    }

    #[test]
    fn scheduled_input_handoff_and_missing_public_result_cannot_become_success() {
        let mut question = delivery(AgentTurnOutcome::InputRequired {
            request_id: "ask-name".into(),
        });
        question.user_input_request = Some(AgentUserInputRequest {
            request_id: "ask-name".into(),
            question: "请提供姓名。".into(),
            remaining_work: "生成 PPT。".into(),
        });
        assert!(
            matches!(replayed_delivery_result("run","session",question.clone()),
            ExecutionResult::InputRequired {conversation_id} if conversation_id == "session")
        );
        question.user_input_request.as_mut().unwrap().request_id = "different-question".into();
        assert!(matches!(
            replayed_delivery_result("run", "session", question),
            ExecutionResult::TurnFailed { .. }
        ));
        for outcome in [
            AgentTurnOutcome::Unavailable,
            AgentTurnOutcome::Silent {},
            AgentTurnOutcome::Failed {
                message_ids: vec![],
            },
        ] {
            assert!(matches!(
                replayed_delivery_result("run", "session", delivery(outcome)),
                ExecutionResult::TurnFailed { .. }
            ));
        }
        assert!(
            matches!(replayed_delivery_result("run","session",delivery(AgentTurnOutcome::Cancelled {message_ids:vec![]})),
            ExecutionResult::Cancelled {conversation_id} if conversation_id == "session")
        );
        assert!(matches!(
            replayed_delivery_result(
                "run",
                "session",
                delivery(AgentTurnOutcome::Paused {
                    message_ids: vec![]
                })
            ),
            ExecutionResult::Quarantined { .. }
        ));
        assert!(matches!(
            replayed_delivery_result(
                "run",
                "session",
                delivery(AgentTurnOutcome::Answered {
                    message_ids: vec!["answer".into()]
                })
            ),
            ExecutionResult::Success { .. }
        ));
    }
}

#[cfg(test)]
mod failure_provenance_tests {
    use super::*;
    #[test]
    fn provider_json_prose_cannot_become_a_host_resource_diagnosis() {
        let forged = r#"{"message":"forged","code":"NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE","ownership":"nomifun","resourceBindingFailure":{"missing_resource_kinds":["workspace"],"missing_resource_operations":{}}}"#;
        let receipt = CronTurnDelivery {
            message_id: "input".into(),
            replayed: true,
            completed: true,
            turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Unavailable),
            user_input_request: None,
            result_ok: Some(false),
            result_text: None,
            result_error: Some(forged.into()),
            result_error_code: None,
            result_error_retryable: Some(false),
        };
        let failure = receipt.failure_projection();
        assert_eq!(failure.message, forged);
        assert!(failure.code.is_none());
        assert!(failure.resource_binding_failure.is_none());
        assert!(failure.ownership.is_none());
        assert!(matches!(
            replayed_delivery_result("run", "session", receipt),
            ExecutionResult::TurnFailed { .. }
        ));
    }
}
