use nomifun_common::{
    KnowledgeBaseId, KnowledgeTreeOperationId, TimestampMs, is_visible_ascii_key,
};
use serde_json::Value;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::{
    KNOWLEDGE_TREE_EVENT_STATUS_PENDING, KNOWLEDGE_TREE_EVENT_STATUS_PUBLISHED,
    KNOWLEDGE_TREE_OPERATION_STATE_COMMITTED,
    KnowledgeTreeEventStatus, KnowledgeTreeOperationRow, KnowledgeTreeOperationState,
};
use crate::repository::knowledge_tree_operation::{
    CommitKnowledgeTreeOperationParams, IKnowledgeTreeOperationRepository,
    KnowledgeTreeOperationPageCursor, MAX_KNOWLEDGE_TREE_OPERATION_PAGE_SIZE,
    PrepareKnowledgeTreeOperationParams, PreparedKnowledgeTreeOperation,
};

const MAX_REQUEST_ID_LEN: usize = 128;
const MAX_RECOVERY_ERROR_CHARS: usize = 8192;
const MAX_FS_IDENTITY_LEN: usize = 512;

#[derive(Clone, Debug)]
pub struct SqliteKnowledgeTreeOperationRepository {
    pool: SqlitePool,
}

impl SqliteKnowledgeTreeOperationRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn operation_query_error(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(database_error)
            if database_error
                .message()
                .to_ascii_lowercase()
                .contains("constraint failed") =>
        {
            DbError::Conflict(format!(
                "knowledge-tree operation violates a uniqueness or value constraint: {}",
                database_error.message()
            ))
        }
        _ => DbError::Query(error),
    }
}

fn validate_request_id(request_id: &str) -> Result<(), DbError> {
    if !is_visible_ascii_key(request_id, MAX_REQUEST_ID_LEN) {
        return Err(DbError::Conflict(
            "knowledge-tree request_id must contain 1 to 128 visible ASCII characters".into(),
        ));
    }
    Ok(())
}

fn validate_fingerprint(fingerprint: &str) -> Result<(), DbError> {
    if fingerprint.len() != 64
        || !fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DbError::Conflict(
            "knowledge-tree operation fingerprint must be a lowercase SHA-256 hex digest"
                .into(),
        ));
    }
    Ok(())
}

fn validate_rel_path(path: &str, label: &str) -> Result<(), DbError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains(['\\', '\0'])
        || path
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(DbError::Conflict(format!(
            "knowledge-tree operation {label} must be a canonical non-empty relative path"
        )));
    }
    Ok(())
}

fn validate_timestamp(timestamp: TimestampMs, label: &str) -> Result<(), DbError> {
    if timestamp < 0 {
        return Err(DbError::Conflict(format!(
            "knowledge-tree operation {label} must be non-negative"
        )));
    }
    Ok(())
}

fn validate_transition_timestamp(
    timestamp: TimestampMs,
    row: &KnowledgeTreeOperationRow,
    label: &str,
) -> Result<(), DbError> {
    validate_timestamp(timestamp, label)?;
    if timestamp < row.updated_at {
        return Err(DbError::Conflict(format!(
            "knowledge-tree operation {label} predates its latest durable transition"
        )));
    }
    Ok(())
}

fn canonical_json(value: &Value, label: &str) -> Result<String, DbError> {
    if !value.is_object() {
        return Err(DbError::Conflict(format!(
            "knowledge-tree operation {label} must be a JSON object"
        )));
    }
    serde_json::to_string(value).map_err(|error| {
        DbError::Conflict(format!(
            "knowledge-tree operation {label} cannot be serialized: {error}"
        ))
    })
}

fn persisted_json_equals(persisted: Option<&str>, expected: &Value) -> bool {
    persisted
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .is_some_and(|value| value == *expected)
}

fn validate_page_size(limit: u32) -> Result<i64, DbError> {
    if limit == 0 || limit > MAX_KNOWLEDGE_TREE_OPERATION_PAGE_SIZE {
        return Err(DbError::Conflict(format!(
            "knowledge-tree operation page size must be between 1 and {MAX_KNOWLEDGE_TREE_OPERATION_PAGE_SIZE}"
        )));
    }
    Ok(i64::from(limit))
}

async fn fetch_by_operation(
    transaction: &mut Transaction<'_, Sqlite>,
    operation_id: &KnowledgeTreeOperationId,
) -> Result<Option<KnowledgeTreeOperationRow>, DbError> {
    sqlx::query_as::<_, KnowledgeTreeOperationRow>(
        "SELECT * FROM knowledge_tree_operations WHERE operation_id = ?",
    )
    .bind(operation_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DbError::Query)
}

