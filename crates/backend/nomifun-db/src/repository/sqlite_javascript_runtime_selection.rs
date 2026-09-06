use std::collections::BTreeSet;

use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    DbError, JavaScriptRuntimeSelectionRecord, JavaScriptRuntimeSelectionRow,
    SaveJavaScriptRuntimeSelectionParams,
};
use crate::repository::javascript_runtime_selection::IJavaScriptRuntimeSelectionRepository;

const SINGLETON_KEY: &str = "javascript_runtime_selection";
const MAX_ERROR_CODE_CHARS: usize = 256;
const MAX_RUNTIME_ID_CHARS: usize = 512;

#[derive(Clone, Debug)]
pub struct SqliteJavaScriptRuntimeSelectionRepository {
    pool: SqlitePool,
}

impl SqliteJavaScriptRuntimeSelectionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn conflict(message: impl Into<String>) -> DbError {
    DbError::Conflict(message.into())
}

fn query_error(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(database_error) => conflict(format!(
            "JavaScript Runtime selection mutation was rejected by SQLite: {}",
            database_error.message()
        )),
        _ => DbError::Query(error),
    }
}

fn require_json_object(value: Option<&Value>, label: &str) -> Result<Option<String>, DbError> {
    value
        .map(|value| {
            if !value.is_object() {
                return Err(conflict(format!("{label} must be a JSON object")));
            }
            serde_json::to_string(value)
                .map_err(|error| conflict(format!("{label} cannot be serialized: {error}")))
        })
        .transpose()
}

