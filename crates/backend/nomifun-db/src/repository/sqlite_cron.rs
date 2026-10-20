use nomifun_common::{ConversationId, CronJobId, CronJobRunId, UserId, now_ms};
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{CronJobRow, CronRunHistoryRow, CronRunReservationRow};
use crate::repository::bind::{BindValue, bind_value};
use crate::repository::cron::{
    CRON_RUN_HISTORY_LIMIT, FinalizeCronRunOutcome, FinalizeCronRunParams, ICronRepository,
    ReserveCronRunParams, UpdateCronJobParams,
};

#[derive(Clone, Debug)]
pub struct SqliteCronRepository {
    pool: SqlitePool,
}

impl SqliteCronRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn insert_row(
        &self,
        row: &CronJobRow,
        bind_session_relation: bool,
    ) -> Result<(), DbError> {
        nomifun_common::CronJobId::parse(&row.cron_job_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron_job_id: {error}")))?;
        if row.last_run_id.is_some() {
            return Err(DbError::Conflict(
                "new Cron definitions cannot author runtime summary provenance".into(),
            ));
        }
        if bind_session_relation && row.conversation_id.is_none() {
            return Err(DbError::Conflict(
                "atomic Cron/Session relation insertion requires a conversation_id".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        validate_cron_binding(
            &mut tx,
            &row.user_id,
            row.enabled,
            &row.execution_mode,
            row.conversation_id.as_deref(),
            row.agent_binding.as_deref(),
            row.skill_content.as_deref(),
        )
        .await?;
        if bind_session_relation {
            ensure_session_relation_available(
                &mut tx,
                &row.user_id,
                row.conversation_id
                    .as_deref()
                    .expect("atomic relation insertion validated conversation_id"),
                None,
            )
            .await?;
        }
        sqlx::query("INSERT INTO cron_jobs (cron_job_id, user_id, name, enabled, schedule_revision, schedule_kind, schedule_value, schedule_tz, schedule_description, payload_message, execution_mode, agent_selection, model_selection, agent_binding, clear_context_each_run, configuration_error, conversation_id, conversation_title, created_by, skill_content, description, created_at, updated_at, next_run_at, last_run_at, last_run_id, last_status, last_error, run_count) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&row.cron_job_id)
            .bind(&row.user_id)
            .bind(&row.name)
            .bind(&row.enabled)
            .bind(&row.schedule_revision)
            .bind(&row.schedule_kind)
            .bind(&row.schedule_value)
            .bind(&row.schedule_tz)
            .bind(&row.schedule_description)
            .bind(&row.payload_message)
            .bind(&row.execution_mode)
            .bind(&row.agent_selection)
            .bind(&row.model_selection)
            .bind(&row.agent_binding)
            .bind(&row.clear_context_each_run)
            .bind(&row.configuration_error)
            .bind(&row.conversation_id)
            .bind(&row.conversation_title)
            .bind(&row.created_by)
            .bind(&row.skill_content)
            .bind(&row.description)
            .bind(&row.created_at)
            .bind(&row.updated_at)
            .bind(&row.next_run_at)
            .bind(&row.last_run_at)
            .bind(&row.last_run_id)
            .bind(&row.last_status)
            .bind(&row.last_error)
            .bind(&row.run_count)
            .execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(())
    }
}

/// Database writes validate the standard immutable reference chain, never a
/// Cron-specific Agent mirror. Missing configuration is a disabled draft.
async fn validate_cron_binding(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    enabled: bool,
    execution_mode: &str,
    conversation_id: Option<&str>,
    binding: Option<&str>,
    skill_content: Option<&str>,
) -> Result<(), DbError> {
    if execution_mode == "new_conversation" && conversation_id.is_some() {
        return Err(DbError::Conflict(
            "per-occurrence Cron definitions cannot own an anchor Session".into(),
        ));
    }
    if let Some(session) = conversation_id {
        lock_owned_session_relation(tx, user_id, session).await?;
    }
    let installation_owner: String = sqlx::query_scalar(
        "SELECT owner_user_id FROM installation_identity WHERE singleton_key='installation'",
    )
    .fetch_one(&mut **tx)
    .await?;
    if let Some(raw) = binding {
        let binding: nomifun_agent_contracts::AgentBindingValue = serde_json::from_str(raw)
            .map_err(|error| DbError::Conflict(format!("invalid Cron Agent binding: {error}")))?;
        if binding
            .typed_resource_bindings
            .iter()
            .any(|resource| resource.owner_id != user_id)
        {
            return Err(DbError::Conflict(
                "Cron Agent resource owner mismatch".into(),
            ));
        }
        if binding.binding_version == 0 {
            return Err(DbError::Conflict(
                "Cron Agent binding version is invalid".into(),
            ));
        }
        for resource in &binding.typed_resource_bindings {
            let expected = nomifun_agent_contracts::resource_definition_id(resource)
                .map_err(|error| DbError::Conflict(error.to_string()))?;
            if expected != resource.binding_id {
                return Err(DbError::Conflict(
                    "Cron resource definition digest differs".into(),
                ));
            }
        }
        let revision =
            crate::provider_cleanup::revision_artifact_tx(tx, &binding.preset_revision_ref).await?;
        if revision.created_by.as_ref() != user_id {
            return Err(DbError::Conflict(
                "Cron Agent Revision owner mismatch".into(),
            ));
        }
        let raw:Option<String>=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=? AND snapshot_digest=?")
            .bind(binding.resolved_snapshot_ref.snapshot_id.as_ref()).bind(binding.resolved_snapshot_ref.snapshot_digest.as_ref()).fetch_optional(&mut **tx).await?;
        let snapshot: nomifun_agent_contracts::ResolvedSnapshotEnvelope = serde_json::from_str(
            &raw.ok_or_else(|| DbError::Conflict("Cron exact Snapshot is missing".into()))?,
        )
        .map_err(|error| DbError::Conflict(error.to_string()))?;
        snapshot
            .validate()
            .map_err(|error| DbError::Conflict(error.message))?;
        if snapshot.content.preset_revision_ref != binding.preset_revision_ref
            || snapshot.snapshot_ref != binding.resolved_snapshot_ref
            || snapshot.actor.principal_kind != "user"
            || snapshot.actor.principal_id != user_id
        {
            return Err(DbError::Conflict(
                "Cron Agent immutable identity or owner differs".into(),
            ));
        }
        if enabled {
            let route = revision
                .payload
                .chat_route_records
                .get(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT)
                .ok_or_else(|| {
                    DbError::Conflict("Scheduled Agent has no exact Chat route".into())
                })?;
            let available:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM providers provider JOIN provider_models model ON model.provider_id=provider.provider_id WHERE provider.provider_id=? AND provider.enabled=1 AND model.model=? AND model.enabled=1)")
                .bind(&route.primary.provider_id).bind(&route.primary.model).fetch_one(&mut **tx).await?;
            if !available {
                return Err(DbError::Conflict(
                    "Select a model again: the frozen scheduled model is unavailable".into(),
                ));
            }
        }
        if !crate::cron_binding_normalization::complete_resources(&binding, &snapshot, user_id) {
            return Err(DbError::Conflict(
                "Cron Agent has incomplete frozen resource operation grants".into(),
            ));
        }
        if user_id != installation_owner
            && (!binding.typed_resource_bindings.is_empty() || skill_content.is_some())
        {
            return Err(DbError::Conflict(
                "non-owner scheduled Agent cannot obtain host resources".into(),
            ));
        }
    } else if enabled && !(execution_mode == "existing" && conversation_id.is_some()) {
        return Err(DbError::Conflict(
            "enabled Cron job requires a canonical Agent binding or bound Session".into(),
        ));
    }
    Ok(())
}

/// Lock and authenticate the exact Session relation target inside the caller's
/// write transaction. The target is a live canonical AgentSession owned by
/// this user. A tombstone or deleting row is not a valid scheduling target.
async fn lock_owned_session_relation(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    agent_session_id: &str,
) -> Result<(), DbError> {
    let canonical = sqlx::query(
        "UPDATE agent_sessions SET next_seq = next_seq \
         WHERE agent_session_id = ? AND state = 'live' \
           AND json_extract(owner_ref_json, '$.principal_kind') = 'user' \
           AND json_extract(owner_ref_json, '$.principal_id') = ?",
    )
    .bind(agent_session_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if canonical == 1 {
        return Ok(());
    }

    Err(DbError::Conflict(
        "cron job AgentSession owner mismatch or Session is not live".into(),
    ))
}

/// Prove that this owner-scoped Session has no different Cron relation while
/// the same SQLite writer lock acquired by [`lock_owned_session_relation`] is
/// still held. This supplies the one-Session/one-Cron CAS for canonical Store
/// sessions, which intentionally have no mutable `cron_job_id` back-reference.
async fn ensure_session_relation_available(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    agent_session_id: &str,
    allowed_cron_job_id: Option<&str>,
) -> Result<(), DbError> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT cron_job_id FROM cron_jobs \
         WHERE user_id = ? AND conversation_id = ? \
         ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .bind(agent_session_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(existing) = existing
        && Some(existing.as_str()) != allowed_cron_job_id
    {
        return Err(DbError::Conflict(format!(
            "AgentSession '{agent_session_id}' is already bound to Cron job {existing}"
        )));
    }
    Ok(())
}

#[async_trait::async_trait]
impl ICronRepository for SqliteCronRepository {
    async fn insert(&self, row: &CronJobRow) -> Result<(), DbError> {
        self.insert_row(row, false).await
    }

    async fn insert_with_session_relation(&self, row: &CronJobRow) -> Result<(), DbError> {
        self.insert_row(row, true).await
    }

    async fn update(
        &self,
        user_id: &str,
        cron_job_id: &str,
        params: &UpdateCronJobParams,
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let existing = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE cron_job_id = ? AND user_id = ?",
        )
        .bind(cron_job_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("cron job '{cron_job_id}'")))?;
        if let Some(expected_schedule_revision) = params.expected_schedule_revision
            && existing.schedule_revision != expected_schedule_revision
        {
            return Err(DbError::Conflict(format!(
                "cron job '{cron_job_id}' schedule revision changed from expected \
                 revision {expected_schedule_revision} to {}",
                existing.schedule_revision
            )));
        }

        let mut set_parts: Vec<String> = Vec::new();
        let mut binds: Vec<BindValue> = Vec::new();

        macro_rules! push_str {
            ($field:ident) => {
                if let Some(ref v) = params.$field {
                    set_parts.push(concat!(stringify!($field), " = ?").to_string());
                    binds.push(BindValue::Str(v.clone()));
                }
            };
        }

        macro_rules! push_opt_str {
            ($field:ident) => {
                if let Some(ref v) = params.$field {
                    set_parts.push(concat!(stringify!($field), " = ?").to_string());
                    binds.push(BindValue::OptStr(v.clone()));
                }
            };
        }

        macro_rules! push_opt_i64 {
            ($field:ident) => {
                if let Some(ref v) = params.$field {
                    set_parts.push(concat!(stringify!($field), " = ?").to_string());
                    binds.push(BindValue::OptI64(*v));
                }
            };
        }

        macro_rules! push_i64 {
            ($field:ident) => {
                if let Some(v) = params.$field {
                    set_parts.push(concat!(stringify!($field), " = ?").to_string());
                    binds.push(BindValue::I64(v));
                }
            };
        }

        if let Some(v) = params.enabled {
            set_parts.push("enabled = ?".to_string());
            binds.push(BindValue::Bool(v));
        }

        push_str!(name);
        push_i64!(schedule_revision);
        push_str!(schedule_kind);
        push_str!(schedule_value);
        push_opt_str!(schedule_tz);
        push_opt_str!(schedule_description);
        push_str!(payload_message);
        push_str!(execution_mode);
        push_opt_str!(agent_selection);
        push_opt_str!(model_selection);
        push_opt_str!(agent_binding);
        push_opt_str!(configuration_error);
        if let Some(value) = params.clear_context_each_run {
            set_parts.push("clear_context_each_run = ?".into());
            binds.push(BindValue::Bool(value));
        }
        push_opt_str!(conversation_id);
        push_opt_str!(conversation_title);
        push_opt_str!(skill_content);
        push_opt_str!(description);
        push_opt_i64!(next_run_at);
        push_opt_i64!(last_run_at);
        push_opt_str!(last_status);
        push_opt_str!(last_error);
        push_i64!(run_count);

        if set_parts.is_empty() {
            tx.commit().await?;
            return Ok(());
        }

        let final_binding = params
            .agent_binding
            .as_ref()
            .map(|value| value.as_deref())
            .unwrap_or(existing.agent_binding.as_deref());
        let final_session = params
            .conversation_id
            .as_ref()
            .map(|value| value.as_deref())
            .unwrap_or(existing.conversation_id.as_deref());
        let final_skill = params
            .skill_content
            .as_ref()
            .map(|value| value.as_deref())
            .unwrap_or(existing.skill_content.as_deref());
        validate_cron_binding(
            &mut tx,
            user_id,
            params.enabled.unwrap_or(existing.enabled),
            params
                .execution_mode
                .as_deref()
                .unwrap_or(&existing.execution_mode),
            final_session,
            final_binding,
            final_skill,
        )
        .await?;

        set_parts.push("updated_at = ?".to_string());
        binds.push(BindValue::I64(now_ms()));

        let schedule_revision_predicate = if params.expected_schedule_revision.is_some() {
            " AND schedule_revision = ?"
        } else {
            ""
        };
        let sql = format!(
            "UPDATE cron_jobs SET {} WHERE cron_job_id = ? AND user_id = ?{}",
            set_parts.join(", "),
            schedule_revision_predicate
        );

        let mut query = sqlx::query(&sql);
        for bind in &binds {
            query = bind_value(query, bind);
        }
        query = query.bind(cron_job_id);
        query = query.bind(user_id);
        if let Some(expected_schedule_revision) = params.expected_schedule_revision {
            query = query.bind(expected_schedule_revision);
        }

        let result = query.execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            if let Some(expected_schedule_revision) = params.expected_schedule_revision {
                return Err(DbError::Conflict(format!(
                    "cron job '{cron_job_id}' schedule revision changed from expected \
                     revision {expected_schedule_revision}"
                )));
            }
            return Err(DbError::NotFound(format!("cron job '{cron_job_id}'")));
        }
        tx.commit().await?;
        Ok(())
    }

    async fn delete(&self, user_id: &str, cron_job_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE cron_jobs \
             SET updated_at = updated_at \
             WHERE cron_job_id = ? AND user_id = ?",
        )
        .bind(cron_job_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("cron job '{cron_job_id}'")));
        }

        let reserved_run_id: Option<String> = sqlx::query_scalar(
            "SELECT cron_job_run_id FROM cron_run_reservations \
             WHERE cron_job_id = ? AND status = 'reserved' \
             ORDER BY created_at_ms, id LIMIT 1",
        )
        .bind(cron_job_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(run_id) = reserved_run_id {
            return Err(DbError::Conflict(format!(
                "cron job '{cron_job_id}' has admitted run '{run_id}'; \
                 wait for its durable receipt to become terminal before deletion"
            )));
        }

        sqlx::query("DELETE FROM cron_run_reservations WHERE cron_job_id = ?")
            .bind(cron_job_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM cron_jobs WHERE cron_job_id = ? AND user_id = ?")
            .bind(cron_job_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn get_by_cron_job_id(
        &self,
        user_id: &str,
        cron_job_id: &str,
    ) -> Result<Option<CronJobRow>, DbError> {
        let row = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE cron_job_id = ? AND user_id = ?",
        )
        .bind(cron_job_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn list_all(&self, user_id: &str) -> Result<Vec<CronJobRow>, DbError> {
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE user_id = ? ORDER BY created_at ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get_by_cron_job_id_for_scheduler(
        &self,
        cron_job_id: &str,
    ) -> Result<Option<CronJobRow>, DbError> {
        let row = sqlx::query_as::<_, CronJobRow>("SELECT * FROM cron_jobs WHERE cron_job_id = ?")
            .bind(cron_job_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn list_enabled_for_scheduler(&self) -> Result<Vec<CronJobRow>, DbError> {
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE enabled = 1 ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_by_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<CronJobRow>, DbError> {
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE user_id = ? AND conversation_id = ? ORDER BY created_at ASC",
        )
        .bind(user_id)
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete_by_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<String>, DbError> {
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE cron_jobs \
             SET updated_at = updated_at \
             WHERE user_id = ? AND conversation_id = ?",
        )
        .bind(user_id)
        .bind(conversation_id)
        .execute(&mut *tx)
        .await?;
        if locked.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(Vec::new());
        }

        let job_ids = sqlx::query_scalar::<_, String>(
            "SELECT cron_job_id FROM cron_jobs \
             WHERE user_id = ? AND conversation_id = ? ORDER BY cron_job_id",
        )
        .bind(user_id)
        .bind(conversation_id)
        .fetch_all(&mut *tx)
        .await?;

        let reserved_run: Option<(String, String)> = sqlx::query_as(
            "SELECT reservation.cron_job_run_id, reservation.cron_job_id \
             FROM cron_run_reservations reservation \
             JOIN cron_jobs job ON job.cron_job_id = reservation.cron_job_id \
             WHERE job.user_id = ? AND job.conversation_id = ? \
               AND reservation.status = 'reserved' \
             ORDER BY reservation.created_at_ms, reservation.id LIMIT 1",
        )
        .bind(user_id)
        .bind(conversation_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((run_id, cron_job_id)) = reserved_run {
            return Err(DbError::Conflict(format!(
                "cron job '{cron_job_id}' has admitted run '{run_id}'; \
                 wait for its durable receipt to become terminal before cascade deletion"
            )));
        }

        sqlx::query(
            "DELETE FROM cron_run_reservations \
             WHERE cron_job_id IN (\
                 SELECT cron_job_id FROM cron_jobs \
                 WHERE user_id = ? AND conversation_id = ?\
             )",
        )
        .bind(user_id)
        .bind(conversation_id)
        .execute(&mut *tx)
        .await?;

        let result = sqlx::query("DELETE FROM cron_jobs WHERE user_id = ? AND conversation_id = ?")
            .bind(user_id)
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() != job_ids.len() as u64 {
            return Err(DbError::Conflict(
                "cron Session cleanup changed while the deletion fence was held".to_owned(),
            ));
        }
        tx.commit().await?;
        Ok(job_ids)
    }

    async fn list_runs_by_job(
        &self,
        user_id: &str,
        cron_job_id: &str,
        limit: i64,
    ) -> Result<Vec<CronRunHistoryRow>, DbError> {
        let limit = limit.clamp(0, CRON_RUN_HISTORY_LIMIT);
        let rows=sqlx::query_as::<_,CronRunHistoryRow>("SELECT reservation.id,reservation.cron_job_run_id,reservation.cron_job_id,reservation.settled_at_ms AS executed_at_ms,reservation.status,reservation.settled_at_ms AS created_at_ms,reservation.conversation_id,reservation.result_error,reservation.domain_failure_json FROM cron_run_reservations reservation JOIN cron_jobs job ON job.cron_job_id=reservation.cron_job_id WHERE reservation.cron_job_id=? AND job.user_id=? AND reservation.status<>'reserved' ORDER BY reservation.settled_at_ms DESC,reservation.id DESC LIMIT ?")
        .bind(cron_job_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn reserve_run(
        &self,
        user_id: &str,
        params: &ReserveCronRunParams,
    ) -> Result<(CronRunReservationRow, bool), DbError> {
        UserId::parse(user_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron owner id: {error}")))?;
        CronJobId::parse(&params.cron_job_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron job id: {error}")))?;
        CronJobRunId::parse(&params.cron_job_run_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron run id: {error}")))?;
        if params.operation_key.is_empty() || params.request_fingerprint.is_empty() {
            return Err(DbError::Conflict(
                "cron run operation key and request fingerprint must not be empty".to_owned(),
            ));
        }
        match params.trigger_kind.as_str() {
            "scheduled"
                if params
                    .schedule_revision
                    .is_some_and(|revision| revision > 0)
                    && params.planned_at_ms.is_some() => {}
            "run_now" if params.schedule_revision.is_none() && params.planned_at_ms.is_none() => {}
            _ => {
                return Err(DbError::Conflict(
                    "cron run trigger fields are inconsistent".to_owned(),
                ));
            }
        }

        if params.trigger_kind == "scheduled"
            && params
                .next_run_at
                .zip(params.planned_at_ms)
                .is_some_and(|(next, planned)| next <= planned)
        {
            return Err(DbError::Conflict(
                "scheduled successor must advance beyond the claimed occurrence".into(),
            ));
        }
        if params.trigger_kind == "scheduled" && params.disable == params.next_run_at.is_some() {
            return Err(DbError::Conflict(
                "scheduled reservation requires exactly a successor or disable".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        // The first statement is a writer and is also the authority proof.
        // For scheduled callbacks the immutable reservation can only be
        // inserted from a currently enabled job whose persisted revision and
        // exact next occurrence still match the timer's claim. This avoids the
        // SELECT -> INSERT snapshot race across independent SQLite pools.
        let inserted = if params.trigger_kind == "scheduled" {
            let schedule_revision = params
                .schedule_revision
                .expect("validated scheduled revision");
            let planned_at_ms = params
                .planned_at_ms
                .expect("validated scheduled planned time");
            sqlx::query(
                "INSERT OR IGNORE INTO cron_run_reservations (\
                     cron_job_run_id, cron_job_id, trigger_kind, operation_key, \
                     request_fingerprint, schedule_revision, planned_at_ms, status, \
                     created_at_ms, updated_at_ms, job_projection_state\
                 ) \
                 SELECT ?, job.cron_job_id, 'scheduled', ?, ?, ?, ?, 'reserved', ?, ?, 'pending' \
                 FROM cron_jobs job \
                 WHERE job.cron_job_id = ? AND job.user_id = ? AND job.enabled = 1 \
                   AND job.schedule_revision = ? AND job.next_run_at = ?",
            )
            .bind(&params.cron_job_run_id)
            .bind(&params.operation_key)
            .bind(&params.request_fingerprint)
            .bind(schedule_revision)
            .bind(planned_at_ms)
            .bind(params.now)
            .bind(params.now)
            .bind(&params.cron_job_id)
            .bind(user_id)
            .bind(schedule_revision)
            .bind(planned_at_ms)
            .execute(&mut *tx)
            .await?
            .rows_affected()
                == 1
        } else {
            sqlx::query(
                "INSERT OR IGNORE INTO cron_run_reservations (\
                     cron_job_run_id, cron_job_id, trigger_kind, operation_key, \
                     request_fingerprint, schedule_revision, planned_at_ms, status, \
                     created_at_ms, updated_at_ms, job_projection_state\
                 ) \
                 SELECT ?, job.cron_job_id, 'run_now', ?, ?, NULL, NULL, 'reserved', ?, ?, 'pending' \
                 FROM cron_jobs job \
                 WHERE job.cron_job_id = ? AND job.user_id = ?",
            )
            .bind(&params.cron_job_run_id)
            .bind(&params.operation_key)
            .bind(&params.request_fingerprint)
            .bind(params.now)
            .bind(params.now)
            .bind(&params.cron_job_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
                == 1
        };

        let row = if params.trigger_kind == "scheduled" {
            sqlx::query_as::<_, CronRunReservationRow>(
                "SELECT * FROM cron_run_reservations \
                 WHERE cron_job_id = ? AND trigger_kind = 'scheduled' \
                   AND schedule_revision = ? AND planned_at_ms = ?",
            )
            .bind(&params.cron_job_id)
            .bind(params.schedule_revision)
            .bind(params.planned_at_ms)
            .fetch_optional(&mut *tx)
            .await?
        } else {
            sqlx::query_as::<_, CronRunReservationRow>(
                "SELECT * FROM cron_run_reservations \
                 WHERE cron_job_id = ? AND trigger_kind = 'run_now' AND operation_key = ?",
            )
            .bind(&params.cron_job_id)
            .bind(&params.operation_key)
            .fetch_optional(&mut *tx)
            .await?
        };
        let Some(row) = row else {
            let job = sqlx::query_as::<_, (bool, i64, Option<i64>)>(
                "SELECT enabled, schedule_revision, next_run_at FROM cron_jobs \
                 WHERE cron_job_id = ? AND user_id = ?",
            )
            .bind(&params.cron_job_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
            if job.is_none() {
                return Err(DbError::NotFound(format!(
                    "cron job '{}'",
                    params.cron_job_id
                )));
            }
            if params.trigger_kind == "scheduled" {
                let (enabled, actual_revision, actual_next_run_at) =
                    job.expect("checked cron job presence");
                if enabled
                    && Some(actual_revision) == params.schedule_revision
                    && actual_next_run_at == params.planned_at_ms
                {
                    return Err(DbError::Conflict(
                        "cron run reservation collided with a different durable identity"
                            .to_owned(),
                    ));
                }
                return Err(DbError::Conflict(format!(
                    "scheduled occurrence is stale: enabled={enabled}, revision={actual_revision}, \
                     next_run_at={actual_next_run_at:?}, claimed_revision={}, claimed_planned_at={:?}",
                    params.schedule_revision.unwrap_or_default(),
                    params.planned_at_ms
                )));
            }
            return Err(DbError::Conflict(
                "cron run reservation collided with a different durable identity".to_owned(),
            ));
        };

        if row.trigger_kind != params.trigger_kind
            || row.operation_key != params.operation_key
            || row.request_fingerprint != params.request_fingerprint
            || row.schedule_revision != params.schedule_revision
            || row.planned_at_ms != params.planned_at_ms
        {
            return Err(DbError::Conflict(
                "cron run idempotency key was reused with a different request".to_owned(),
            ));
        }
        if params.trigger_kind == "scheduled" {
            sqlx::query("UPDATE cron_jobs SET next_run_at=?, enabled=CASE WHEN ? THEN 0 ELSE enabled END, updated_at=? WHERE cron_job_id=? AND user_id=? AND enabled=1 AND schedule_revision=? AND next_run_at=?")
                .bind(params.next_run_at).bind(params.disable).bind(params.now).bind(&params.cron_job_id)
                .bind(user_id).bind(params.schedule_revision).bind(params.planned_at_ms).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok((row, inserted))
    }

    async fn attach_continuing_session(
        &self,
        user_id: &str,
        run_id: &str,
        session: &str,
        now: i64,
    ) -> Result<CronRunReservationRow, DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE cron_run_reservations SET updated_at_ms=updated_at_ms WHERE cron_job_run_id=?",
        )
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
        let run: CronRunReservationRow=sqlx::query_as("SELECT reservation.* FROM cron_run_reservations reservation JOIN cron_jobs job ON job.cron_job_id=reservation.cron_job_id WHERE reservation.cron_job_run_id=? AND job.user_id=? AND job.execution_mode='existing'")
            .bind(run_id).bind(user_id).fetch_optional(&mut *tx).await?.ok_or_else(||DbError::NotFound("continuing Cron reservation".into()))?;
        if run.status != "reserved"
            || run.cancel_requested_at_ms.is_some()
            || run
                .conversation_id
                .as_deref()
                .is_some_and(|value| value != session)
        {
            return Err(DbError::Conflict(
                "continuing Cron reservation cannot acquire this Session".into(),
            ));
        }
        lock_owned_session_relation(&mut tx, user_id, session).await?;
        ensure_session_relation_available(&mut tx, user_id, session, Some(&run.cron_job_id))
            .await?;
        let current: Option<String> = sqlx::query_scalar(
            "SELECT conversation_id FROM cron_jobs WHERE cron_job_id=? AND user_id=?",
        )
        .bind(&run.cron_job_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if current.as_deref().is_some_and(|value| value != session) {
            return Err(DbError::Conflict(
                "continuing Cron already owns another Session".into(),
            ));
        }
        sqlx::query("UPDATE cron_run_reservations SET conversation_id=COALESCE(conversation_id,?),updated_at_ms=? WHERE cron_job_run_id=?")
            .bind(session).bind(now).bind(run_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE cron_jobs SET conversation_id=COALESCE(conversation_id,?),agent_binding=NULL,agent_selection=NULL,model_selection=NULL,updated_at=? WHERE cron_job_id=? AND user_id=?")
            .bind(session).bind(now).bind(&run.cron_job_id).bind(user_id).execute(&mut *tx).await?;
        let result = sqlx::query_as("SELECT * FROM cron_run_reservations WHERE cron_job_run_id=?")
            .bind(run_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    async fn attach_run_conversation(
        &self,
        user_id: &str,
        cron_job_run_id: &str,
        conversation_id: &str,
        now: i64,
    ) -> Result<CronRunReservationRow, DbError> {
        UserId::parse(user_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron owner id: {error}")))?;
        CronJobRunId::parse(cron_job_run_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron run id: {error}")))?;
        ConversationId::parse(conversation_id)
            .map_err(|error| DbError::Conflict(format!("invalid conversation id: {error}")))?;

        let mut tx = self.pool.begin().await?;
        let current = sqlx::query_as::<_, CronRunReservationRow>(
            "SELECT reservation.* FROM cron_run_reservations reservation \
             JOIN cron_jobs job ON job.cron_job_id = reservation.cron_job_id \
             WHERE reservation.cron_job_run_id = ? AND job.user_id = ?",
        )
        .bind(cron_job_run_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("cron run '{cron_job_run_id}'")))?;
        if let Some(existing) = current.conversation_id.as_deref()
            && existing != conversation_id
        {
            return Err(DbError::Conflict(
                "cron run was already attached to a different conversation".to_owned(),
            ));
        }
        if current.status != "reserved" && current.conversation_id.is_none() {
            return Err(DbError::Conflict(
                "settled cron run cannot acquire a conversation".to_owned(),
            ));
        }
        if current.cancel_requested_at_ms.is_some() && current.conversation_id.is_none() {
            return Err(DbError::Conflict(
                "cancelled cron run cannot acquire a conversation".into(),
            ));
        }

        sqlx::query(
            "UPDATE cron_run_reservations \
             SET conversation_id = COALESCE(conversation_id, ?), updated_at_ms = ? \
             WHERE cron_job_run_id = ?",
        )
        .bind(conversation_id)
        .bind(now)
        .bind(cron_job_run_id)
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query_as::<_, CronRunReservationRow>(
            "SELECT * FROM cron_run_reservations WHERE cron_job_run_id = ?",
        )
        .bind(cron_job_run_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn get_run_reservation(
        &self,
        user_id: &str,
        cron_job_id: &str,
        cron_job_run_id: &str,
    ) -> Result<Option<CronRunReservationRow>, DbError> {
        UserId::parse(user_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron owner id: {error}")))?;
        CronJobId::parse(cron_job_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron job id: {error}")))?;
        CronJobRunId::parse(cron_job_run_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron run id: {error}")))?;
        Ok(sqlx::query_as::<_, CronRunReservationRow>(
            "SELECT reservation.* FROM cron_run_reservations reservation \
             LEFT JOIN cron_jobs job ON job.cron_job_id=reservation.cron_job_id \
             LEFT JOIN agent_sessions session ON session.agent_session_id=reservation.conversation_id \
             WHERE (job.user_id=? OR (job.cron_job_id IS NULL AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
               AND json_extract(session.owner_ref_json,'$.principal_id')=?)) AND reservation.cron_job_id=? AND reservation.cron_job_run_id=?")
            .bind(user_id).bind(user_id).bind(cron_job_id).bind(cron_job_run_id).fetch_optional(&self.pool).await?)
    }

    async fn request_run_cancellation(
        &self,
        user_id: &str,
        cron_job_id: &str,
        cron_job_run_id: &str,
        now: i64,
    ) -> Result<CronRunReservationRow, DbError> {
        UserId::parse(user_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron owner id: {error}")))?;
        CronJobId::parse(cron_job_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron job id: {error}")))?;
        CronJobRunId::parse(cron_job_run_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron run id: {error}")))?;
        let mut tx = self.pool.begin().await?;
        // Acquire the writer before reading: attachment/terminalization and
        // cancellation must have one SQLite winner even across processes.
        sqlx::query("UPDATE cron_run_reservations SET cancel_requested_at_ms=COALESCE(cancel_requested_at_ms,?),updated_at_ms=? \
            WHERE cron_job_run_id=? AND cron_job_id=? AND status='reserved' \
              AND (EXISTS(SELECT 1 FROM cron_jobs job WHERE job.cron_job_id=cron_run_reservations.cron_job_id AND job.user_id=?) \
                OR (NOT EXISTS(SELECT 1 FROM cron_jobs job WHERE job.cron_job_id=cron_run_reservations.cron_job_id) \
                  AND EXISTS(SELECT 1 FROM agent_sessions session WHERE session.agent_session_id=cron_run_reservations.conversation_id \
                    AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=?)))")
            .bind(now).bind(now).bind(cron_job_run_id).bind(cron_job_id).bind(user_id).bind(user_id).execute(&mut *tx).await?;
        let row = sqlx::query_as::<_, CronRunReservationRow>(
            "SELECT reservation.* FROM cron_run_reservations reservation \
             LEFT JOIN cron_jobs job ON job.cron_job_id=reservation.cron_job_id \
             LEFT JOIN agent_sessions session ON session.agent_session_id=reservation.conversation_id \
             WHERE (job.user_id=? OR (job.cron_job_id IS NULL AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
               AND json_extract(session.owner_ref_json,'$.principal_id')=?)) AND reservation.cron_job_id=? AND reservation.cron_job_run_id=?")
            .bind(user_id).bind(user_id).bind(cron_job_id).bind(cron_job_run_id).fetch_optional(&mut *tx).await?
            .ok_or_else(|| DbError::NotFound(format!("cron run '{cron_job_run_id}'")))?;
        tx.commit().await?;
        Ok(row)
    }

    async fn attached_run_owner_for_scheduler(
        &self,
        cron_job_run_id: &str,
    ) -> Result<Option<String>, DbError> {
        let owner:Option<Option<String>>=sqlx::query_scalar("SELECT CASE WHEN job.cron_job_id IS NOT NULL THEN job.user_id \
            WHEN json_extract(session.owner_ref_json,'$.principal_kind')='user' THEN json_extract(session.owner_ref_json,'$.principal_id') ELSE NULL END \
            FROM cron_run_reservations reservation LEFT JOIN cron_jobs job ON job.cron_job_id=reservation.cron_job_id \
            JOIN agent_sessions session ON session.agent_session_id=reservation.conversation_id WHERE reservation.cron_job_run_id=?")
            .bind(cron_job_run_id).fetch_optional(&self.pool).await?;
        Ok(owner.flatten())
    }

    async fn finalize_run_with_job_projection(
        &self,
        user_id: &str,
        params: &FinalizeCronRunParams,
    ) -> Result<FinalizeCronRunOutcome, DbError> {
        UserId::parse(user_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron owner id: {error}")))?;
        CronJobRunId::parse(&params.cron_job_run_id)
            .map_err(|error| DbError::Conflict(format!("invalid cron run id: {error}")))?;
        if !matches!(
            params.status.as_str(),
            "ok" | "error" | "skipped" | "missed" | "input_required" | "cancelled"
        ) {
            return Err(DbError::Conflict(format!(
                "invalid cron run terminal status '{}'",
                params.status
            )));
        }
        if let Some(last_status) = params.last_status.as_deref()
            && !matches!(
                last_status,
                "ok" | "error" | "skipped" | "missed" | "input_required" | "cancelled"
            )
        {
            return Err(DbError::Conflict(format!(
                "invalid cron job projected status '{last_status}'"
            )));
        }
        if let Some(conversation_id) = params.conversation_id.as_deref() {
            ConversationId::parse(conversation_id)
                .map_err(|error| DbError::Conflict(format!("invalid conversation id: {error}")))?;
        }

        let mut tx = self.pool.begin().await?;
        let current = sqlx::query_as::<_, CronRunReservationRow>(
            "SELECT reservation.* FROM cron_run_reservations reservation \
             LEFT JOIN cron_jobs job ON job.cron_job_id = reservation.cron_job_id \
             LEFT JOIN agent_sessions session ON session.agent_session_id=reservation.conversation_id \
             WHERE reservation.cron_job_run_id = ? AND (job.user_id = ? OR (job.cron_job_id IS NULL \
               AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=?))",
        )
        .bind(&params.cron_job_run_id)
        .bind(user_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("cron run '{}'", params.cron_job_run_id)))?;
        if current.status != "reserved" {
            let same_terminal_outcome = current.job_projection_state == "applied"
                && current.status == params.status
                && params
                    .conversation_id
                    .as_ref()
                    .is_none_or(|conversation_id| {
                        current.conversation_id.as_ref() == Some(conversation_id)
                    })
                && current.result_error == params.result_error
                && current.domain_failure_json == params.domain_failure_json;
            if !same_terminal_outcome {
                return Err(DbError::Conflict(
                    "cron run was already finalized with a different durable outcome".to_owned(),
                ));
            }
            tx.commit().await?;
            return Ok(FinalizeCronRunOutcome::AlreadyApplied);
        }
        if current.job_projection_state != "pending" {
            tx.commit().await?;
            return Ok(FinalizeCronRunOutcome::LegacyProjectionUnknown);
        }
        if current.cancel_requested_at_ms.is_some() && params.status != "cancelled" {
            // An old preparation callback cannot turn a user-cancelled,
            // not-yet-claimed invocation into a scheduler failure or retry.
            let terminal: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns turn WHERE turn.session_id=? \
                  AND turn.operation_id='turn:user:'||?||':'||turn.session_id||':cron:'||?||':turn' \
                  AND turn.state IN ('completed','failed','cancelled'))")
                .bind(&current.conversation_id).bind(user_id)
                .bind(&current.cron_job_run_id).fetch_one(&mut *tx).await?;
            if !terminal {
                return Err(DbError::Conflict(
                    "cancelled Cron admission awaits its exact cancellation settlement".into(),
                ));
            }
        }
        if let (Some(existing), Some(requested)) = (
            current.conversation_id.as_deref(),
            params.conversation_id.as_deref(),
        ) && existing != requested
        {
            return Err(DbError::Conflict(
                "cron run finalization targets a different conversation".to_owned(),
            ));
        }

        let settled = sqlx::query(
            "UPDATE cron_run_reservations \
             SET status = ?, conversation_id = COALESCE(conversation_id, ?), \
                 result_error = ?, domain_failure_json = ?, updated_at_ms = ?, settled_at_ms = ?, \
                 job_projection_state = 'applied', job_projected_at_ms = ? \
             WHERE cron_job_run_id = ? AND status = 'reserved' \
               AND job_projection_state = 'pending'",
        )
        .bind(&params.status)
        .bind(&params.conversation_id)
        .bind(&params.result_error)
        .bind(&params.domain_failure_json)
        .bind(params.now)
        .bind(params.now)
        .bind(params.now)
        .bind(&params.cron_job_run_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !settled {
            tx.rollback().await?;
            return Ok(FinalizeCronRunOutcome::AlreadyApplied);
        }
        let job_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE cron_job_id=?)")
                .bind(&current.cron_job_id)
                .fetch_one(&mut *tx)
                .await?;
        if !job_exists {
            // Only the exact attached Session supplied ownership above. No
            // deleted job is recreated and no later schedule is projected.
            tx.commit().await?;
            return Ok(FinalizeCronRunOutcome::Applied);
        }

        let project_last_run_at = params.last_run_at.is_some();
        let project_last_status = params.last_status.is_some();
        let project_last_error = params.last_error.is_some();
        let last_error = params.last_error.clone().flatten();
        let updated = sqlx::query(
            "UPDATE cron_jobs \
             SET last_run_at = CASE WHEN ? THEN ? ELSE last_run_at END, \
                 last_status = CASE WHEN ? THEN ? ELSE last_status END, \
                 last_run_id = CASE WHEN ? THEN ? ELSE last_run_id END, \
                 last_error = CASE WHEN ? THEN ? ELSE last_error END, \
                 run_count = run_count + ?, \
                 updated_at = ? \
             WHERE cron_job_id = ? AND user_id = ?",
        )
        .bind(project_last_run_at)
        .bind(params.last_run_at)
        .bind(project_last_status)
        .bind(&params.last_status)
        .bind(project_last_status || project_last_run_at)
        .bind(&params.cron_job_run_id)
        .bind(project_last_error)
        .bind(last_error)
        .bind(i64::from(params.increment_run_count))
        .bind(params.now)
        .bind(&current.cron_job_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(DbError::NotFound(format!(
                "cron job '{}'",
                current.cron_job_id
            )));
        }

        tx.commit().await?;
        Ok(FinalizeCronRunOutcome::Applied)
    }

    async fn list_reserved_runs_for_scheduler(
        &self,
    ) -> Result<Vec<CronRunReservationRow>, DbError> {
        Ok(sqlx::query_as::<_, CronRunReservationRow>(
            "SELECT reservation.* FROM cron_run_reservations reservation \
             LEFT JOIN cron_jobs job ON job.cron_job_id = reservation.cron_job_id \
             WHERE reservation.status = 'reserved' \
             ORDER BY reservation.created_at_ms, reservation.id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn get_scheduled_run_reservation(
        &self,
        user_id: &str,
        cron_job_id: &str,
        schedule_revision: i64,
        planned_at_ms: i64,
    ) -> Result<Option<CronRunReservationRow>, DbError> {
        Ok(sqlx::query_as::<_, CronRunReservationRow>(
            "SELECT reservation.* FROM cron_run_reservations reservation \
             JOIN cron_jobs job ON job.cron_job_id = reservation.cron_job_id \
             WHERE reservation.cron_job_id = ? AND job.user_id = ? \
               AND reservation.trigger_kind = 'scheduled' \
               AND reservation.schedule_revision = ? AND reservation.planned_at_ms = ?",
        )
        .bind(cron_job_id)
        .bind(user_id)
        .bind(schedule_revision)
        .bind(planned_at_ms)
        .fetch_optional(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    const CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const OTHER_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678902";
    const MISSING_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678903";
    const CANONICAL_SESSION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678906";
    const DELETED_CANONICAL_SESSION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678907";
    const MISSING_CRON_JOB_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678999";

    async fn setup() -> (SqliteCronRepository, crate::Database, String) {
        let db = init_database_memory().await.expect("init db");
        let installation_owner = crate::installation_owner_id(db.pool()).await.unwrap();
        let repo = SqliteCronRepository::new(db.pool().clone());

        // General Cron repository tests exercise the complete host-capable
        // shape, so their target is a live installation-owned AgentSession.
        insert_canonical_session(&db, CONVERSATION_ID, &installation_owner, "live").await;

        (repo, db, installation_owner)
    }

    fn make_row(installation_owner: &str) -> CronJobRow {
        let now = now_ms();
        CronJobRow {
            id: 0,
            cron_job_id: nomifun_common::CronJobId::new().into_string(),
            user_id: installation_owner.to_owned(),
            name: "Test Job".into(),
            enabled: true,
            schedule_revision: 1,
            schedule_kind: "every".into(),
            schedule_value: "60000".into(),
            schedule_tz: None,
            schedule_description: Some("Every minute".into()),
            payload_message: "ping".into(),
            execution_mode: "existing".into(),
            agent_selection: None,
            model_selection: None,
            agent_binding: None,
            clear_context_each_run: false,
            configuration_error: None,

            conversation_id: Some(CONVERSATION_ID.into()),
            conversation_title: Some("Test Conv".into()),

            created_by: "user".into(),
            skill_content: None,
            description: None,
            created_at: now,
            updated_at: now,
            next_run_at: Some(now + 60_000),
            last_run_at: None,
            last_run_id: None,
            last_status: None,
            last_error: None,
            run_count: 0,
        }
    }

    async fn insert_canonical_session(
        db: &crate::Database,
        agent_session_id: &str,
        owner_id: &str,
        state: &str,
    ) {
        let owner_ref = serde_json::json!({
            "principal_kind": "user",
            "principal_id": owner_id,
        })
        .to_string();
        match state {
            "live" => {
                sqlx::query(
                    "INSERT INTO agent_sessions (\
                        agent_session_id, owner_ref_json, state, title, archived, pinned, \
                        agent_binding_json, next_seq, created_at\
                     ) VALUES (?, ?, 'live', 'Canonical', 0, 0, '{}', 1, 1)",
                )
                .bind(agent_session_id)
                .bind(owner_ref)
                .execute(db.pool())
                .await
                .unwrap();
            }
            "deleted" => {
                sqlx::query(
                    "INSERT INTO agent_sessions (\
                        agent_session_id, owner_ref_json, state, deleted_at\
                     ) VALUES (?, ?, 'deleted', 1)",
                )
                .bind(agent_session_id)
                .bind(owner_ref)
                .execute(db.pool())
                .await
                .unwrap();
            }
            other => panic!("unsupported canonical Session fixture state {other}"),
        }
    }

    #[tokio::test]
    async fn atomic_relation_supports_store_only_session_and_rejects_second_job() {
        let (repo, db, owner) = setup().await;
        insert_canonical_session(&db, CANONICAL_SESSION_ID, &owner, "live").await;

        let mut first = make_row(&owner);
        first.conversation_id = Some(CANONICAL_SESSION_ID.into());
        first.conversation_title = Some("Canonical".into());
        let first_job_id = first.cron_job_id.clone();
        repo.insert_with_session_relation(&first).await.unwrap();

        assert_eq!(
            repo.list_by_conversation(&owner, CANONICAL_SESSION_ID)
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.cron_job_id)
                .collect::<Vec<_>>(),
            vec![first_job_id]
        );

        let mut second = make_row(&owner);
        second.conversation_id = Some(CANONICAL_SESSION_ID.into());
        let error = repo
            .insert_with_session_relation(&second)
            .await
            .unwrap_err();
        assert!(
            matches!(error, DbError::Conflict(ref message) if message.contains("already bound")),
            "unexpected duplicate relation error: {error:?}"
        );
        assert_eq!(repo.list_all(&owner).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn atomic_relation_fails_closed_for_wrong_or_deleted_canonical_owner_state() {
        let (repo, db, owner) = setup().await;
        insert_canonical_session(&db, CANONICAL_SESSION_ID, &owner, "live").await;
        let mut wrong_owner = make_row(&owner);
        wrong_owner.user_id = "0190f5fe-7c00-7a00-8000-000000000099".into();
        wrong_owner.conversation_id = Some(CANONICAL_SESSION_ID.into());
        let error = repo
            .insert_with_session_relation(&wrong_owner)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("owner mismatch"));

        insert_canonical_session(&db, DELETED_CANONICAL_SESSION_ID, &owner, "deleted").await;
        let mut deleted = make_row(&owner);
        deleted.conversation_id = Some(DELETED_CANONICAL_SESSION_ID.into());
        let error = repo
            .insert_with_session_relation(&deleted)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not live"));
    }

    #[tokio::test]
    async fn input_handoff_and_cancellation_settle_once_without_failure_or_a_permanent_reservation()
    {
        for status in ["input_required", "cancelled"] {
            let (repo, _db, owner) = setup().await;
            let job = make_row(&owner);
            repo.insert(&job).await.unwrap();
            let run = nomifun_common::CronJobRunId::new().into_string();
            let params = ReserveCronRunParams {
                cron_job_run_id: run.clone(),
                cron_job_id: job.cron_job_id.clone(),
                trigger_kind: "run_now".into(),
                operation_key: format!("handoff:{run}"),
                request_fingerprint: "exact-input".into(),
                schedule_revision: None,
                planned_at_ms: None,
                next_run_at: None,
                disable: false,
                now: 1_000,
            };
            repo.reserve_run(&owner, &params).await.unwrap();
            assert!(repo.delete(&owner, &job.cron_job_id).await.is_err());
            let terminal = FinalizeCronRunParams {
                cron_job_run_id: run,
                status: status.into(),
                conversation_id: None,
                result_error: None,
                domain_failure_json: None,
                now: 1_001,
                last_run_at: Some(1_001),
                last_status: Some(status.into()),
                last_error: Some(None),
                increment_run_count: true,
            };
            assert_eq!(
                repo.finalize_run_with_job_projection(&owner, &terminal)
                    .await
                    .unwrap(),
                FinalizeCronRunOutcome::Applied
            );
            assert_eq!(
                repo.finalize_run_with_job_projection(&owner, &terminal)
                    .await
                    .unwrap(),
                FinalizeCronRunOutcome::AlreadyApplied
            );
            let saved = repo
                .get_by_cron_job_id(&owner, &job.cron_job_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.last_status.as_deref(), Some(status));
            assert_eq!(saved.last_error, None);
            assert_eq!(saved.run_count, 1);
            assert_eq!(
                repo.list_runs_by_job(&owner, &job.cron_job_id, 7)
                    .await
                    .unwrap()[0]
                    .status,
                status
            );
            assert!(
                repo.list_reserved_runs_for_scheduler()
                    .await
                    .unwrap()
                    .is_empty()
            );
            repo.delete(&owner, &job.cron_job_id).await.unwrap();
        }
    }

    #[tokio::test]
    async fn exact_run_cancellation_preserves_source_and_fences_attachment_claim_resume_and_old_callbacks()
     {
        let (repo, db, owner) = setup().await;
        let job = make_row(&owner);
        repo.insert(&job).await.unwrap();
        let other = make_row(&owner);
        repo.insert(&other).await.unwrap();
        let run = CronJobRunId::new().into_string();
        let original = ReserveCronRunParams {
            cron_job_run_id: run.clone(),
            cron_job_id: job.cron_job_id.clone(),
            trigger_kind: "run_now".into(),
            operation_key: format!("cancel-source:{run}"),
            request_fingerprint: "immutable-source".into(),
            schedule_revision: None,
            planned_at_ms: None,
            next_run_at: None,
            disable: false,
            now: 1_000,
        };
        repo.reserve_run(&owner, &original).await.unwrap();
        let foreign = CronJobRunId::new().into_string();
        assert!(
            repo.request_run_cancellation(&foreign, &job.cron_job_id, &run, 1_001)
                .await
                .is_err()
        );
        assert!(
            repo.request_run_cancellation(&owner, &other.cron_job_id, &run, 1_001)
                .await
                .is_err()
        );
        assert_eq!(
            repo.get_run_reservation(&owner, &job.cron_job_id, &run)
                .await
                .unwrap()
                .unwrap()
                .cancel_requested_at_ms,
            None
        );
        let stopped = repo
            .request_run_cancellation(&owner, &job.cron_job_id, &run, 1_002)
            .await
            .unwrap();
        assert_eq!(stopped.cancel_requested_at_ms, Some(1_002));
        assert_eq!(stopped.request_fingerprint, original.request_fingerprint);
        assert_eq!(stopped.operation_key, original.operation_key);
        assert_eq!(
            repo.request_run_cancellation(&owner, &job.cron_job_id, &run, 1_003)
                .await
                .unwrap()
                .cancel_requested_at_ms,
            Some(1_002)
        );
        assert!(
            repo.attach_run_conversation(&owner, &run, CONVERSATION_ID, 1_004)
                .await
                .is_err()
        );
        let error = FinalizeCronRunParams {
            cron_job_run_id: run.clone(),
            status: "error".into(),
            conversation_id: None,
            result_error: Some("late old preparation error".into()),
            domain_failure_json: None,
            now: 1_005,
            last_run_at: Some(1_005),
            last_status: Some("error".into()),
            last_error: Some(Some("late old preparation error".into())),
            increment_run_count: true,
        };
        assert!(
            repo.finalize_run_with_job_projection(&owner, &error)
                .await
                .is_err()
        );
        // An already-attached sibling proves the last native writer, not just
        // a Cron preflight read, consumes the same durable cancellation fact.
        let sibling = CronJobRunId::new().into_string();
        let original_sibling = ReserveCronRunParams {
            cron_job_run_id: sibling.clone(),
            operation_key: format!("cancel-source:{sibling}"),
            ..original.clone()
        };
        repo.reserve_run(&owner, &original_sibling).await.unwrap();
        repo.attach_run_conversation(&owner, &sibling, CONVERSATION_ID, 1_006)
            .await
            .unwrap();
        let operation = format!("turn:user:{owner}:{CONVERSATION_ID}:cron:{sibling}:turn");
        let mut tx = db.pool().begin().await.unwrap();
        assert!(
            crate::native_execution_claim_admitted_tx(&mut tx, &owner, CONVERSATION_ID, &operation)
                .await
                .unwrap()
        );
        tx.commit().await.unwrap();
        repo.request_run_cancellation(&owner, &job.cron_job_id, &sibling, 1_007)
            .await
            .unwrap();
        let restarted = SqliteCronRepository::new(db.pool().clone());
        assert_eq!(
            restarted
                .get_run_reservation(&owner, &job.cron_job_id, &sibling)
                .await
                .unwrap()
                .unwrap()
                .cancel_requested_at_ms,
            Some(1_007)
        );
        let mut tx = db.pool().begin().await.unwrap();
        assert!(
            crate::native_execution_claim_admitted_tx(&mut tx, &owner, CONVERSATION_ID, &operation)
                .await
                .is_err()
        );
        assert!(
            crate::native_resume_capacity_available_tx(
                &mut tx,
                &owner,
                CONVERSATION_ID,
                &operation
            )
            .await
            .is_err()
        );
        assert!(
            !crate::native_resume_target_eligible_tx(&mut tx, &owner, CONVERSATION_ID, &operation)
                .await
                .unwrap()
        );
        assert!(
            crate::native_execution_claim_admitted_tx(
                &mut tx,
                &owner,
                CONVERSATION_ID,
                "ordinary-turn"
            )
            .await
            .unwrap()
        );
        tx.commit().await.unwrap();
        let saved = repo
            .get_by_cron_job_id(&owner, &job.cron_job_id)
            .await
            .unwrap()
            .unwrap();
        assert!(saved.enabled);
        assert_eq!(
            (saved.schedule_revision, saved.next_run_at, saved.run_count),
            (job.schedule_revision, job.next_run_at, job.run_count)
        );
    }

    #[tokio::test]
    async fn insert_and_get_by_cron_job_id() {
        let (repo, _db, owner) = setup().await;
        let row = make_row(&owner);
        let cron_job_id = row.cron_job_id.clone();
        repo.insert(&row).await.unwrap();

        let found = repo
            .get_by_cron_job_id(&owner, &cron_job_id)
            .await
            .unwrap()
            .expect("found");
        assert!(found.id > 0);
        assert_eq!(found.cron_job_id, cron_job_id);
        assert_eq!(found.name, "Test Job");
        assert!(found.enabled);
        assert_eq!(found.schedule_kind, "every");
        assert_eq!(found.run_count, 0);
    }

    #[tokio::test]
    async fn get_by_cron_job_id_returns_none_for_missing() {
        let (repo, _db, owner) = setup().await;
        let result = repo
            .get_by_cron_job_id(&owner, MISSING_CRON_JOB_ID)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_all_returns_all_rows() {
        let (repo, _db, owner) = setup().await;
        repo.insert(&make_row(&owner)).await.unwrap();
        repo.insert(&make_row(&owner)).await.unwrap();

        let all = repo.list_all(&owner).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_enabled_filters_disabled() {
        let (repo, _db, owner) = setup().await;
        let enabled_row = make_row(&owner);
        let enabled_id = enabled_row.cron_job_id.clone();
        repo.insert(&enabled_row).await.unwrap();

        let mut disabled = make_row(&owner);
        disabled.enabled = false;
        repo.insert(&disabled).await.unwrap();

        let enabled = repo.list_enabled_for_scheduler().await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert!(enabled[0].id > 0);
        assert_eq!(enabled[0].cron_job_id, enabled_id);
    }

    #[tokio::test]
    async fn list_by_conversation_filters_correctly() {
        let (repo, db, owner) = setup().await;
        insert_canonical_session(&db, OTHER_CONVERSATION_ID, &owner, "live").await;

        let conv1_job = make_row(&owner);
        let conv1_job_id = conv1_job.cron_job_id.clone();
        repo.insert(&conv1_job).await.unwrap();
        let mut other = make_row(&owner);
        other.conversation_id = Some(OTHER_CONVERSATION_ID.into());
        let conv2_job_id = other.cron_job_id.clone();
        repo.insert(&other).await.unwrap();

        let conv1_jobs = repo
            .list_by_conversation(&owner, CONVERSATION_ID)
            .await
            .unwrap();
        assert_eq!(conv1_jobs.len(), 1);
        assert_eq!(conv1_jobs[0].cron_job_id, conv1_job_id);

        let conv2_jobs = repo
            .list_by_conversation(&owner, OTHER_CONVERSATION_ID)
            .await
            .unwrap();
        assert_eq!(conv2_jobs.len(), 1);
        assert_eq!(conv2_jobs[0].cron_job_id, conv2_job_id);
    }

    #[tokio::test]
    async fn update_partial_fields() {
        let (repo, _db, owner) = setup().await;
        let row = make_row(&owner);
        let cron_job_id = row.cron_job_id.clone();
        repo.insert(&row).await.unwrap();

        let params = UpdateCronJobParams {
            name: Some("Renamed".into()),
            enabled: Some(false),
            run_count: Some(42),
            ..Default::default()
        };
        repo.update(&owner, &cron_job_id, &params).await.unwrap();

        let updated = repo
            .get_by_cron_job_id(&owner, &cron_job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "Renamed");
        assert!(!updated.enabled);
        assert_eq!(updated.run_count, 42);
        assert!(updated.updated_at >= updated.created_at);
    }

    #[tokio::test]
    async fn update_optional_nullable_fields() {
        let (repo, _db, owner) = setup().await;
        let row = make_row(&owner);
        let cron_job_id = row.cron_job_id.clone();
        repo.insert(&row).await.unwrap();

        let params = UpdateCronJobParams {
            last_status: Some(Some("ok".into())),
            last_error: Some(Some("timeout".into())),
            skill_content: Some(Some("---\nname: skill\n---\nDo it".into())),
            ..Default::default()
        };
        repo.update(&owner, &cron_job_id, &params).await.unwrap();

        let updated = repo
            .get_by_cron_job_id(&owner, &cron_job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.last_status.as_deref(), Some("ok"));
        assert_eq!(updated.last_error.as_deref(), Some("timeout"));
        assert!(updated.skill_content.is_some());

        let clear_params = UpdateCronJobParams {
            last_status: Some(None),
            last_error: Some(None),
            skill_content: Some(None),
            ..Default::default()
        };
        repo.update(&owner, &cron_job_id, &clear_params)
            .await
            .unwrap();

        let cleared = repo
            .get_by_cron_job_id(&owner, &cron_job_id)
            .await
            .unwrap()
            .unwrap();
        assert!(cleared.last_status.is_none());
        assert!(cleared.last_error.is_none());
        assert!(cleared.skill_content.is_none());
    }

    #[tokio::test]
    async fn update_nonexistent_returns_not_found() {
        let (repo, _db, owner) = setup().await;
        let params = UpdateCronJobParams {
            name: Some("x".into()),
            ..Default::default()
        };
        let err = repo
            .update(&owner, MISSING_CRON_JOB_ID, &params)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_empty_params_is_noop() {
        let (repo, _db, owner) = setup().await;
        let row = make_row(&owner);
        let cron_job_id = row.cron_job_id.clone();
        repo.insert(&row).await.unwrap();

        let before = repo
            .get_by_cron_job_id(&owner, &cron_job_id)
            .await
            .unwrap()
            .unwrap();
        repo.update(&owner, &cron_job_id, &UpdateCronJobParams::default())
            .await
            .unwrap();
        let after = repo
            .get_by_cron_job_id(&owner, &cron_job_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(before.updated_at, after.updated_at);
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let (repo, db, owner) = setup().await;
        let row = make_row(&owner);
        let cron_job_id = row.cron_job_id.clone();
        repo.insert(&row).await.unwrap();

        repo.delete(&owner, &cron_job_id).await.unwrap();
        let result = repo.get_by_cron_job_id(&owner, &cron_job_id).await.unwrap();
        assert!(result.is_none());
        let run_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cron_run_reservations WHERE cron_job_id = ?")
                .bind(&cron_job_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(run_count, 0);
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        let (repo, _db, owner) = setup().await;
        let err = repo.delete(&owner, MISSING_CRON_JOB_ID).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_by_conversation_removes_all_related() {
        let (repo, _db, owner) = setup().await;
        repo.insert(&make_row(&owner)).await.unwrap();
        repo.insert(&make_row(&owner)).await.unwrap();

        let deleted = repo
            .delete_by_conversation(&owner, CONVERSATION_ID)
            .await
            .unwrap();
        assert_eq!(deleted.len(), 2);
        assert!(deleted.windows(2).all(|pair| pair[0] < pair[1]));

        let remaining = repo.list_all(&owner).await.unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn delete_by_conversation_returns_zero_for_no_match() {
        let (repo, _db, owner) = setup().await;
        let deleted = repo
            .delete_by_conversation(&owner, MISSING_CONVERSATION_ID)
            .await
            .unwrap();
        assert!(deleted.is_empty());
    }

    #[tokio::test]
    async fn update_schedule_fields() {
        let (repo, _db, owner) = setup().await;
        let row = make_row(&owner);
        let cron_job_id = row.cron_job_id.clone();
        repo.insert(&row).await.unwrap();

        let params = UpdateCronJobParams {
            schedule_kind: Some("cron".into()),
            schedule_value: Some("0 0 9 * * *".into()),
            schedule_tz: Some(Some("Asia/Shanghai".into())),
            schedule_description: Some(Some("Daily at 9am".into())),
            next_run_at: Some(Some(9999999)),
            ..Default::default()
        };
        repo.update(&owner, &cron_job_id, &params).await.unwrap();

        let updated = repo
            .get_by_cron_job_id(&owner, &cron_job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.schedule_kind, "cron");
        assert_eq!(updated.schedule_value, "0 0 9 * * *");
        assert_eq!(updated.schedule_tz.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(updated.next_run_at, Some(9999999));
    }

    #[tokio::test]
    async fn insert_all_schedule_kinds() {
        let (repo, _db, owner) = setup().await;

        let mut at_job = make_row(&owner);
        at_job.schedule_kind = "at".into();
        at_job.schedule_value = "1700000000000".into();
        repo.insert(&at_job).await.unwrap();

        let mut cron_job = make_row(&owner);
        cron_job.schedule_kind = "cron".into();
        cron_job.schedule_value = "0 */5 * * * *".into();
        cron_job.schedule_tz = Some("UTC".into());
        repo.insert(&cron_job).await.unwrap();

        let all = repo.list_all(&owner).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn insert_with_skill_content() {
        let (repo, _db, owner) = setup().await;
        let mut row = make_row(&owner);
        row.skill_content = Some("---\nname: My Skill\ndescription: A test\n---\nDo X".into());
        let cron_job_id = row.cron_job_id.clone();
        repo.insert(&row).await.unwrap();

        let found = repo
            .get_by_cron_job_id(&owner, &cron_job_id)
            .await
            .unwrap()
            .unwrap();
        assert!(found.skill_content.unwrap().contains("My Skill"));
    }

    fn scheduled(job: &CronJobRow, planned: i64) -> ReserveCronRunParams {
        ReserveCronRunParams {
            cron_job_run_id: CronJobRunId::new().into_string(),
            cron_job_id: job.cron_job_id.clone(),
            trigger_kind: "scheduled".into(),
            operation_key: format!(
                "cron:scheduled:{}:{}:{planned}",
                job.cron_job_id, job.schedule_revision
            ),
            request_fingerprint: format!(
                "scheduled:v1:{}:{}:{planned}",
                job.cron_job_id, job.schedule_revision
            ),
            schedule_revision: Some(job.schedule_revision),
            planned_at_ms: Some(planned),
            next_run_at: Some(planned + 60000),
            disable: false,
            now: planned,
        }
    }

    fn completed(run: &str, now: i64) -> FinalizeCronRunParams {
        FinalizeCronRunParams {
            cron_job_run_id: run.into(),
            status: "ok".into(),
            conversation_id: None,
            result_error: None,
            domain_failure_json: None,
            now,
            last_run_at: Some(now),
            last_status: Some("ok".into()),
            last_error: Some(None),
            increment_run_count: true,
        }
    }

    #[tokio::test]
    async fn scheduled_claim_advances_atomically_while_the_original_run_is_still_reserved() {
        let (repo, db, owner) = setup().await;
        let mut job = make_row(&owner);
        job.next_run_at = Some(60000);
        repo.insert(&job).await.unwrap();
        let first = scheduled(&job, 60000);
        let (original, inserted) = repo.reserve_run(&owner, &first).await.unwrap();
        assert!(inserted);
        assert_eq!(
            repo.get_by_cron_job_id(&owner, &job.cron_job_id)
                .await
                .unwrap()
                .unwrap()
                .next_run_at,
            Some(120000)
        );
        let second = scheduled(&job, 120000);
        let (_, inserted) = repo.reserve_run(&owner, &second).await.unwrap();
        assert!(inserted);
        assert_eq!(
            repo.get_run_reservation(&owner, &job.cron_job_id, &original.cron_job_run_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "reserved"
        );
        repo.finalize_run_with_job_projection(&owner, &completed(&first.cron_job_run_id, 125000))
            .await
            .unwrap();
        assert_eq!(
            repo.get_by_cron_job_id(&owner, &job.cron_job_id)
                .await
                .unwrap()
                .unwrap()
                .next_run_at,
            Some(180000),
            "late terminal cannot move a successor schedule"
        );
        assert!(!repo.reserve_run(&owner, &first).await.unwrap().1);
        assert_eq!(
            repo.get_by_cron_job_id(&owner, &job.cron_job_id)
                .await
                .unwrap()
                .unwrap()
                .next_run_at,
            Some(180000)
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cron_run_reservations")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn terminal_history_is_bounded_without_pruning_idempotency_evidence() {
        let (repo, db, owner) = setup().await;
        let job = make_row(&owner);
        repo.insert(&job).await.unwrap();
        for index in 0..10 {
            let id = CronJobRunId::new().into_string();
            repo.reserve_run(
                &owner,
                &ReserveCronRunParams {
                    cron_job_run_id: id.clone(),
                    cron_job_id: job.cron_job_id.clone(),
                    trigger_kind: "run_now".into(),
                    operation_key: format!("history:{index}"),
                    request_fingerprint: "history".into(),
                    schedule_revision: None,
                    planned_at_ms: None,
                    next_run_at: None,
                    disable: false,
                    now: index,
                },
            )
            .await
            .unwrap();
            let finalizer = completed(&id, index + 100);
            repo.finalize_run_with_job_projection(&owner, &finalizer)
                .await
                .unwrap();
            assert_eq!(
                repo.finalize_run_with_job_projection(&owner, &finalizer)
                    .await
                    .unwrap(),
                FinalizeCronRunOutcome::AlreadyApplied
            );
        }
        let history = repo
            .list_runs_by_job(&owner, &job.cron_job_id, 100)
            .await
            .unwrap();
        assert_eq!(history.len(), 7);
        assert_eq!(history[0].executed_at_ms, 109);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cron_run_reservations")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            10
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='cron_job_runs'"
            )
            .fetch_one(db.pool())
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            repo.get_by_cron_job_id(&owner, &job.cron_job_id)
                .await
                .unwrap()
                .unwrap()
                .run_count,
            10
        );
    }

    #[tokio::test]
    async fn continuing_session_is_fixed_before_turn_and_cannot_be_replaced_or_attached_after_stop()
    {
        let (repo, db, owner) = setup().await;
        let mut job = make_row(&owner);
        job.enabled = false;
        job.conversation_id = None;
        repo.insert(&job).await.unwrap();
        let id = CronJobRunId::new().into_string();
        let request = ReserveCronRunParams {
            cron_job_run_id: id.clone(),
            cron_job_id: job.cron_job_id.clone(),
            trigger_kind: "run_now".into(),
            operation_key: "first-opening".into(),
            request_fingerprint: "first".into(),
            schedule_revision: None,
            planned_at_ms: None,
            next_run_at: None,
            disable: false,
            now: 1,
        };
        repo.reserve_run(&owner, &request).await.unwrap();
        repo.attach_continuing_session(&owner, &id, CONVERSATION_ID, 2)
            .await
            .unwrap();
        let bound = repo
            .get_by_cron_job_id(&owner, &job.cron_job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bound.conversation_id.as_deref(), Some(CONVERSATION_ID));
        assert!(bound.agent_binding.is_none());
        insert_canonical_session(&db, OTHER_CONVERSATION_ID, &owner, "live").await;
        assert!(
            repo.attach_continuing_session(&owner, &id, OTHER_CONVERSATION_ID, 3)
                .await
                .is_err()
        );
        let stopped = CronJobRunId::new().into_string();
        repo.reserve_run(
            &owner,
            &ReserveCronRunParams {
                cron_job_run_id: stopped.clone(),
                operation_key: "stopped-opening".into(),
                ..request
            },
        )
        .await
        .unwrap();
        repo.request_run_cancellation(&owner, &job.cron_job_id, &stopped, 4)
            .await
            .unwrap();
        assert!(
            repo.attach_continuing_session(&owner, &stopped, CONVERSATION_ID, 5)
                .await
                .is_err()
        );
        assert!(
            repo.get_run_reservation(&owner, &job.cron_job_id, &stopped)
                .await
                .unwrap()
                .unwrap()
                .conversation_id
                .is_none()
        );
    }
    #[tokio::test]
    async fn same_millisecond_late_terminal_keeps_its_exact_summary_source() {
        let (repo, _db, owner) = setup().await;
        let job = make_row(&owner);
        repo.insert(&job).await.unwrap();
        let mut ids = vec![];
        for index in 0..3 {
            let id = CronJobRunId::new().into_string();
            repo.reserve_run(
                &owner,
                &ReserveCronRunParams {
                    cron_job_run_id: id.clone(),
                    cron_job_id: job.cron_job_id.clone(),
                    trigger_kind: "run_now".into(),
                    operation_key: format!("same-time:{index}"),
                    request_fingerprint: "same-time".into(),
                    schedule_revision: None,
                    planned_at_ms: None,
                    next_run_at: None,
                    disable: false,
                    now: 1,
                },
            )
            .await
            .unwrap();
            ids.push(id);
        }
        repo.finalize_run_with_job_projection(&owner, &completed(&ids[1], 100))
            .await
            .unwrap();
        let mut late = completed(&ids[0], 100);
        late.status = "error".into();
        late.last_status = Some("error".into());
        late.domain_failure_json=Some(serde_json::json!({"message":"original late failure","code":"NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE"}).to_string());
        repo.finalize_run_with_job_projection(&owner, &late)
            .await
            .unwrap();
        let current = repo
            .get_by_cron_job_id(&owner, &job.cron_job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.last_run_id.as_deref(), Some(ids[0].as_str()));
        assert_eq!(current.last_status.as_deref(), Some("error"));
        let history = repo
            .list_runs_by_job(&owner, &job.cron_job_id, 7)
            .await
            .unwrap();
        assert_eq!(
            history[0].cron_job_run_id, ids[1],
            "history sorting must not become summary authority"
        );
        let no_summary = FinalizeCronRunParams {
            cron_job_run_id: ids[2].clone(),
            status: "cancelled".into(),
            conversation_id: None,
            result_error: None,
            domain_failure_json: None,
            now: 100,
            last_run_at: None,
            last_status: None,
            last_error: None,
            increment_run_count: false,
        };
        repo.finalize_run_with_job_projection(&owner, &no_summary)
            .await
            .unwrap();
        repo.finalize_run_with_job_projection(&owner, &completed(&ids[1], 100))
            .await
            .unwrap();
        assert_eq!(
            repo.get_by_cron_job_id(&owner, &job.cron_job_id)
                .await
                .unwrap()
                .unwrap()
                .last_run_id
                .as_deref(),
            Some(ids[0].as_str()),
            "no-summary cancellation and old replay cannot borrow the source pointer"
        );
    }
}
