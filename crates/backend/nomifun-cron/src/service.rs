use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use nomifun_api_types::{
    CreateCronJobRequest, CronJobResponse, CronJobRunResponse, CronScheduleDto, HasSkillResponse,
    ListCronJobsQuery, RunNowResponse, SaveCronSkillRequest, UpdateCronJobRequest,
};
use nomifun_common::{
    AgentType, AppError, generate_prefixed_id, now_ms, workspace_path_has_edge_whitespace_segment,
};
use nomifun_db::{CRON_RUN_HISTORY_LIMIT, CronJobRunRow, ICronRepository, UpdateCronJobParams};
use tracing::{error, info, warn};

use crate::events::CronEventEmitter;

use crate::error::CronError;
use crate::executor::{ExecutionResult, JobExecutor, RETRY_INTERVAL_MS};
use crate::scheduler::{CronScheduler, compute_next_run, validate_schedule};
use crate::skill_file::{
    delete_skill_file, has_skill_file, write_raw_skill_file, write_skill_file,
};
use crate::types::{
    CreatedBy, CronAgentConfig, CronJob, CronSchedule, ExecutionMode, TargetKind,
    cron_job_from_row, cron_job_to_response, cron_job_to_row, schedule_from_dto,
};

const PLACEHOLDER_PATTERNS: &[&str] = &[
    "todo:",
    "todo ",
    "fill in",
    "placeholder",
    "replace this",
    "your ",
    "insert ",
    "add your",
    "write your",
    "put your",
];

#[derive(Clone)]
pub struct CronService {
    repo: Arc<dyn ICronRepository>,
    scheduler: Arc<CronScheduler>,
    executor: Arc<JobExecutor>,
    emitter: CronEventEmitter,
    data_dir: PathBuf,
}

