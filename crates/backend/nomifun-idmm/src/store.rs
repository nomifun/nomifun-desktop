use nomifun_api_types::{
    IdmmConfig, IdmmIntervention, IdmmInterventionStatus, IdmmMode, IdmmRunState, IdmmState,
};
use nomifun_common::{AppError, now_ms};
use nomifun_db::SqlitePool;
use serde::{Deserialize, Serialize};

const KEY_PREFIX: &str = "agent_session.idmm.";
const MAX_INTERVENTIONS: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedIdmmRecord {
    schema_version: u32,
    pub session_id: String,
    pub revision: u64,
    pub config: IdmmConfig,
    pub last_checked_at: Option<i64>,
    pub interventions: Vec<IdmmIntervention>,
}

impl PersistedIdmmRecord {
    fn new(session_id: &str) -> Self {
        Self {
            schema_version: 1,
            session_id: session_id.to_owned(),
            revision: 0,
            config: IdmmConfig::default(),
            last_checked_at: None,
            interventions: Vec::new(),
        }
    }

    pub fn state(&self) -> IdmmState {
        let run_state = if self.config.mode == IdmmMode::Off {
            IdmmRunState::Off
        } else if self
            .interventions
            .first()
            .is_some_and(|item| item.status == IdmmInterventionStatus::Pending)
        {
            IdmmRunState::Intervening
        } else if self
            .interventions
            .first()
            .is_some_and(|item| item.status == IdmmInterventionStatus::Failed)
        {
            IdmmRunState::Degraded
        } else {
            IdmmRunState::Monitoring
        };
        IdmmState {
            agent_session_id: self.session_id.clone(),
            revision: self.revision,
            config: self.config.clone(),
            run_state,
            last_checked_at: self.last_checked_at,
            recent_interventions: self.interventions.iter().take(20).cloned().collect(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct IdmmStore {
    pool: SqlitePool,
}

impl IdmmStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn key(session_id: &str) -> String {
        format!("{KEY_PREFIX}{session_id}")
    }

    pub async fn load(&self, session_id: &str) -> Result<PersistedIdmmRecord, AppError> {
        let value: Option<String> = nomifun_db::sqlx::query_scalar(
            "SELECT value FROM client_preferences WHERE key = ?",
        )
        .bind(Self::key(session_id))
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Internal(format!("read IDMM state: {error}")))?;
        let Some(value) = value else {
            return Ok(PersistedIdmmRecord::new(session_id));
        };
        let record: PersistedIdmmRecord = serde_json::from_str(&value)
            .map_err(|error| AppError::Internal(format!("decode IDMM state: {error}")))?;
        if record.schema_version != 1 || record.session_id != session_id {
            return Err(AppError::Conflict("IDMM state identity is invalid".into()));
        }
        Ok(record)
    }

    pub async fn save(
        &self,
        record: &PersistedIdmmRecord,
        expected_revision: u64,
    ) -> Result<(), AppError> {
        let value = serde_json::to_string(record)
            .map_err(|error| AppError::Internal(format!("encode IDMM state: {error}")))?;
        let key = Self::key(&record.session_id);
        let now = now_ms();
        let updated = nomifun_db::sqlx::query(
            "UPDATE client_preferences SET value = ?, updated_at = ? \
             WHERE key = ? AND json_extract(value, '$.revision') = ?",
        )
        .bind(value)
        .bind(now)
        .bind(&key)
        .bind(i64::try_from(expected_revision).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await
        .map_err(|error| AppError::Internal(format!("write IDMM state: {error}")))?;
        if updated.rows_affected() == 1 {
            return Ok(());
        }
        let exists: bool = nomifun_db::sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM client_preferences WHERE key = ?)",
        )
        .bind(&key)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| AppError::Internal(format!("check IDMM state revision: {error}")))?;
        if exists || expected_revision != 0 {
            return Err(AppError::Conflict(
                "IDMM configuration changed while an intervention was in progress".into(),
            ));
        }
        nomifun_db::sqlx::query(
            "INSERT INTO client_preferences (key, value, updated_at) VALUES (?, ?, ?)",
        )
        .bind(key)
        .bind(
            serde_json::to_string(record)
                .map_err(|error| AppError::Internal(format!("encode IDMM state: {error}")))?,
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|error| AppError::Conflict(format!("create IDMM state: {error}")))?;
        Ok(())
    }

    pub async fn list_enabled(&self) -> Result<Vec<PersistedIdmmRecord>, AppError> {
        let rows: Vec<String> = nomifun_db::sqlx::query_scalar(
            "SELECT value FROM client_preferences WHERE key GLOB ? ORDER BY key",
        )
        .bind(format!("{KEY_PREFIX}%"))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| AppError::Internal(format!("list IDMM states: {error}")))?;
        let mut records = Vec::new();
        for value in rows {
            match serde_json::from_str::<PersistedIdmmRecord>(&value) {
                Ok(record)
                    if record.schema_version == 1 && record.config.mode != IdmmMode::Off =>
                {
                    records.push(record);
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "ignored malformed persisted IDMM state"),
            }
        }
        Ok(records)
    }

    pub async fn remove(&self, session_id: &str) -> Result<(), AppError> {
        nomifun_db::sqlx::query("DELETE FROM client_preferences WHERE key = ?")
            .bind(Self::key(session_id))
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Internal(format!("remove IDMM state: {error}")))?;
        Ok(())
    }

    pub fn push_intervention(record: &mut PersistedIdmmRecord, item: IdmmIntervention) {
        record.interventions.insert(0, item);
        record.interventions.truncate(MAX_INTERVENTIONS);
    }
}