async fn fetch_by_request(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_base_id: &KnowledgeBaseId,
    request_id: &str,
) -> Result<Option<KnowledgeTreeOperationRow>, DbError> {
    sqlx::query_as::<_, KnowledgeTreeOperationRow>(
        "SELECT * FROM knowledge_tree_operations \
         WHERE knowledge_base_id = ? AND request_id = ?",
    )
    .bind(knowledge_base_id.as_str())
    .bind(request_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DbError::Query)
}

async fn lock_operation(
    transaction: &mut Transaction<'_, Sqlite>,
    operation_id: &KnowledgeTreeOperationId,
) -> Result<KnowledgeTreeOperationRow, DbError> {
    let locked = sqlx::query(
        "UPDATE knowledge_tree_operations SET updated_at = updated_at WHERE operation_id = ?",
    )
    .bind(operation_id.as_str())
    .execute(&mut **transaction)
    .await?;
    if locked.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "knowledge-tree operation {operation_id}"
        )));
    }
    fetch_by_operation(transaction, operation_id)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("knowledge-tree operation {operation_id}")))
}

fn transition_conflict(
    row: &KnowledgeTreeOperationRow,
    transition: &str,
) -> DbError {
    DbError::Conflict(format!(
        "knowledge-tree operation {} cannot {transition} from state {}",
        row.operation_id,
        row.state.as_str()
    ))
}