fn validate_error_code(value: Option<&str>) -> Result<(), DbError> {
    if value.is_some_and(|code| {
        code.is_empty()
            || code.chars().count() > MAX_ERROR_CODE_CHARS
            || !code.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        return Err(conflict(
            "Runtime selection last_error_code must contain 1 to 256 visible ASCII characters",
        ));
    }
    Ok(())
}

fn validate_warning_acknowledgements(values: &BTreeSet<String>) -> Result<(), DbError> {
    if values.iter().any(|value| {
        value.is_empty()
            || value.chars().count() > MAX_RUNTIME_ID_CHARS
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        return Err(conflict(
            "Runtime warning acknowledgement IDs must contain 1 to 512 visible ASCII characters",
        ));
    }
    Ok(())
}

fn validate_pending_validation_pair(
    pending_candidate: Option<&Value>,
    validation_result: Option<&Value>,
) -> Result<(), DbError> {
    let Some(validation_result) = validation_result else {
        return Ok(());
    };
    let pending_candidate = pending_candidate.ok_or_else(|| {
        conflict("Runtime switch validation_result requires a pending_candidate")
    })?;
    if validation_result.get("candidate") != Some(pending_candidate) {
        return Err(conflict(
            "Runtime switch validation_result must bind the exact pending_candidate",
        ));
    }
    Ok(())
}

fn decode_optional_object(value: Option<String>, label: &str) -> Result<Option<Value>, DbError> {
    value
        .map(|value| {
            let decoded: Value = serde_json::from_str(&value).map_err(|error| {
                DbError::Init(format!("persisted {label} is invalid JSON: {error}"))
            })?;
            if !decoded.is_object() {
                return Err(DbError::Init(format!(
                    "persisted {label} must be a JSON object"
                )));
            }
            Ok(decoded)
        })
        .transpose()
}

fn decode_row(row: JavaScriptRuntimeSelectionRow) -> Result<JavaScriptRuntimeSelectionRecord, DbError> {
    if row.singleton_key != SINGLETON_KEY {
        return Err(DbError::Init(
            "JavaScript Runtime selection singleton key is invalid".into(),
        ));
    }
    let selected_runtime =
        decode_optional_object(row.selected_runtime_json, "selected_runtime_json")?;
    let pending_candidate =
        decode_optional_object(row.pending_candidate_json, "pending_candidate_json")?;
    let validation_result =
        decode_optional_object(row.validation_result_json, "validation_result_json")?;
    validate_error_code(row.last_error_code.as_deref())
        .map_err(|error| DbError::Init(error.to_string()))?;
    validate_pending_validation_pair(
        pending_candidate.as_ref(),
        validation_result.as_ref(),
    )
    .map_err(|error| DbError::Init(error.to_string()))?;
    let warnings: Vec<String> =
        serde_json::from_str(&row.non_recommended_warning_acknowledged_json).map_err(|error| {
            DbError::Init(format!(
                "persisted Runtime warning acknowledgements are invalid JSON: {error}"
            ))
        })?;
    let warning_count = warnings.len();
    let warnings = warnings.into_iter().collect::<BTreeSet<_>>();
    if warnings.len() != warning_count {
        return Err(DbError::Init(
            "persisted Runtime warning acknowledgements contain duplicates".into(),
        ));
    }
    validate_warning_acknowledgements(&warnings)
        .map_err(|error| DbError::Init(error.to_string()))?;
    Ok(JavaScriptRuntimeSelectionRecord {
        selected_runtime,
        pending_candidate,
        validation_result,
        last_error_code: row.last_error_code,
        non_recommended_warning_acknowledged: warnings,
        revision: row.revision,
        updated_at: row.updated_at,
    })
}

#[async_trait::async_trait]
impl IJavaScriptRuntimeSelectionRepository for SqliteJavaScriptRuntimeSelectionRepository {
    async fn load(&self) -> Result<JavaScriptRuntimeSelectionRecord, DbError> {
        let rows: Vec<JavaScriptRuntimeSelectionRow> =
            sqlx::query_as("SELECT * FROM javascript_runtime_selection")
                .fetch_all(&self.pool)
                .await
                .map_err(DbError::Query)?;
        match rows.as_slice() {
            [] => Ok(JavaScriptRuntimeSelectionRecord::default()),
            [row] => decode_row(row.clone()),
            _ => Err(DbError::Init(format!(
                "JavaScript Runtime selection must contain at most one row, found {}",
                rows.len()
            ))),
        }
    }

    async fn save_cas(
        &self,
        params: &SaveJavaScriptRuntimeSelectionParams,
    ) -> Result<JavaScriptRuntimeSelectionRecord, DbError> {
        if params.expected_revision < 0 || params.updated_at < 0 {
            return Err(conflict(
                "Runtime selection expected_revision and updated_at must be non-negative",
            ));
        }
        validate_error_code(params.last_error_code.as_deref())?;
        validate_warning_acknowledgements(
            &params.non_recommended_warning_acknowledged,
        )?;
        validate_pending_validation_pair(
            params.pending_candidate.as_ref(),
            params.validation_result.as_ref(),
        )?;
        let selected_runtime_json =
            require_json_object(params.selected_runtime.as_ref(), "selected_runtime")?;
        let pending_candidate_json =
            require_json_object(params.pending_candidate.as_ref(), "pending_candidate")?;
        let validation_result_json =
            require_json_object(params.validation_result.as_ref(), "validation_result")?;
        let warnings_json =
            serde_json::to_string(&params.non_recommended_warning_acknowledged)
                .map_err(|error| conflict(format!("warning IDs cannot be serialized: {error}")))?;

        let mut tx = self.pool.begin().await?;
        let next_revision = params
            .expected_revision
            .checked_add(1)
            .ok_or_else(|| conflict("Runtime selection revision overflow"))?;
        let changed = if params.expected_revision == 0 {
            sqlx::query(
                "INSERT INTO javascript_runtime_selection (
                    singleton_key, selected_runtime_json, pending_candidate_json,
                    validation_result_json, last_error_code,
                    non_recommended_warning_acknowledged_json, revision, updated_at
                 )
                 SELECT ?, ?, ?, ?, ?, ?, ?, ?
                 WHERE NOT EXISTS (SELECT 1 FROM javascript_runtime_selection)",
            )
            .bind(SINGLETON_KEY)
            .bind(&selected_runtime_json)
            .bind(&pending_candidate_json)
            .bind(&validation_result_json)
            .bind(&params.last_error_code)
            .bind(&warnings_json)
            .bind(next_revision)
            .bind(params.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?
            .rows_affected()
        } else {
            sqlx::query(
                "UPDATE javascript_runtime_selection
                 SET selected_runtime_json = ?, pending_candidate_json = ?,
                     validation_result_json = ?, last_error_code = ?,
                     non_recommended_warning_acknowledged_json = ?,
                     revision = ?, updated_at = ?
                 WHERE singleton_key = ? AND revision = ? AND updated_at <= ?",
            )
            .bind(&selected_runtime_json)
            .bind(&pending_candidate_json)
            .bind(&validation_result_json)
            .bind(&params.last_error_code)
            .bind(&warnings_json)
            .bind(next_revision)
            .bind(params.updated_at)
            .bind(SINGLETON_KEY)
            .bind(params.expected_revision)
            .bind(params.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?
            .rows_affected()
        };
        if changed != 1 {
            return Err(conflict(format!(
                "Runtime selection revision CAS failed at expected revision {}",
                params.expected_revision
            )));
        }
        let row: JavaScriptRuntimeSelectionRow =
            sqlx::query_as("SELECT * FROM javascript_runtime_selection")
                .fetch_one(&mut *tx)
                .await?;
        let record = decode_row(row)?;
        tx.commit().await?;
        Ok(record)
    }
}