impl CronService {
    pub fn new(
        repo: Arc<dyn ICronRepository>,
        scheduler: Arc<CronScheduler>,
        executor: Arc<JobExecutor>,
        emitter: CronEventEmitter,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            repo,
            scheduler,
            executor,
            emitter,
            data_dir,
        }
    }

    // -----------------------------------------------------------------------
    // CRUD
    // -----------------------------------------------------------------------

    pub async fn add_job(&self, req: CreateCronJobRequest) -> Result<CronJob, CronError> {
        let schedule = schedule_from_dto(&req.schedule);
        validate_schedule(&schedule)?;

        let execution_mode = parse_execution_mode(req.execution_mode.as_deref())?;
        // The DTO carries the conversation key as `i64`; the domain keeps the
        // empty-string "unbound" convention. A non-positive key (`0`, used by
        // lazy-bind jobs and some legacy rows) maps to unbound.
        let conversation_id = if req.conversation_id > 0 {
            req.conversation_id.to_string()
        } else {
            String::new()
        };

        let target_kind = TargetKind::from_str(&req.target_kind)?;
        // The model source depends on execution mode: an Existing job bound to
        // a conversation takes its model from that conversation at run time, so
        // `agent_config` may legitimately be absent (the desktop "指定会话"
        // flow omits it). Only NewConversation / lazy-bind jobs require
        // `agent_config.backend`.
        self.validate_nomi_job_model(
            &req.agent_type,
            execution_mode,
            &conversation_id,
            req.agent_config.as_ref(),
        )
        .await?;

        let created_by = CreatedBy::from_str(&req.created_by)?;
        let message = req.message.or(req.prompt).unwrap_or_default();

        let agent_config = req.agent_config.map(|c| CronAgentConfig {
            backend: c.backend,
            name: c.name,
            cli_path: c.cli_path,
            is_preset: c.is_preset,
            custom_agent_id: c.custom_agent_id,
            preset_agent_type: c.preset_agent_type,
            mode: c.mode,
            model_id: c.model_id,
            config_options: c.config_options,
            workspace: c.workspace,
            clear_context_each_run: c.clear_context_each_run,
        });

        let now = now_ms();
        let next_run_at = compute_next_run(&schedule, now);

        let job = CronJob {
            id: generate_prefixed_id("cron"),
            name: req.name,
            enabled: true,
            schedule,
            message,
            execution_mode,
            agent_config,
            conversation_id,
            conversation_title: req.conversation_title,
            agent_type: req.agent_type,
            created_by,
            skill_content: None,
            description: req.description,
            created_at: now,
            updated_at: now,
            next_run_at,
            last_run_at: None,
            last_status: None,
            last_error: None,
            run_count: 0,
            retry_count: 0,
            max_retries: 3,
            target_kind,
        };

        self.validate_job_workspace(&job).await?;

        let row = cron_job_to_row(&job)?;
        self.repo.insert(&row).await?;
        self.bind_existing_conversation_if_needed(&job).await;
        self.scheduler.schedule_job(&job);
        self.emitter.emit_job_created(&cron_job_to_response(&job));

        info!(job_id = %job.id, name = %job.name, "Cron job created");
        Ok(job)
    }

    pub async fn update_job(
        &self,
        job_id: &str,
        req: UpdateCronJobRequest,
    ) -> Result<CronJob, CronError> {
        let existing_row = self
            .repo
            .get_by_id(job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_owned()))?;
        let mut job = cron_job_from_row(existing_row)?;

        if let Some(name) = &req.name {
            job.name = name.clone();
        }
        if let Some(description) = &req.description {
            job.description = Some(description.clone());
        }
        if let Some(enabled) = req.enabled {
            job.enabled = enabled;
        }
        if let Some(schedule_dto) = &req.schedule {
            let schedule = schedule_from_dto_with_existing_timezone(schedule_dto, &job.schedule);
            validate_schedule(&schedule)?;
            job.schedule = schedule;
        }
        if let Some(message) = &req.message {
            job.message = message.clone();
        }
        if let Some(mode_str) = &req.execution_mode {
            job.execution_mode = parse_execution_mode(Some(mode_str))?;
        }
        if let Some(config_dto) = &req.agent_config {
            // Execution-mode-aware: an Existing job bound to a conversation is
            // validated against that conversation's model, not agent_config.
            self.validate_nomi_job_model(
                &job.agent_type,
                job.execution_mode,
                &job.conversation_id,
                Some(config_dto),
            )
            .await?;
            job.agent_config = Some(CronAgentConfig {
                backend: config_dto.backend.clone(),
                name: config_dto.name.clone(),
                cli_path: config_dto.cli_path.clone(),
                is_preset: config_dto.is_preset,
                custom_agent_id: config_dto.custom_agent_id.clone(),
                preset_agent_type: config_dto.preset_agent_type.clone(),
                mode: config_dto.mode.clone(),
                model_id: config_dto.model_id.clone(),
                config_options: config_dto.config_options.clone(),
                workspace: config_dto.workspace.clone(),
                clear_context_each_run: config_dto.clear_context_each_run,
            });
        }
        if let Some(title) = &req.conversation_title {
            job.conversation_title = Some(title.clone());
        }
        if let Some(max_retries) = req.max_retries {
            job.max_retries = max_retries;
        }
        if let Some(kind_str) = &req.target_kind {
            job.target_kind = TargetKind::from_str(kind_str)?;
        }

        if req.schedule.is_some() || req.enabled.is_some() {
            job.next_run_at = compute_next_run(&job.schedule, now_ms());
        }

        job.updated_at = now_ms();
        self.validate_job_workspace(&job).await?;

        let params = build_update_params(&job, &req);
        self.repo.update(job_id, &params).await?;

        self.bind_existing_conversation_if_needed(&job).await;
        self.scheduler.reschedule_job(&job);
        self.emitter.emit_job_updated(&cron_job_to_response(&job));

        info!(job_id = %job.id, "Cron job updated");
        Ok(job)
    }

    /// Validate that a nomi agent job has a usable model source before it is
    /// created or updated.
    ///
    /// The model source depends on the execution mode (see [`nomi_model_check`]):
    ///
    /// * An [`ExecutionMode::Existing`] job bound to a real conversation takes
    ///   its model from that conversation row at run time (`executor`'s
    ///   `execute_inner` → `provider_model_from_conversation_row`), *not* from
    ///   `agent_config.backend`. The desktop "指定会话" flow deliberately omits
    ///   `agent_config` (passing it would clobber the conversation's own
    ///   workspace), so demanding `agent_config.backend` here wrongly rejected
    ///   every nomi specified-conversation job. Validate the bound conversation
    ///   actually carries a model instead — only then is the "no model
    ///   configured" message accurate.
    /// * An [`ExecutionMode::NewConversation`] job — or an `Existing` job with
    ///   no bound conversation yet, whose first run lazily creates one — uses
    ///   `agent_config.backend` as the model source (`executor::resolve_model`),
    ///   so the original static check applies.
    async fn validate_nomi_job_model(
        &self,
        agent_type: &str,
        execution_mode: ExecutionMode,
        conversation_id: &str,
        agent_config: Option<&nomifun_api_types::CronAgentConfigDto>,
    ) -> Result<(), CronError> {
        match nomi_model_check(agent_type, execution_mode, conversation_id) {
            NomiModelCheck::Skip => Ok(()),
            NomiModelCheck::AgentConfig => validate_nomi_agent_config(agent_type, agent_config),
            NomiModelCheck::BoundConversation => {
                match self.executor.get_conversation_row(conversation_id).await {
                    Ok(Some(row)) => {
                        let model = nomifun_conversation::task_options::provider_model_from_conversation_row(&row);
                        if model.provider_id.trim().is_empty() {
                            return Err(CronError::InvalidAgentConfig(
                                "the bound nomi conversation has no model configured; \
                             open the conversation and choose a model first, then create the job"
                                    .into(),
                            ));
                        }
                        Ok(())
                    }
                    // Row missing/unreadable at creation time: don't block here. The
                    // executor verifies the conversation exists and resolves the
                    // model on the first tick, surfacing a precise error then; a
                    // lazy-bind job also legitimately has no row yet.
                    Ok(None) => Ok(()),
                    Err(err) => {
                        warn!(
                            conversation_id,
                            error = %err,
                            "Could not load conversation to validate nomi cron model; deferring to run time"
                        );
                        Ok(())
                    }
                }
            }
        }
    }

    pub async fn remove_job(&self, job_id: &str) -> Result<(), CronError> {
        self.scheduler.cancel_job(job_id);
        if let Err(err) = delete_skill_file(&self.data_dir, job_id).await {
            warn!(job_id, error = %err, "Failed to delete cron skill file during job removal");
        }
        self.repo.delete(job_id).await?;
        self.emitter.emit_job_removed(job_id);
        info!(job_id, "Cron job removed");
        Ok(())
    }

    pub async fn get_job(&self, job_id: &str) -> Result<CronJob, CronError> {
        let row = self
            .repo
            .get_by_id(job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_owned()))?;
        cron_job_from_row(row)
    }

    pub async fn list_jobs(&self, query: &ListCronJobsQuery) -> Result<Vec<CronJob>, CronError> {
        let rows = if let Some(conv_id) = &query.conversation_id {
            self.repo.list_by_conversation(*conv_id).await?
        } else {
            self.repo.list_all().await?
        };

        let mut jobs = Vec::new();
        for row in rows {
            let row_id = row.id.clone();
            let row_target_kind = row.target_kind.clone();
            match cron_job_from_row(row) {
                Ok(job) => jobs.push(job),
                Err(err) if is_removed_terminal_target(&row_target_kind, &err) => {
                    warn!(job_id = %row_id, "Skipping terminal cron job because terminal scheduling support was removed");
                }
                Err(err) => return Err(err),
            }
        }
        Ok(jobs)
    }

    pub async fn list_runs(&self, job_id: &str) -> Result<Vec<CronJobRunResponse>, CronError> {
        self.repo
            .get_by_id(job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_owned()))?;

        let rows = self
            .repo
            .list_runs_by_job(job_id, CRON_RUN_HISTORY_LIMIT)
            .await?;

        Ok(rows.into_iter().map(cron_run_to_response).collect())
    }

    // -----------------------------------------------------------------------
    // Init / Tick / Resume / RunNow
    // -----------------------------------------------------------------------

    pub async fn init(&self) {
        let rows = match self.repo.list_enabled().await {
            Ok(rows) => rows,
            Err(e) => {
                error!(error = %e, "Failed to load enabled cron jobs");
                return;
            }
        };

        let mut scheduled = 0u32;
        let mut orphans = 0u32;
        for row in rows {
            let row_id = row.id.clone();
            let row_target_kind = row.target_kind.clone();
            let job = match cron_job_from_row(row) {
                Ok(j) => j,
                Err(e) if is_removed_terminal_target(&row_target_kind, &e) => {
                    warn!(job_id = %row_id, "Skipping terminal cron job because terminal scheduling support was removed");
                    continue;
                }
                Err(e) => {
                    error!(job_id = %row_id, error = %e, "Failed to parse cron job row");
                    continue;
                }
            };

            if self.is_orphan(&job).await {
                warn!(
                    job_id = %job.id,
                    job_name = %job.name,
                    conversation_id = %job.conversation_id,
                    execution_mode = job.execution_mode.as_str(),
                    "Deleting orphan cron job whose bound conversation no longer exists"
                );
                if let Err(e) = self.repo.delete(&job.id).await {
                    error!(job_id = %job.id, error = %e, "Failed to delete orphan cron job");
                }
                orphans += 1;
                continue;
            }

            self.scheduler.schedule_job(&job);
            scheduled += 1;
        }

        info!(scheduled, orphans, "Cron service initialized");
    }

    pub async fn tick(&self, job_id: &str) {
        let row = match self.repo.get_by_id(job_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                warn!(job_id, "Tick: job not found, cancelling timer");
                self.scheduler.cancel_job(job_id);
                return;
            }
            Err(e) => {
                error!(job_id, error = %e, "Tick: failed to load job");
                return;
            }
        };

        let job = match cron_job_from_row(row) {
            Ok(j) => j,
            Err(e) => {
                error!(job_id, error = %e, "Tick: failed to parse job");
                self.scheduler.cancel_job(job_id);
                return;
            }
        };

        if !job.enabled {
            info!(job_id, "Tick: job disabled, skipping");
            return;
        }

        let result = self.executor.execute(&job).await;
        self.handle_execution_result(job, result).await;
    }

    pub async fn handle_system_resume(&self) {
        let rows = match self.repo.list_enabled().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "Resume: failed to load enabled jobs");
                return;
            }
        };

        let now = now_ms();

        for row in rows {
            let row_id = row.id.clone();
            let row_target_kind = row.target_kind.clone();
            let job = match cron_job_from_row(row) {
                Ok(j) => j,
                Err(e) if is_removed_terminal_target(&row_target_kind, &e) => {
                    warn!(job_id = %row_id, "Resume: skipping terminal cron job because terminal scheduling support was removed");
                    continue;
                }
                Err(e) => {
                    error!(job_id = %row_id, error = %e, "Resume: failed to parse job");
                    continue;
                }
            };

            if let Some(next_run) = job.next_run_at
                && next_run < now
            {
                info!(
                    job_id = %job.id,
                    conversation_id = %job.conversation_id,
                    "Resume: missed job detected, marking missed without auto-execution"
                );
                self.record_missed_execution(&job).await;
                self.insert_missed_job_tips(&job).await;
                self.reschedule_after_missed(&job).await;
                self.emitter.emit_job_executed(&job.id, "missed", None);
                continue;
            }

            self.scheduler.reschedule_job(&job);
        }

        info!("System resume: all cron timers rescheduled");
    }

    pub async fn run_now(&self, job_id: &str) -> Result<RunNowResponse, CronError> {
        let row = self
            .repo
            .get_by_id(job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_owned()))?;
        let job = cron_job_from_row(row)?;

        let prepared = self.executor.prepare_run_now(&job).await?;
        let conversation_id = prepared.conversation_id.clone();
        let service = self.clone();
        let job_id = job.id.clone();

        tokio::spawn(async move {
            let result = service.executor.execute_prepared(&job, prepared).await;
            service.handle_run_now_result(&job_id, result).await;
        });

        // `conversation_id` was produced from an integer conversation key by the
        // executor; parse it back for the i64 wire field (defensive `0` if a
        // legacy/empty id somehow reaches here).
        Ok(RunNowResponse {
            conversation_id: conversation_id.parse::<i64>().unwrap_or(0),
        })
    }

    // -----------------------------------------------------------------------
    // Skill management
    // -----------------------------------------------------------------------

    pub async fn save_skill(
        &self,
        job_id: &str,
        req: SaveCronSkillRequest,
    ) -> Result<(), CronError> {
        let row = self
            .repo
            .get_by_id(job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_owned()))?;

        validate_skill_body_content(&req.content)?;
        let job = cron_job_from_row(row)?;
        persist_skill_file(&self.data_dir, &job, &req.content).await?;

        let params = UpdateCronJobParams {
            skill_content: Some(Some(req.content)),
            ..Default::default()
        };
        self.repo.update(job_id, &params).await?;
        self.executor
            .mark_skill_suggest_artifacts_saved(job_id)
            .await?;

        info!(job_id, "Skill content saved");
        Ok(())
    }

    pub async fn has_skill(&self, job_id: &str) -> Result<HasSkillResponse, CronError> {
        let row = self
            .repo
            .get_by_id(job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_owned()))?;

        let has_skill = has_skill_file(&self.data_dir, job_id).await?
            || row
                .skill_content
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());

        Ok(HasSkillResponse { has_skill })
    }

    pub async fn delete_skill(&self, job_id: &str) -> Result<(), CronError> {
        self.repo
            .get_by_id(job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_owned()))?;

        delete_skill_file(&self.data_dir, job_id).await?;

        let params = UpdateCronJobParams {
            skill_content: Some(None),
            ..Default::default()
        };
        self.repo.update(job_id, &params).await?;

        info!(job_id, "Skill content deleted");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    pub fn to_response(job: &CronJob) -> CronJobResponse {
        cron_job_to_response(job)
    }

    async fn bind_existing_conversation_if_needed(&self, job: &CronJob) {
        if !matches!(job.execution_mode, ExecutionMode::Existing)
            || job.conversation_id.trim().is_empty()
        {
            return;
        }

        if let Err(err) = self
            .executor
            .bind_cron_job_to_conversation(&job.conversation_id, &job.id)
            .await
        {
            warn!(
                conversation_id = %job.conversation_id,
                job_id = %job.id,
                error = %err,
                "Failed to bind existing-conversation cron job to conversation"
            );
        }
    }

    async fn is_orphan(&self, job: &CronJob) -> bool {
        // NewConversation jobs never depend on an existing conversation —
        // every run materializes a fresh one. They must not be cleaned up
        // based on conversation state.
        if matches!(job.execution_mode, ExecutionMode::NewConversation) {
            return false;
        }

        // Existing-mode jobs can legitimately carry an empty conversation_id
        // until the first run performs lazy binding. Leave them alone.
        if job.conversation_id.trim().is_empty() {
            return false;
        }

        if self.executor.busy_guard().is_busy(&job.conversation_id) {
            return false;
        }

        // Only true orphan case: Existing + bound conversation_id, but that
        // conversation has been deleted.
        match self
            .executor
            .conversation_exists(&job.conversation_id)
            .await
        {
            Ok(exists) => !exists,
            Err(err) => {
                warn!(
                    job_id = %job.id,
                    conversation_id = %job.conversation_id,
                    error = %err,
                    "Failed to verify cron conversation during orphan cleanup"
                );
                false
            }
        }
    }

    async fn validate_job_workspace(&self, job: &CronJob) -> Result<(), CronError> {
        // The guard rejects pathological directory names (leading/trailing
        // whitespace — they break Win32 path round-tripping).
        let workspace = self.executor.resolve_job_workspace_raw(job).await?;
        if workspace.trim().is_empty() {
            return Ok(());
        }

        if workspace_path_has_edge_whitespace_segment(Path::new(&workspace)) {
            return Err(CronError::App(AppError::WorkspacePathEdgeWhitespace(
                workspace,
            )));
        }

        Ok(())
    }

    async fn handle_execution_result(&self, job: CronJob, result: ExecutionResult) {
        let job_id = &job.id;

        match result {
            ExecutionResult::Success { conversation_id } => {
                self.update_job_after_success(job_id, &conversation_id)
                    .await;
                self.record_execution_run(job_id, "ok").await;
                self.reschedule_after_execution(&job).await;
                self.emitter.emit_job_executed(job_id, "ok", None);
            }
            ExecutionResult::Retrying { attempt } => {
                let params = UpdateCronJobParams {
                    retry_count: Some(attempt),
                    ..Default::default()
                };
                if let Err(e) = self.repo.update(job_id, &params).await {
                    error!(job_id, error = %e, "Failed to update retry count");
                }
                self.schedule_retry(job_id, attempt);
            }
            ExecutionResult::Skipped => {
                let params = UpdateCronJobParams {
                    last_status: Some(Some("skipped".into())),
                    retry_count: Some(0),
                    ..Default::default()
                };
                if let Err(e) = self.repo.update(job_id, &params).await {
                    error!(job_id, error = %e, "Failed to update skipped status");
                }
                self.record_execution_run(job_id, "skipped").await;
                self.reschedule_after_execution(&job).await;
                self.emitter.emit_job_executed(job_id, "skipped", None);
            }
            ExecutionResult::Error { message } => {
                self.update_job_after_error(job_id, &message).await;
                self.record_execution_run(job_id, "error").await;
                self.reschedule_after_execution(&job).await;
                self.emitter
                    .emit_job_executed(job_id, "error", Some(&message));
            }
        }
    }

    async fn handle_run_now_result(&self, job_id: &str, result: ExecutionResult) {
        match result {
            ExecutionResult::Success { conversation_id } => {
                self.update_job_after_success(job_id, &conversation_id)
                    .await;
                self.record_execution_run(job_id, "ok").await;
                self.emitter.emit_job_executed(job_id, "ok", None);
            }
            ExecutionResult::Error { message } => {
                self.update_job_after_error(job_id, &message).await;
                self.record_execution_run(job_id, "error").await;
                self.emitter
                    .emit_job_executed(job_id, "error", Some(&message));
            }
            ExecutionResult::Retrying { attempt } => {
                let params = UpdateCronJobParams {
                    retry_count: Some(attempt),
                    ..Default::default()
                };
                if let Err(err) = self.repo.update(job_id, &params).await {
                    error!(
                        job_id,
                        error = %err,
                        "Failed to update run-now retry count"
                    );
                }
            }
            ExecutionResult::Skipped => {
                let params = UpdateCronJobParams {
                    last_status: Some(Some("skipped".into())),
                    retry_count: Some(0),
                    ..Default::default()
                };
                if let Err(err) = self.repo.update(job_id, &params).await {
                    error!(
                        job_id,
                        error = %err,
                        "Failed to update run-now skipped status"
                    );
                }
                self.record_execution_run(job_id, "skipped").await;
                self.emitter.emit_job_executed(job_id, "skipped", None);
            }
        }
    }

    async fn record_execution_run(&self, job_id: &str, status: &str) {
        let row = build_run_row(job_id, status);
        if let Err(err) = self.repo.insert_run_pruned(&row).await {
            error!(
                job_id,
                status,
                error = %err,
                "Failed to record cron execution history"
            );
        }
    }

    async fn update_job_after_success(&self, job_id: &str, conversation_id: &str) {
        let existing_row = match self.repo.get_by_id(job_id).await {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                error!(job_id, error = %e, "Failed to read job for run_count");
                return;
            }
        };
        let now = now_ms();
        // Persist the conversation_id back onto the job the first time an
        // "existing" job is materialized (lazy binding). Subsequent runs then
        // reuse the same conversation, matching the UX where the job is the
        // continuation anchor. The DB FK is `Option<i64>`: unbound == `None`.
        let new_conversation_key = conversation_id.trim().parse::<i64>().ok();
        let needs_conversation_bind =
            existing_row.conversation_id.is_none() && new_conversation_key.is_some();
        let params = UpdateCronJobParams {
            last_run_at: Some(Some(now)),
            last_status: Some(Some("ok".into())),
            last_error: Some(None),
            retry_count: Some(0),
            run_count: Some(existing_row.run_count + 1),
            // `Some(Some(id))` sets the FK; `None` leaves it unchanged. Only
            // bind on the first materialization of a lazy "existing" job.
            conversation_id: needs_conversation_bind
                .then_some(Some(new_conversation_key.unwrap_or_default())),
            ..Default::default()
        };
        if let Err(e) = self.repo.update(job_id, &params).await {
            error!(job_id, error = %e, "Failed to update job after success");
            return;
        }

        if needs_conversation_bind
            && let Err(e) = self
                .executor
                .bind_cron_job_to_conversation(conversation_id, job_id)
                .await
        {
            warn!(
                job_id,
                conversation_id,
                error = %e,
                "Failed to bind lazily-created conversation to cron job"
            );
        }
    }

    async fn update_job_after_error(&self, job_id: &str, message: &str) {
        let run_count = match self.repo.get_by_id(job_id).await {
            Ok(Some(r)) => r.run_count,
            Ok(None) => return,
            Err(e) => {
                error!(job_id, error = %e, "Failed to read job for run_count");
                return;
            }
        };
        let now = now_ms();
        let params = UpdateCronJobParams {
            last_run_at: Some(Some(now)),
            last_status: Some(Some("error".into())),
            last_error: Some(Some(message.to_owned())),
            retry_count: Some(0),
            run_count: Some(run_count + 1),
            ..Default::default()
        };
        if let Err(e) = self.repo.update(job_id, &params).await {
            error!(job_id, error = %e, "Failed to update job after error");
        }
    }

    async fn reschedule_after_execution(&self, job: &CronJob) {
        let is_at = matches!(job.schedule, CronSchedule::At { .. });
        if is_at {
            let params = UpdateCronJobParams {
                enabled: Some(false),
                next_run_at: Some(None),
                ..Default::default()
            };
            if let Err(e) = self.repo.update(&job.id, &params).await {
                error!(job_id = %job.id, error = %e, "Failed to disable at-type job");
            }
            self.scheduler.cancel_job(&job.id);

            let disabled = CronJob {
                enabled: false,
                next_run_at: None,
                ..job.clone()
            };
            self.emitter
                .emit_job_updated(&cron_job_to_response(&disabled));

            info!(job_id = %job.id, "At-type job executed, auto-disabled");
            return;
        }

        let now = now_ms();
        let next = compute_next_run(&job.schedule, now);
        let updated = CronJob {
            next_run_at: next,
            ..job.clone()
        };
        let params = UpdateCronJobParams {
            next_run_at: Some(next),
            ..Default::default()
        };
        if let Err(e) = self.repo.update(&job.id, &params).await {
            error!(job_id = %job.id, error = %e, "Failed to update next_run_at");
        }
        self.scheduler.reschedule_job(&updated);
    }

    async fn record_missed_execution(&self, job: &CronJob) {
        let params = UpdateCronJobParams {
            last_status: Some(Some("missed".into())),
            last_error: Some(None),
            retry_count: Some(0),
            ..Default::default()
        };
        if let Err(err) = self.repo.update(&job.id, &params).await {
            error!(
                job_id = %job.id,
                error = %err,
                "Failed to mark cron job as missed"
            );
        }
        self.record_execution_run(&job.id, "missed").await;
    }

    async fn insert_missed_job_tips(&self, job: &CronJob) {
        if job.conversation_id.trim().is_empty() {
            return;
        }

        let content = format!(
            "Scheduled task \"{}\" was missed while the system was unavailable. It was not run automatically.",
            job.name
        );

        match self
            .executor
            .insert_tips_message(&job.conversation_id, &content, "warning")
            .await
        {
            Ok(()) => {
                self.emitter
                    .emit_conversation_tips(&job.conversation_id, &content, "warning")
            }
            Err(err) => {
                warn!(
                    job_id = %job.id,
                    conversation_id = %job.conversation_id,
                    error = %err,
                    "Failed to persist missed-job tips message"
                );
            }
        }
    }

    async fn reschedule_after_missed(&self, job: &CronJob) {
        let is_at = matches!(job.schedule, CronSchedule::At { .. });
        if is_at {
            let params = UpdateCronJobParams {
                enabled: Some(false),
                next_run_at: Some(None),
                ..Default::default()
            };
            if let Err(err) = self.repo.update(&job.id, &params).await {
                error!(
                    job_id = %job.id,
                    error = %err,
                    "Failed to disable missed at-type job"
                );
            }
            self.scheduler.cancel_job(&job.id);
            return;
        }

        let next = compute_next_run(&job.schedule, now_ms());
        let params = UpdateCronJobParams {
            next_run_at: Some(next),
            ..Default::default()
        };
        if let Err(err) = self.repo.update(&job.id, &params).await {
            error!(
                job_id = %job.id,
                error = %err,
                "Failed to reschedule missed cron job"
            );
            return;
        }

        let updated = CronJob {
            next_run_at: next,
            ..job.clone()
        };
        self.scheduler.reschedule_job(&updated);
    }

    fn schedule_retry(&self, job_id: &str, _attempt: i64) {
        let next_run = now_ms() + RETRY_INTERVAL_MS as i64;
        let retry_job = CronJob {
            id: job_id.to_owned(),
            name: String::new(),
            enabled: true,
            schedule: CronSchedule::At {
                at_ms: next_run,
                description: None,
            },
            message: String::new(),
            execution_mode: ExecutionMode::Existing,
            agent_config: None,
            conversation_id: String::new(),
            conversation_title: None,
            agent_type: String::new(),
            created_by: CreatedBy::User,
            skill_content: None,
            description: None,
            created_at: 0,
            updated_at: 0,
            next_run_at: Some(next_run),
            last_run_at: None,
            last_status: None,
            last_error: None,
            run_count: 0,
            retry_count: 0,
            max_retries: 0,
            target_kind: TargetKind::Agent,
        };
        self.scheduler.schedule_job(&retry_job);
    }

    pub async fn delete_jobs_by_conversation(&self, conversation_id: i64) {
        let jobs = match self.repo.list_by_conversation(conversation_id).await {
            Ok(rows) => rows,
            Err(e) => {
                error!(conversation_id, error = %e, "Failed to list cron jobs for cascade delete");
                return;
            }
        };

        for row in &jobs {
            self.scheduler.cancel_job(&row.id);
            self.emitter.emit_job_removed(&row.id);
        }

        if let Err(e) = self.repo.delete_by_conversation(conversation_id).await {
            error!(conversation_id, error = %e, "Failed to cascade-delete cron jobs");
        } else if !jobs.is_empty() {
            info!(
                conversation_id,
                count = jobs.len(),
                "Cascade-deleted cron jobs for conversation"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// OnConversationDelete implementation (cascade delete)
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl nomifun_common::OnConversationDelete for CronService {
    async fn on_conversation_deleted(&self, conversation_id: i64) {
        self.delete_jobs_by_conversation(conversation_id).await;
    }
}

// ---------------------------------------------------------------------------
// ICronService implementation (for middleware)
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl nomifun_conversation::response_middleware::ICronService for CronService {
    async fn create_job(
        &self,
        _user_id: &str,
        conversation_id: &str,
        params: &nomifun_conversation::response_middleware::CronCreateParams,
    ) -> nomifun_conversation::response_middleware::CronCommandResult {
        // The trait carries the conversation id as `&str`; the DTO field is the
        // integer key. Reject a non-integer id up front rather than failing
        // deeper in `add_job`.
        let Ok(conversation_key) = conversation_id.parse::<i64>() else {
            return nomifun_conversation::response_middleware::CronCommandResult {
                success: false,
                message: format!("invalid conversation id '{conversation_id}'"),
            };
        };

        let schedule_dto = CronScheduleDto::Cron {
            expr: params.schedule.clone(),
            tz: None,
            description: Some(params.schedule_description.clone()),
        };

        let (agent_type, conversation_title, agent_config) = match self
            .executor
            .get_conversation_row(conversation_id)
            .await
        {
            Ok(Some(row)) => {
                let title = Some(row.name.clone());
                let (agent_type, agent_config) = build_agent_config_from_conversation(&row);
                (agent_type, title, agent_config)
            }
            Ok(None) => ("acp".to_owned(), None, None),
            Err(err) => {
                warn!(
                    conversation_id,
                    error = %err,
                    "Failed to load conversation context for cron create; falling back to defaults"
                );
                ("acp".to_owned(), None, None)
            }
        };

        let req = CreateCronJobRequest {
            name: params.name.clone(),
            description: None,
            schedule: schedule_dto,
            prompt: None,
            message: Some(params.message.clone()),
            conversation_id: conversation_key,
            conversation_title,
            agent_type,
            created_by: "agent".to_owned(),
            execution_mode: Some("existing".to_owned()),
            agent_config,
            target_kind: "agent".to_owned(),
        };

        match self.add_job(req).await {
            Ok(job) => {
                if let Err(err) = self
                    .executor
                    .bind_cron_job_to_conversation(conversation_id, &job.id)
                    .await
                {
                    warn!(
                        conversation_id,
                        job_id = %job.id,
                        error = %err,
                        "Cron job created but failed to bind conversation to job"
                    );
                }

                nomifun_conversation::response_middleware::CronCommandResult {
                    success: true,
                    message: format!("Created cron job '{}' ({})", job.name, job.id),
                }
            }
            Err(e) => nomifun_conversation::response_middleware::CronCommandResult {
                success: false,
                message: e.to_string(),
            },
        }
    }

    async fn update_job(
        &self,
        _user_id: &str,
        conversation_id: &str,
        params: &nomifun_conversation::response_middleware::CronUpdateParams,
    ) -> nomifun_conversation::response_middleware::CronCommandResult {
        let req = UpdateCronJobRequest {
            name: Some(params.name.clone()),
            description: None,
            enabled: None,
            schedule: Some(CronScheduleDto::Cron {
                expr: params.schedule.clone(),
                tz: None,
                description: Some(params.schedule_description.clone()),
            }),
            message: Some(params.message.clone()),
            execution_mode: None,
            agent_config: None,
            conversation_title: None,
            max_retries: None,
            target_kind: None,
        };

        match self.update_job(&params.job_id, req).await {
            Ok(job) => {
                if let Err(err) = self
                    .executor
                    .bind_cron_job_to_conversation(conversation_id, &job.id)
                    .await
                {
                    warn!(
                        conversation_id,
                        job_id = %job.id,
                        error = %err,
                        "Cron job updated but failed to bind conversation to job"
                    );
                }

                nomifun_conversation::response_middleware::CronCommandResult {
                    success: true,
                    message: format!("Updated cron job '{}' ({})", job.name, job.id),
                }
            }
            Err(e) => nomifun_conversation::response_middleware::CronCommandResult {
                success: false,
                message: e.to_string(),
            },
        }
    }

    async fn list_jobs(
        &self,
        _user_id: &str,
        conversation_id: &str,
    ) -> nomifun_conversation::response_middleware::CronCommandResult {
        // A non-integer conversation id can never scope to real jobs.
        let Ok(conversation_key) = conversation_id.parse::<i64>() else {
            return nomifun_conversation::response_middleware::CronCommandResult {
                success: true,
                message: format!("No cron jobs found for conversation '{}'.", conversation_id),
            };
        };
        let query = ListCronJobsQuery {
            conversation_id: Some(conversation_key),
        };
        match self.list_jobs(&query).await {
            Ok(jobs) => {
                if jobs.is_empty() {
                    return nomifun_conversation::response_middleware::CronCommandResult {
                        success: true,
                        message: format!(
                            "No cron jobs found for conversation '{}'.",
                            conversation_id
                        ),
                    };
                }

                let lines: Vec<String> = jobs
                    .iter()
                    .map(|j| {
                        let status = if j.enabled { "enabled" } else { "disabled" };
                        format!("- {} ({}) [{}]", j.name, j.id, status)
                    })
                    .collect();

                nomifun_conversation::response_middleware::CronCommandResult {
                    success: true,
                    message: format!(
                        "Found {} cron job(s) for conversation '{}':\n{}",
                        jobs.len(),
                        conversation_id,
                        lines.join("\n")
                    ),
                }
            }
            Err(e) => nomifun_conversation::response_middleware::CronCommandResult {
                success: false,
                message: e.to_string(),
            },
        }
    }

    async fn delete_job(
        &self,
        _user_id: &str,
        job_id: &str,
    ) -> nomifun_conversation::response_middleware::CronCommandResult {
        match self.remove_job(job_id).await {
            Ok(()) => nomifun_conversation::response_middleware::CronCommandResult {
                success: true,
                message: format!("Deleted cron job '{job_id}'"),
            },
            Err(e) => nomifun_conversation::response_middleware::CronCommandResult {
                success: false,
                message: e.to_string(),
            },
        }
    }
}

fn build_agent_config_from_conversation(
    row: &nomifun_db::models::ConversationRow,
) -> (String, Option<nomifun_api_types::CronAgentConfigDto>) {
    let extra = serde_json::from_str::<serde_json::Value>(&row.extra)
        .unwrap_or_else(|_| serde_json::json!({}));
    // Both interactive `send_message` and the cron executor parse
    // `conversation.model` via the same helper. Keeping the cron-side
    // `agent_config.backend` derivation in sync with that parser
    // prevents the cached vendor-label fallback (`"nomi"`) from
    // sneaking back in (Sentry ELECTRON-1HM).
    let model_resolved =
        nomifun_conversation::task_options::provider_model_from_conversation_row(row);
    let model = (!model_resolved.provider_id.is_empty()).then_some(&model_resolved);

    let backend = if row.r#type == "nomi" {
        model
            .map(|value| value.provider_id.clone())
            .filter(|value| !value.is_empty())
            .or_else(|| get_string(&extra, &["backend"]))
            .unwrap_or_else(|| "nomi".to_owned())
    } else {
        get_string(&extra, &["backend"])
            .or_else(|| {
                model
                    .map(|value| value.provider_id.clone())
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| row.r#type.clone())
    };

    let preset_assistant_id = get_string(&extra, &["preset_assistant_id", "presetAssistantId"]);
    let custom_agent_id =
        get_string(&extra, &["custom_agent_id", "customAgentId"]).or(preset_assistant_id.clone());
    let is_preset = preset_assistant_id.as_ref().map(|_| true);
    let preset_agent_type = if preset_assistant_id.is_some() {
        Some(backend.clone())
    } else {
        None
    };

    let agent_type_enum =
        serde_json::from_value::<AgentType>(serde_json::Value::String(row.r#type.clone())).ok();
    // Backend is now the vendor label (e.g. "claude"); pass through as
    // &str so `full_auto_mode_id` can key on it without re-parsing.
    let full_auto_mode = agent_type_enum
        .unwrap_or(AgentType::Acp)
        .full_auto_mode_id(Some(backend.as_str()))
        .to_owned();
    let agent_config = nomifun_api_types::CronAgentConfigDto {
        backend,
        name: get_string(&extra, &["agent_name", "agentName"]).unwrap_or_else(|| row.name.clone()),
        cli_path: get_string(&extra, &["cli_path", "cliPath"]).or_else(|| {
            extra
                .get("gateway")
                .and_then(|gateway| gateway.get("cli_path").or_else(|| gateway.get("cliPath")))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        }),
        is_preset,
        custom_agent_id,
        preset_agent_type,
        mode: Some(full_auto_mode),
        model_id: get_string(&extra, &["current_model_id", "currentModelId"]).or_else(|| {
            model.and_then(|value| {
                value
                    .use_model
                    .clone()
                    .or_else(|| (!value.model.is_empty()).then(|| value.model.clone()))
            })
        }),
        config_options: None,
        workspace: get_string(&extra, &["workspace"]),
        clear_context_each_run: false,
    };

    (row.r#type.clone(), Some(agent_config))
}
fn get_string(extra: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        extra
            .get(*key)
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned)
            .filter(|value| !value.is_empty())
    })
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Nomi cron jobs require `agent_config.backend` (provider_id) to be set —
/// the executor uses it to look up the provider row and build the agent.
/// Reject add/update requests that would produce an invalid nomi job.
///
/// The literal `"nomi"` is ALSO rejected: it is the cached vendor-label
/// fallback `build_agent_config_from_conversation` emits when the bound
/// conversation has NO configured model. Letting it through used to defer
/// the failure to execution time, where it surfaced as an opaque
/// "Provider '' not found" — fail at creation with a clear message instead.
fn validate_nomi_agent_config(
    agent_type: &str,
    agent_config: Option<&nomifun_api_types::CronAgentConfigDto>,
) -> Result<(), CronError> {
    if agent_type != "nomi" {
        return Ok(());
    }
    let backend = agent_config.map(|c| c.backend.trim()).unwrap_or("");
    if backend.is_empty() || backend == "nomi" {
        return Err(CronError::InvalidAgentConfig(
            "the bound nomi conversation has no model configured (agent_config.backend must be a provider_id); \
             set the conversation's model first, then create the job"
                .into(),
        ));
    }
    Ok(())
}

/// Where a nomi cron job's model comes from. Used to pick the right validation
/// without performing any I/O, so the routing decision is unit-testable on its
/// own.
#[derive(Debug, PartialEq, Eq)]
enum NomiModelCheck {
    /// Not a nomi job — no model validation applies.
    Skip,
    /// `agent_config.backend` is the model source; apply the static check.
    AgentConfig,
    /// The bound conversation is the model source; load it and confirm it has a
    /// model.
    BoundConversation,
}

/// Decide how a nomi agent job's model must be validated. Pure (no I/O).
///
/// An `Existing` job bound to a non-empty `conversation_id` resolves its model
/// from that conversation at run time (`executor::execute_inner`), so
/// `agent_config` need not carry one — the desktop "指定会话" flow legitimately
/// omits it. Everything else (a new conversation, or a lazy-bind `Existing` job
/// with no conversation yet) relies on `agent_config.backend`.
fn nomi_model_check(
    agent_type: &str,
    execution_mode: ExecutionMode,
    conversation_id: &str,
) -> NomiModelCheck {
    if agent_type != "nomi" {
        return NomiModelCheck::Skip;
    }
    if matches!(execution_mode, ExecutionMode::Existing) && !conversation_id.trim().is_empty() {
        NomiModelCheck::BoundConversation
    } else {
        NomiModelCheck::AgentConfig
    }
}

fn parse_execution_mode(mode: Option<&str>) -> Result<ExecutionMode, CronError> {
    match mode {
        None | Some("existing") => Ok(ExecutionMode::Existing),
        Some(s) => ExecutionMode::from_str(s),
    }
}

fn validate_skill_body_content(content: &str) -> Result<(), CronError> {
    let trimmed = content.trim();

    if trimmed.is_empty() {
        return Err(CronError::InvalidSkillContent(
            "content must not be empty".into(),
        ));
    }

    let lower = trimmed.to_lowercase();
    for pattern in PLACEHOLDER_PATTERNS {
        if lower.starts_with(pattern) {
            return Err(CronError::InvalidSkillContent(
                "content appears to be placeholder text".into(),
            ));
        }
    }

    Ok(())
}

fn schedule_description(schedule: &CronSchedule) -> Option<&str> {
    match schedule {
        CronSchedule::At { description, .. }
        | CronSchedule::Every { description, .. }
        | CronSchedule::Cron { description, .. } => description.as_deref(),
    }
}

async fn persist_skill_file(
    data_dir: &Path,
    job: &CronJob,
    raw_content: &str,
) -> Result<(), CronError> {
    match write_raw_skill_file(data_dir, &job.id, raw_content).await {
        Ok(_) => Ok(()),
        Err(CronError::InvalidSkillContent(_)) => {
            let description = job
                .description
                .clone()
                .unwrap_or_else(|| format!("Saved cron skill for {}", job.name));
            write_skill_file(
                data_dir,
                &job.id,
                &job.name,
                &description,
                raw_content.trim(),
                schedule_description(&job.schedule),
            )
            .await
            .map(|_| ())
        }
        Err(err) => Err(err),
    }
}

fn is_removed_terminal_target(row_target_kind: &str, err: &CronError) -> bool {
    row_target_kind == "terminal" && matches!(err, CronError::InvalidTargetKind(_))
}

fn build_update_params(job: &CronJob, req: &UpdateCronJobRequest) -> UpdateCronJobParams {
    let (schedule_kind, schedule_value, schedule_tz, schedule_description) =
        if req.schedule.is_some() {
            let (k, v, tz, d) = schedule_to_row_fields(&job.schedule);
            (Some(k), Some(v), Some(tz), Some(d))
        } else {
            (None, None, None, None)
        };

    let agent_config = req.agent_config.as_ref().map(|c| {
        let config = CronAgentConfig {
            backend: c.backend.clone(),
            name: c.name.clone(),
            cli_path: c.cli_path.clone(),
            is_preset: c.is_preset,
            custom_agent_id: c.custom_agent_id.clone(),
            preset_agent_type: c.preset_agent_type.clone(),
            mode: c.mode.clone(),
            model_id: c.model_id.clone(),
            config_options: c.config_options.clone(),
            workspace: c.workspace.clone(),
            clear_context_each_run: c.clear_context_each_run,
        };
        Some(serde_json::to_string(&config).unwrap_or_default())
    });

    // Persist the target discriminator only when the request touched it. The
    // legacy terminal columns are cleared on that write; schema compatibility
    // remains, but terminal scheduling behavior is gone.
    let target_touched = req.target_kind.is_some();
    let target_kind = target_touched.then(|| job.target_kind.as_str().to_owned());

    UpdateCronJobParams {
        name: req.name.clone(),
        enabled: req.enabled,
        schedule_kind,
        schedule_value,
        schedule_tz,
        schedule_description,
        payload_message: req.message.clone(),
        execution_mode: req.execution_mode.clone(),
        agent_config,
        conversation_id: None,
        conversation_title: req.conversation_title.as_ref().map(|t| Some(t.clone())),
        agent_type: None,
        skill_content: None,
        description: req.description.as_ref().map(|value| Some(value.clone())),
        next_run_at: if req.schedule.is_some() || req.enabled.is_some() {
            Some(job.next_run_at)
        } else {
            None
        },
        last_run_at: None,
        last_status: None,
        last_error: None,
        run_count: None,
        retry_count: None,
        target_kind,
        terminal_mode: target_touched.then_some(None),
        terminal_session_id: target_touched.then_some(None),
        terminal_command: target_touched.then_some(None),
        terminal_args: target_touched.then_some(None),
        terminal_script: target_touched.then_some(None),
    }
}

fn build_run_row(job_id: &str, status: &str) -> CronJobRunRow {
    let now = now_ms();
    CronJobRunRow {
        id: generate_prefixed_id("cron_run"),
        job_id: job_id.to_owned(),
        executed_at_ms: now,
        status: status.to_owned(),
        created_at_ms: now,
    }
}

fn cron_run_to_response(row: CronJobRunRow) -> CronJobRunResponse {
    CronJobRunResponse {
        id: row.id,
        job_id: row.job_id,
        executed_at_ms: row.executed_at_ms,
        status: row.status,
    }
}

fn schedule_from_dto_with_existing_timezone(
    dto: &CronScheduleDto,
    existing: &CronSchedule,
) -> CronSchedule {
    match dto {
        CronScheduleDto::Cron {
            expr,
            tz,
            description,
        } => CronSchedule::Cron {
            expr: expr.clone(),
            tz: tz.clone().or_else(|| match existing {
                CronSchedule::Cron { tz, .. } => tz.clone(),
                _ => None,
            }),
            description: description.clone(),
        },
        _ => schedule_from_dto(dto),
    }
}

fn schedule_to_row_fields(
    schedule: &CronSchedule,
) -> (String, String, Option<String>, Option<String>) {
    match schedule {
        CronSchedule::At { at_ms, description } => (
            "at".to_owned(),
            at_ms.to_string(),
            None,
            description.clone(),
        ),
        CronSchedule::Every {
            every_ms,
            description,
        } => (
            "every".to_owned(),
            every_ms.to_string(),
            None,
            description.clone(),
        ),
        CronSchedule::Cron {
            expr,
            tz,
            description,
        } => (
            "cron".to_owned(),
            expr.clone(),
            tz.clone(),
            description.clone(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- validate_skill_body_content -------------------------------------------

    #[test]
    fn validate_skill_empty_content() {
        let err = validate_skill_body_content("").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_whitespace_only() {
        let err = validate_skill_body_content("   \n  ").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_placeholder_todo() {
        let err = validate_skill_body_content("TODO: fill in later").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_placeholder_fill_in() {
        let err = validate_skill_body_content("Fill in your instructions here").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_placeholder_replace() {
        let err = validate_skill_body_content("Replace this with your skill").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_valid_content() {
        assert!(validate_skill_body_content("---\nname: test\n---\nDo something useful").is_ok());
    }

    #[test]
    fn validate_skill_valid_short() {
        assert!(validate_skill_body_content("Run daily report").is_ok());
    }

    // -- validate_nomi_agent_config ----------------------------------------

    fn agent_cfg_dto(backend: &str) -> nomifun_api_types::CronAgentConfigDto {
        nomifun_api_types::CronAgentConfigDto {
            backend: backend.to_owned(),
            name: "provider".into(),
            cli_path: None,
            is_preset: None,
            custom_agent_id: None,
            preset_agent_type: None,
            mode: None,
            model_id: Some("gpt-4o".into()),
            config_options: None,
            workspace: None,
            clear_context_each_run: false,
        }
    }

    #[test]
    fn validate_nomi_accepts_valid_config() {
        let cfg = agent_cfg_dto("4056cdea");
        assert!(validate_nomi_agent_config("nomi", Some(&cfg)).is_ok());
    }

    #[test]
    fn validate_nomi_rejects_missing_config() {
        let err = validate_nomi_agent_config("nomi", None).unwrap_err();
        assert!(matches!(err, CronError::InvalidAgentConfig(_)));
    }

    #[test]
    fn validate_nomi_rejects_empty_backend() {
        let cfg = agent_cfg_dto("");
        let err = validate_nomi_agent_config("nomi", Some(&cfg)).unwrap_err();
        assert!(matches!(err, CronError::InvalidAgentConfig(_)));
    }

    #[test]
    fn validate_nomi_rejects_whitespace_backend() {
        let cfg = agent_cfg_dto("   ");
        let err = validate_nomi_agent_config("nomi", Some(&cfg)).unwrap_err();
        assert!(matches!(err, CronError::InvalidAgentConfig(_)));
    }

    /// `build_agent_config_from_conversation` falls back to the literal
    /// `"nomi"` backend when the bound conversation has no model — that
    /// placeholder used to slip past validation and explode at execution
    /// time ("Provider '' not found"). It must be rejected at creation.
    #[test]
    fn validate_nomi_rejects_placeholder_nomi_backend() {
        let cfg = agent_cfg_dto("nomi");
        let err = validate_nomi_agent_config("nomi", Some(&cfg)).unwrap_err();
        assert!(matches!(err, CronError::InvalidAgentConfig(_)));
        assert!(err.to_string().contains("no model configured"), "{err}");
    }

    #[test]
    fn validate_nomi_placeholder_check_trims_whitespace() {
        let cfg = agent_cfg_dto("  nomi  ");
        let err = validate_nomi_agent_config("nomi", Some(&cfg)).unwrap_err();
        assert!(matches!(err, CronError::InvalidAgentConfig(_)));
    }

    #[test]
    fn validate_nomi_ignores_non_nomi_type() {
        // ACP / other types may legitimately omit agent_config or leave backend empty.
        assert!(validate_nomi_agent_config("acp", None).is_ok());
        let cfg = agent_cfg_dto("");
        assert!(validate_nomi_agent_config("claude", Some(&cfg)).is_ok());
    }

    // -- nomi_model_check (execution-mode-aware routing) ----------------------

    #[test]
    fn nomi_model_check_skips_non_nomi() {
        // Non-nomi jobs never carry a nomi model requirement, regardless of mode.
        assert_eq!(
            nomi_model_check("acp", ExecutionMode::Existing, "42"),
            NomiModelCheck::Skip
        );
        assert_eq!(
            nomi_model_check("claude", ExecutionMode::NewConversation, ""),
            NomiModelCheck::Skip
        );
    }

    #[test]
    fn nomi_existing_bound_conversation_is_model_source() {
        // 指定会话 / reuse an existing conversation: the model comes from the
        // bound conversation at run time, so agent_config is NOT what we check.
        // This is the case the desktop create flow hit (agent_config omitted).
        assert_eq!(
            nomi_model_check("nomi", ExecutionMode::Existing, "42"),
            NomiModelCheck::BoundConversation
        );
    }

    #[test]
    fn nomi_new_conversation_requires_agent_config() {
        // A fresh conversation is created from agent_config, so its backend
        // (provider_id) must be present.
        assert_eq!(
            nomi_model_check("nomi", ExecutionMode::NewConversation, "42"),
            NomiModelCheck::AgentConfig
        );
    }

    #[test]
    fn nomi_existing_without_bound_conversation_requires_agent_config() {
        // Lazy-bind Existing job (no conversation yet): the first run creates a
        // new conversation from agent_config, so agent_config.backend is required.
        assert_eq!(
            nomi_model_check("nomi", ExecutionMode::Existing, ""),
            NomiModelCheck::AgentConfig
        );
        assert_eq!(
            nomi_model_check("nomi", ExecutionMode::Existing, "   "),
            NomiModelCheck::AgentConfig
        );
    }

    // -- parse_execution_mode -------------------------------------------------

    #[test]
    fn parse_mode_none_defaults_to_existing() {
        assert_eq!(parse_execution_mode(None).unwrap(), ExecutionMode::Existing);
    }

    #[test]
    fn parse_mode_existing() {
        assert_eq!(
            parse_execution_mode(Some("existing")).unwrap(),
            ExecutionMode::Existing
        );
    }

    #[test]
    fn parse_mode_new_conversation() {
        assert_eq!(
            parse_execution_mode(Some("new_conversation")).unwrap(),
            ExecutionMode::NewConversation
        );
    }

    #[test]
    fn parse_mode_invalid() {
        let err = parse_execution_mode(Some("parallel")).unwrap_err();
        assert!(matches!(err, CronError::InvalidExecutionMode(_)));
    }

    // -- build_update_params --------------------------------------------------

    fn sample_job() -> CronJob {
        CronJob {
            id: "cron_test".into(),
            name: "Test".into(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: 60000,
                description: None,
            },
            message: "do something".into(),
            execution_mode: ExecutionMode::Existing,
            agent_config: None,
            conversation_id: "conv_1".into(),
            conversation_title: None,
            agent_type: "acp".into(),
            created_by: CreatedBy::User,
            skill_content: None,
            description: None,
            created_at: 1000,
            updated_at: 2000,
            next_run_at: Some(61000),
            last_run_at: None,
            last_status: None,
            last_error: None,
            run_count: 0,
            retry_count: 0,
            max_retries: 3,
            target_kind: TargetKind::Agent,
        }
    }

    #[test]
    fn build_run_row_records_minimal_execution_fact() {
        let before = now_ms();
        let row = build_run_row("cron_test", "ok");
        let after = now_ms();

        assert!(row.id.starts_with("cron_run_"));
        assert_eq!(row.job_id, "cron_test");
        assert_eq!(row.status, "ok");
        assert!(row.executed_at_ms >= before);
        assert!(row.executed_at_ms <= after);
        assert_eq!(row.created_at_ms, row.executed_at_ms);
    }

    #[test]
    fn build_update_params_name_only() {
        let job = sample_job();
        let req = UpdateCronJobRequest {
            name: Some("New Name".into()),
            description: None,
            enabled: None,
            schedule: None,
            message: None,
            execution_mode: None,
            agent_config: None,
            conversation_title: None,
            max_retries: None,
            target_kind: None,
        };
        let params = build_update_params(&job, &req);
        assert_eq!(params.name.as_deref(), Some("New Name"));
        assert!(params.enabled.is_none());
        assert!(params.schedule_kind.is_none());
        assert!(params.next_run_at.is_none());
    }

    #[test]
    fn build_update_params_with_schedule_change() {
        let job = CronJob {
            schedule: CronSchedule::Cron {
                expr: "0 0 9 * * *".into(),
                tz: Some("UTC".into()),
                description: Some("daily".into()),
            },
            next_run_at: Some(99999),
            ..sample_job()
        };
        let req = UpdateCronJobRequest {
            name: None,
            description: None,
            enabled: None,
            schedule: Some(CronScheduleDto::Cron {
                expr: "0 0 9 * * *".into(),
                tz: Some("UTC".into()),
                description: Some("daily".into()),
            }),
            message: None,
            execution_mode: None,
            agent_config: None,
            conversation_title: None,
            max_retries: None,
            target_kind: None,
        };
        let params = build_update_params(&job, &req);
        assert_eq!(params.schedule_kind.as_deref(), Some("cron"));
        assert_eq!(params.schedule_value.as_deref(), Some("0 0 9 * * *"));
        assert!(params.next_run_at.is_some());
    }

    #[test]
    fn preserves_existing_cron_timezone_when_update_omits_tz() {
        let existing = CronSchedule::Cron {
            expr: "0 0 9 * * *".into(),
            tz: Some("Asia/Shanghai".into()),
            description: Some("daily".into()),
        };
        let dto = CronScheduleDto::Cron {
            expr: "0 30 9 * * *".into(),
            tz: None,
            description: Some("daily".into()),
        };

        let schedule = schedule_from_dto_with_existing_timezone(&dto, &existing);

        assert_eq!(
            schedule,
            CronSchedule::Cron {
                expr: "0 30 9 * * *".into(),
                tz: Some("Asia/Shanghai".into()),
                description: Some("daily".into()),
            }
        );
    }

    #[test]
    fn build_update_params_enabled_change_triggers_next_run() {
        let job = sample_job();
        let req = UpdateCronJobRequest {
            name: None,
            description: None,
            enabled: Some(false),
            schedule: None,
            message: None,
            execution_mode: None,
            agent_config: None,
            conversation_title: None,
            max_retries: None,
            target_kind: None,
        };
        let params = build_update_params(&job, &req);
        assert_eq!(params.enabled, Some(false));
        assert!(params.next_run_at.is_some());
    }

    #[test]
    fn build_update_params_description_only() {
        let job = sample_job();
        let req = UpdateCronJobRequest {
            name: None,
            description: Some("Updated description".into()),
            enabled: None,
            schedule: None,
            message: None,
            execution_mode: None,
            agent_config: None,
            conversation_title: None,
            max_retries: None,
            target_kind: None,
        };
        let params = build_update_params(&job, &req);
        assert_eq!(
            params
                .description
                .as_ref()
                .and_then(|value| value.as_deref()),
            Some("Updated description")
        );
    }

    // -- schedule_to_row_fields -----------------------------------------------

    #[test]
    fn row_fields_at() {
        let (kind, value, tz, desc) = schedule_to_row_fields(&CronSchedule::At {
            at_ms: 5000,
            description: Some("once".into()),
        });
        assert_eq!(kind, "at");
        assert_eq!(value, "5000");
        assert!(tz.is_none());
        assert_eq!(desc.as_deref(), Some("once"));
    }

    #[test]
    fn row_fields_every() {
        let (kind, value, tz, desc) = schedule_to_row_fields(&CronSchedule::Every {
            every_ms: 30000,
            description: None,
        });
        assert_eq!(kind, "every");
        assert_eq!(value, "30000");
        assert!(tz.is_none());
        assert!(desc.is_none());
    }

    #[test]
    fn row_fields_cron() {
        let (kind, value, tz, desc) = schedule_to_row_fields(&CronSchedule::Cron {
            expr: "0 0 * * * *".into(),
            tz: Some("UTC".into()),
            description: Some("hourly".into()),
        });
        assert_eq!(kind, "cron");
        assert_eq!(value, "0 0 * * * *");
        assert_eq!(tz.as_deref(), Some("UTC"));
        assert_eq!(desc.as_deref(), Some("hourly"));
    }
}