#[async_trait::async_trait]
impl IKnowledgeTreeOperationRepository for SqliteKnowledgeTreeOperationRepository {
    async fn prepare_operation(
        &self,
        params: &PrepareKnowledgeTreeOperationParams,
    ) -> Result<PreparedKnowledgeTreeOperation, DbError> {
        validate_request_id(&params.request_id)?;
        validate_fingerprint(&params.fingerprint)?;
        validate_rel_path(&params.source_rel_path, "source_rel_path")?;
        validate_rel_path(&params.destination_rel_path, "destination_rel_path")?;
        if params.source_fs_identity.as_ref().is_some_and(|identity| {
            identity.is_empty() || identity.len() > MAX_FS_IDENTITY_LEN
        }) {
            return Err(DbError::Conflict(format!(
                "knowledge-tree source_fs_identity must contain 1 to {MAX_FS_IDENTITY_LEN} bytes"
            )));
        }
        validate_timestamp(params.created_at, "created_at")?;

        let operation_id = KnowledgeTreeOperationId::new();
        let mut transaction = self.pool.begin().await?;

        // Take the base row's writer lock before checking/inserting the logical
        // child. This prevents base deletion from interleaving with prepare.
        let base = sqlx::query(
            "UPDATE knowledge_bases SET updated_at = updated_at WHERE knowledge_base_id = ?",
        )
        .bind(params.knowledge_base_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if base.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "knowledge base {}",
                params.knowledge_base_id
            )));
        }

        let inserted = sqlx::query(
            "INSERT INTO knowledge_tree_operations (\
                operation_id, knowledge_base_id, request_id, fingerprint, source_rel_path, \
                destination_rel_path, source_fs_identity, state, event_status, created_at, updated_at\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, 'prepared', 'none', ?, ?) \
             ON CONFLICT(knowledge_base_id, request_id) DO NOTHING",
        )
        .bind(operation_id.as_str())
        .bind(params.knowledge_base_id.as_str())
        .bind(&params.request_id)
        .bind(&params.fingerprint)
        .bind(&params.source_rel_path)
        .bind(&params.destination_rel_path)
        .bind(&params.source_fs_identity)
        .bind(params.created_at)
        .bind(params.created_at)
        .execute(&mut *transaction)
        .await
        .map_err(operation_query_error)?
        .rows_affected()
            == 1;

        let operation = fetch_by_request(
            &mut transaction,
            &params.knowledge_base_id,
            &params.request_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::Query(sqlx::Error::RowNotFound)
        })?;
        if operation.fingerprint != params.fingerprint
            || operation.source_rel_path != params.source_rel_path
            || operation.destination_rel_path != params.destination_rel_path
            || operation.source_fs_identity != params.source_fs_identity
        {
            return Err(DbError::Conflict(format!(
                "knowledge-tree request_id '{}' was already used for a different operation",
                params.request_id
            )));
        }

        transaction.commit().await?;
        Ok(PreparedKnowledgeTreeOperation {
            operation,
            created: inserted,
        })
    }

    async fn mark_filesystem_committed(
        &self,
        operation_id: &KnowledgeTreeOperationId,
        committed_at: TimestampMs,
    ) -> Result<KnowledgeTreeOperationRow, DbError> {
        validate_timestamp(committed_at, "filesystem_committed_at")?;
        let mut transaction = self.pool.begin().await?;
        let row = lock_operation(&mut transaction, operation_id).await?;
        match row.state {
            KnowledgeTreeOperationState::Prepared
            | KnowledgeTreeOperationState::NeedsRecovery => {
                validate_transition_timestamp(committed_at, &row, "filesystem_committed_at")?;
                sqlx::query(
                    "UPDATE knowledge_tree_operations \
                     SET state = 'filesystem_committed', \
                         filesystem_committed_at = COALESCE(filesystem_committed_at, ?), \
                         error_message = NULL, updated_at = ? \
                     WHERE operation_id = ?",
                )
                .bind(committed_at)
                .bind(committed_at)
                .bind(operation_id.as_str())
                .execute(&mut *transaction)
                .await
                .map_err(operation_query_error)?;
            }
            KnowledgeTreeOperationState::FilesystemCommitted
            | KnowledgeTreeOperationState::Committed => {
                transaction.commit().await?;
                return Ok(row);
            }
        }
        let updated = fetch_by_operation(&mut transaction, operation_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn commit_operation(
        &self,
        params: &CommitKnowledgeTreeOperationParams,
    ) -> Result<KnowledgeTreeOperationRow, DbError> {
        validate_timestamp(params.committed_at, "committed_at")?;
        let receipt_json = canonical_json(&params.receipt, "receipt")?;
        let event_payload_json = canonical_json(&params.event_payload, "event_payload")?;
        let mut transaction = self.pool.begin().await?;
        let row = lock_operation(&mut transaction, &params.operation_id).await?;
        match row.state {
            KnowledgeTreeOperationState::Committed => {
                if !persisted_json_equals(row.receipt_json.as_deref(), &params.receipt)
                    || !persisted_json_equals(
                        row.event_payload_json.as_deref(),
                        &params.event_payload,
                    )
                {
                    return Err(DbError::Conflict(format!(
                        "knowledge-tree operation {} was already committed with another receipt or event",
                        params.operation_id
                    )));
                }
                transaction.commit().await?;
                return Ok(row);
            }
            KnowledgeTreeOperationState::FilesystemCommitted => {}
            KnowledgeTreeOperationState::Prepared
            | KnowledgeTreeOperationState::NeedsRecovery => {
                return Err(transition_conflict(&row, "commit"));
            }
        }
        validate_transition_timestamp(params.committed_at, &row, "committed_at")?;
        sqlx::query(
            "UPDATE knowledge_tree_operations \
             SET state = 'committed', receipt_json = ?, error_message = NULL, \
                 event_status = 'pending', event_payload_json = ?, \
                 committed_at = ?, updated_at = ? \
             WHERE operation_id = ?",
        )
        .bind(receipt_json)
        .bind(event_payload_json)
        .bind(params.committed_at)
        .bind(params.committed_at)
        .bind(params.operation_id.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(operation_query_error)?;
        let updated = fetch_by_operation(&mut transaction, &params.operation_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn mark_needs_recovery(
        &self,
        operation_id: &KnowledgeTreeOperationId,
        error_message: &str,
        updated_at: TimestampMs,
    ) -> Result<KnowledgeTreeOperationRow, DbError> {
        if error_message.is_empty() || error_message.chars().count() > MAX_RECOVERY_ERROR_CHARS {
            return Err(DbError::Conflict(format!(
                "knowledge-tree recovery error must contain 1 to {MAX_RECOVERY_ERROR_CHARS} characters"
            )));
        }
        validate_timestamp(updated_at, "updated_at")?;
        let mut transaction = self.pool.begin().await?;
        let row = lock_operation(&mut transaction, operation_id).await?;
        if row.state == KnowledgeTreeOperationState::Committed {
            return Err(transition_conflict(&row, "enter recovery"));
        }
        if row.state == KnowledgeTreeOperationState::NeedsRecovery
            && row.error_message.as_deref() == Some(error_message)
        {
            transaction.commit().await?;
            return Ok(row);
        }
        validate_transition_timestamp(updated_at, &row, "updated_at")?;
        sqlx::query(
            "UPDATE knowledge_tree_operations \
             SET state = 'needs_recovery', error_message = ?, updated_at = ? \
             WHERE operation_id = ?",
        )
        .bind(error_message)
        .bind(updated_at)
        .bind(operation_id.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(operation_query_error)?;
        let updated = fetch_by_operation(&mut transaction, operation_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn load_by_request(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        request_id: &str,
    ) -> Result<Option<KnowledgeTreeOperationRow>, DbError> {
        validate_request_id(request_id)?;
        sqlx::query_as::<_, KnowledgeTreeOperationRow>(
            "SELECT * FROM knowledge_tree_operations \
             WHERE knowledge_base_id = ? AND request_id = ?",
        )
        .bind(knowledge_base_id.as_str())
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn load_by_operation(
        &self,
        operation_id: &KnowledgeTreeOperationId,
    ) -> Result<Option<KnowledgeTreeOperationRow>, DbError> {
        sqlx::query_as::<_, KnowledgeTreeOperationRow>(
            "SELECT * FROM knowledge_tree_operations WHERE operation_id = ?",
        )
        .bind(operation_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn list_pending_recovery_after(
        &self,
        limit: u32,
        after: Option<&KnowledgeTreeOperationPageCursor>,
    ) -> Result<Vec<KnowledgeTreeOperationRow>, DbError> {
        let limit = validate_page_size(limit)?;
        if let Some(after) = after {
            validate_timestamp(after.timestamp, "recovery cursor timestamp")?;
            sqlx::query_as::<_, KnowledgeTreeOperationRow>(
                "SELECT * FROM knowledge_tree_operations \
                 WHERE state <> 'committed' \
                   AND (created_at > ? OR (created_at = ? AND operation_id > ?)) \
                 ORDER BY created_at ASC, operation_id ASC LIMIT ?",
            )
            .bind(after.timestamp)
            .bind(after.timestamp)
            .bind(after.operation_id.as_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
        } else {
            sqlx::query_as::<_, KnowledgeTreeOperationRow>(
                "SELECT * FROM knowledge_tree_operations \
                 WHERE state <> 'committed' \
                 ORDER BY created_at ASC, operation_id ASC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
        }
    }

    async fn list_pending_events_after(
        &self,
        limit: u32,
        after: Option<&KnowledgeTreeOperationPageCursor>,
    ) -> Result<Vec<KnowledgeTreeOperationRow>, DbError> {
        let limit = validate_page_size(limit)?;
        if let Some(after) = after {
            validate_timestamp(after.timestamp, "event cursor timestamp")?;
            sqlx::query_as::<_, KnowledgeTreeOperationRow>(
                "SELECT * FROM knowledge_tree_operations \
                 WHERE state = ? AND event_status = ? \
                   AND (committed_at > ? OR (committed_at = ? AND operation_id > ?)) \
                 ORDER BY committed_at ASC, operation_id ASC LIMIT ?",
            )
            .bind(KNOWLEDGE_TREE_OPERATION_STATE_COMMITTED)
            .bind(KNOWLEDGE_TREE_EVENT_STATUS_PENDING)
            .bind(after.timestamp)
            .bind(after.timestamp)
            .bind(after.operation_id.as_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
        } else {
            sqlx::query_as::<_, KnowledgeTreeOperationRow>(
                "SELECT * FROM knowledge_tree_operations \
                 WHERE state = ? AND event_status = ? \
                 ORDER BY committed_at ASC, operation_id ASC LIMIT ?",
            )
            .bind(KNOWLEDGE_TREE_OPERATION_STATE_COMMITTED)
            .bind(KNOWLEDGE_TREE_EVENT_STATUS_PENDING)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
        }
    }

    async fn mark_event_published(
        &self,
        operation_id: &KnowledgeTreeOperationId,
        published_at: TimestampMs,
    ) -> Result<KnowledgeTreeOperationRow, DbError> {
        validate_timestamp(published_at, "event_published_at")?;
        let mut transaction = self.pool.begin().await?;
        let row = lock_operation(&mut transaction, operation_id).await?;
        if row.state != KnowledgeTreeOperationState::Committed {
            return Err(transition_conflict(&row, "publish its event"));
        }
        match row.event_status {
            KnowledgeTreeEventStatus::Published => {
                transaction.commit().await?;
                return Ok(row);
            }
            KnowledgeTreeEventStatus::None => {
                return Err(DbError::Conflict(format!(
                    "knowledge-tree operation {operation_id} has no outbox event"
                )));
            }
            KnowledgeTreeEventStatus::Pending => {}
        }
        validate_transition_timestamp(published_at, &row, "event_published_at")?;
        sqlx::query(
            "UPDATE knowledge_tree_operations \
             SET event_status = ?, event_published_at = ?, updated_at = ? \
             WHERE operation_id = ? AND event_status = ?",
        )
        .bind(KNOWLEDGE_TREE_EVENT_STATUS_PUBLISHED)
        .bind(published_at)
        .bind(published_at)
        .bind(operation_id.as_str())
        .bind(KNOWLEDGE_TREE_EVENT_STATUS_PENDING)
        .execute(&mut *transaction)
        .await
        .map_err(operation_query_error)?;
        let updated = fetch_by_operation(&mut transaction, operation_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        transaction.commit().await?;
        Ok(updated)
    }
}
