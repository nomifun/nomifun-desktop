use nomifun_common::{
    KnowledgeBaseId, KnowledgeEntryId, KnowledgeSourceId, KnowledgeSourceItemId, TimestampMs,
};
use sqlx::{Sqlite, Transaction};

use crate::error::DbError;
use crate::models::{
    KnowledgeEntryProvenanceRelationship, KnowledgeEntryProvenanceRow, KnowledgeSourceItemRow,
    KnowledgeSourceItemSyncStatus, KnowledgeSourceRow, KnowledgeSourceState,
};
use crate::repository::knowledge_source::{
    BindManagedKnowledgeEntryParams, CreateKnowledgeSourceItemParams,
    EnsureKnowledgeSourceParams, EnsuredKnowledgeSource, IKnowledgeSourceRepository,
    RecordKnowledgeEntryCopyParams, RecordKnowledgeSourceSyncFailureParams,
    RecordKnowledgeSourceSyncSuccessParams, StageKnowledgeSourcePublicationParams,
    UpdateKnowledgeSourceItemParams, UpdateKnowledgeSourceParams,
};
use crate::repository::sqlite_knowledge::SqliteKnowledgeRepository;

const MAX_URL_LEN: usize = 8192;
const MAX_TITLE_LEN: usize = 1024;
const MAX_ETAG_LEN: usize = 4096;
const MAX_HTTP_LAST_MODIFIED_LEN: usize = 512;
const MAX_SYNC_ERROR_LEN: usize = 8192;

fn source_query_error(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(database_error)
            if database_error
                .message()
                .to_ascii_lowercase()
                .contains("constraint failed") =>
        {
            DbError::Conflict(format!(
                "knowledge source violates a uniqueness or value constraint: {}",
                database_error.message()
            ))
        }
        _ => DbError::Query(error),
    }
}

fn validate_revision(revision: i64, label: &str) -> Result<(), DbError> {
    if revision < 0 {
        return Err(DbError::Conflict(format!(
            "knowledge source {label} must be non-negative"
        )));
    }
    Ok(())
}

fn validate_timestamp(timestamp: TimestampMs, label: &str) -> Result<(), DbError> {
    if timestamp < 0 {
        return Err(DbError::Conflict(format!(
            "knowledge source {label} must be non-negative"
        )));
    }
    Ok(())
}

fn validate_transition_timestamp(
    timestamp: TimestampMs,
    current_updated_at: TimestampMs,
    label: &str,
) -> Result<(), DbError> {
    validate_timestamp(timestamp, label)?;
    if timestamp < current_updated_at {
        return Err(DbError::Conflict(format!(
            "knowledge source {label} predates its current revision"
        )));
    }
    Ok(())
}

fn validate_required_text(value: &str, label: &str, max_len: usize) -> Result<(), DbError> {
    if value.is_empty()
        || value.len() > max_len
        || value.trim() != value
        || value.contains('\0')
    {
        return Err(DbError::Conflict(format!(
            "knowledge source {label} must contain 1 to {max_len} trimmed bytes"
        )));
    }
    Ok(())
}

fn validate_optional_text(
    value: Option<&str>,
    label: &str,
    max_len: usize,
    require_trimmed: bool,
) -> Result<(), DbError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty()
        || value.len() > max_len
        || value.contains('\0')
        || (require_trimmed && value.trim() != value)
    {
        return Err(DbError::Conflict(format!(
            "knowledge source {label} must contain 1 to {max_len} valid bytes when present"
        )));
    }
    Ok(())
}

fn validate_hash(value: Option<&str>) -> Result<(), DbError> {
    if value.is_some_and(|value| {
        value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        return Err(DbError::Conflict(
            "knowledge source last_published_hash must be a lowercase SHA-256 hex digest"
                .into(),
        ));
    }
    Ok(())
}

fn validate_removed_state(
    state: KnowledgeSourceState,
    removed_at: Option<TimestampMs>,
    created_at: TimestampMs,
) -> Result<(), DbError> {
    if (state == KnowledgeSourceState::Removed) != removed_at.is_some() {
        return Err(DbError::Conflict(
            "knowledge source removed state and removed_at must change together".into(),
        ));
    }
    if let Some(removed_at) = removed_at {
        validate_timestamp(removed_at, "removed_at")?;
        if removed_at < created_at {
            return Err(DbError::Conflict(
                "knowledge source removed_at predates creation".into(),
            ));
        }
    }
    Ok(())
}

struct ItemValues<'a> {
    requested_url: &'a str,
    normalized_url: &'a str,
    final_url: Option<&'a str>,
    title: Option<&'a str>,
    ordinal: i64,
    state: KnowledgeSourceState,
    sync_status: KnowledgeSourceItemSyncStatus,
    etag: Option<&'a str>,
    http_last_modified: Option<&'a str>,
    last_attempt_at: Option<TimestampMs>,
    last_success_at: Option<TimestampMs>,
    last_error: Option<&'a str>,
    last_published_hash: Option<&'a str>,
    pending_published_hash: Option<&'a str>,
    pending_final_url: Option<&'a str>,
    pending_title: Option<&'a str>,
    pending_publication_at: Option<TimestampMs>,
    removed_at: Option<TimestampMs>,
    created_at: TimestampMs,
}

fn validate_item_values(values: ItemValues<'_>) -> Result<(), DbError> {
    validate_required_text(values.requested_url, "requested_url", MAX_URL_LEN)?;
    validate_required_text(values.normalized_url, "normalized_url", MAX_URL_LEN)?;
    validate_optional_text(values.final_url, "final_url", MAX_URL_LEN, true)?;
    validate_optional_text(values.title, "title", MAX_TITLE_LEN, true)?;
    validate_optional_text(values.etag, "etag", MAX_ETAG_LEN, false)?;
    validate_optional_text(
        values.http_last_modified,
        "http_last_modified",
        MAX_HTTP_LAST_MODIFIED_LEN,
        false,
    )?;
    validate_optional_text(
        values.last_error,
        "last_error",
        MAX_SYNC_ERROR_LEN,
        false,
    )?;
    validate_hash(values.last_published_hash)?;
    validate_hash(values.pending_published_hash)?;
    validate_optional_text(
        values.pending_final_url,
        "pending_final_url",
        MAX_URL_LEN,
        true,
    )?;
    validate_optional_text(
        values.pending_title,
        "pending_title",
        MAX_TITLE_LEN,
        true,
    )?;
    if values.ordinal < 0 {
        return Err(DbError::Conflict(
            "knowledge source item ordinal must be non-negative".into(),
        ));
    }
    if values.sync_status == KnowledgeSourceItemSyncStatus::Syncing
        && values.last_attempt_at.is_none()
    {
        return Err(DbError::Conflict(
            "a syncing knowledge source item requires last_attempt_at".into(),
        ));
    }
    validate_timestamp(values.created_at, "item created_at")?;
    for (value, label) in [
        (values.last_attempt_at, "last_attempt_at"),
        (values.last_success_at, "last_success_at"),
        (values.pending_publication_at, "pending_publication_at"),
    ] {
        if let Some(value) = value {
            validate_timestamp(value, label)?;
        }
    }
    match values.pending_published_hash {
        Some(_) => {
            if values.pending_publication_at.is_none()
                || values.state != KnowledgeSourceState::Active
                || values.sync_status != KnowledgeSourceItemSyncStatus::Syncing
            {
                return Err(DbError::Conflict(
                    "pending knowledge publication requires an active syncing item and publication timestamp"
                        .into(),
                ));
            }
        }
        None => {
            if values.pending_final_url.is_some()
                || values.pending_title.is_some()
                || values.pending_publication_at.is_some()
            {
                return Err(DbError::Conflict(
                    "pending knowledge publication metadata requires pending_published_hash"
                        .into(),
                ));
            }
        }
    }
    validate_removed_state(values.state, values.removed_at, values.created_at)
}

async fn lock_base(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_base_id: &KnowledgeBaseId,
) -> Result<(), DbError> {
    let locked = sqlx::query(
        "UPDATE knowledge_bases SET updated_at = updated_at WHERE knowledge_base_id = ?",
    )
    .bind(knowledge_base_id.as_str())
    .execute(&mut **transaction)
    .await?;
    if locked.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "knowledge base {knowledge_base_id}"
        )));
    }
    Ok(())
}

async fn fetch_source(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_source_id: &KnowledgeSourceId,
) -> Result<Option<KnowledgeSourceRow>, DbError> {
    sqlx::query_as::<_, KnowledgeSourceRow>(
        "SELECT * FROM knowledge_sources WHERE knowledge_source_id = ?",
    )
    .bind(knowledge_source_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DbError::Query)
}

async fn lock_source(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_source_id: &KnowledgeSourceId,
) -> Result<KnowledgeSourceRow, DbError> {
    let locked = sqlx::query(
        "UPDATE knowledge_sources SET updated_at = updated_at WHERE knowledge_source_id = ?",
    )
    .bind(knowledge_source_id.as_str())
    .execute(&mut **transaction)
    .await?;
    if locked.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "knowledge source {knowledge_source_id}"
        )));
    }
    fetch_source(transaction, knowledge_source_id)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("knowledge source {knowledge_source_id}")))
}

async fn fetch_item(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_source_item_id: &KnowledgeSourceItemId,
) -> Result<Option<KnowledgeSourceItemRow>, DbError> {
    sqlx::query_as::<_, KnowledgeSourceItemRow>(
        "SELECT * FROM knowledge_source_items WHERE knowledge_source_item_id = ?",
    )
    .bind(knowledge_source_item_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DbError::Query)
}

async fn lock_item(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_source_item_id: &KnowledgeSourceItemId,
) -> Result<KnowledgeSourceItemRow, DbError> {
    let locked = sqlx::query(
        "UPDATE knowledge_source_items SET updated_at = updated_at WHERE knowledge_source_item_id = ?",
    )
    .bind(knowledge_source_item_id.as_str())
    .execute(&mut **transaction)
    .await?;
    if locked.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "knowledge source item {knowledge_source_item_id}"
        )));
    }
    fetch_item(transaction, knowledge_source_item_id)
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!("knowledge source item {knowledge_source_item_id}"))
        })
}

async fn fetch_provenance(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_entry_id: &KnowledgeEntryId,
) -> Result<Option<KnowledgeEntryProvenanceRow>, DbError> {
    sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
        "SELECT * FROM knowledge_entry_provenance WHERE knowledge_entry_id = ?",
    )
    .bind(knowledge_entry_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(DbError::Query)
}

async fn lock_provenance(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_entry_id: &KnowledgeEntryId,
) -> Result<KnowledgeEntryProvenanceRow, DbError> {
    let locked = sqlx::query(
        "UPDATE knowledge_entry_provenance SET updated_at = updated_at WHERE knowledge_entry_id = ?",
    )
    .bind(knowledge_entry_id.as_str())
    .execute(&mut **transaction)
    .await?;
    if locked.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "knowledge entry provenance {knowledge_entry_id}"
        )));
    }
    fetch_provenance(transaction, knowledge_entry_id)
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!("knowledge entry provenance {knowledge_entry_id}"))
        })
}

async fn validate_default_parent(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_base_id: &KnowledgeBaseId,
    default_parent_entry_id: Option<&KnowledgeEntryId>,
) -> Result<(), DbError> {
    let Some(default_parent_entry_id) = default_parent_entry_id else {
        return Ok(());
    };
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM knowledge_entries \
         WHERE knowledge_base_id = ? AND knowledge_entry_id = ? \
           AND kind = 'directory' AND deleted_at IS NULL)",
    )
    .bind(knowledge_base_id.as_str())
    .bind(default_parent_entry_id.as_str())
    .fetch_one(&mut **transaction)
    .await?;
    if !valid {
        return Err(DbError::Conflict(format!(
            "knowledge source default parent {default_parent_entry_id} is not a live directory in base {knowledge_base_id}"
        )));
    }
    Ok(())
}

async fn validate_entry_and_item_scope(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_entry_id: &KnowledgeEntryId,
    knowledge_source_item_id: &KnowledgeSourceItemId,
    require_live_item: bool,
) -> Result<(), DbError> {
    let scope: Option<(String, Option<i64>, String, String)> = sqlx::query_as(
        "SELECT entry.knowledge_base_id, entry.deleted_at, item.state, source.knowledge_base_id \
         FROM knowledge_entries entry \
         JOIN knowledge_source_items item ON item.knowledge_source_item_id = ? \
         JOIN knowledge_sources source ON source.knowledge_source_id = item.knowledge_source_id \
         WHERE entry.knowledge_entry_id = ?",
    )
    .bind(knowledge_source_item_id.as_str())
    .bind(knowledge_entry_id.as_str())
    .fetch_optional(&mut **transaction)
    .await?;
    let Some((entry_base_id, deleted_at, item_state, source_base_id)) = scope else {
        return Err(DbError::Conflict(
            "knowledge provenance requires an existing entry and source item".into(),
        ));
    };
    if deleted_at.is_some() {
        return Err(DbError::Conflict(format!(
            "knowledge provenance entry {knowledge_entry_id} is deleted"
        )));
    }
    if entry_base_id != source_base_id {
        return Err(DbError::Conflict(
            "knowledge provenance entry and source item belong to different bases".into(),
        ));
    }
    if require_live_item && item_state == KnowledgeSourceState::Removed.as_str() {
        return Err(DbError::Conflict(format!(
            "knowledge source item {knowledge_source_item_id} is removed"
        )));
    }
    Ok(())
}

async fn ensure_no_managed_provenance(
    transaction: &mut Transaction<'_, Sqlite>,
    knowledge_source_item_id: &KnowledgeSourceItemId,
) -> Result<(), DbError> {
    let managed: Option<String> = sqlx::query_scalar(
        "SELECT knowledge_entry_id FROM knowledge_entry_provenance \
         WHERE knowledge_source_item_id = ? AND relationship = 'managed' LIMIT 1",
    )
    .bind(knowledge_source_item_id.as_str())
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(entry_id) = managed {
        return Err(DbError::Conflict(format!(
            "knowledge source item {knowledge_source_item_id} still manages entry {entry_id}; detach or delete it first"
        )));
    }
    Ok(())
}

fn ensure_expected_revision(actual: i64, expected: i64, label: &str) -> Result<(), DbError> {
    validate_revision(expected, "expected_revision")?;
    if actual != expected {
        return Err(DbError::Conflict(format!(
            "{label} revision conflict: expected {expected}, current {actual}"
        )));
    }
    Ok(())
}

fn is_expected_or_one_ahead(actual: i64, expected: i64) -> bool {
    actual == expected || actual == expected.saturating_add(1)
}

#[async_trait::async_trait]
impl IKnowledgeSourceRepository for SqliteKnowledgeRepository {
    async fn ensure_source(
        &self,
        params: &EnsureKnowledgeSourceParams,
    ) -> Result<EnsuredKnowledgeSource, DbError> {
        validate_timestamp(params.created_at, "created_at")?;
        let mut transaction = self.pool.begin().await?;
        lock_base(&mut transaction, &params.knowledge_base_id).await?;

        if let Some(source) = sqlx::query_as::<_, KnowledgeSourceRow>(
            "SELECT * FROM knowledge_sources \
             WHERE knowledge_base_id = ? AND kind = ? AND state <> 'removed'",
        )
        .bind(params.knowledge_base_id.as_str())
        .bind(params.kind.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        {
            transaction.commit().await?;
            return Ok(EnsuredKnowledgeSource {
                source,
                created: false,
            });
        }
        if let Some(source) = fetch_source(&mut transaction, &params.knowledge_source_id).await? {
            if source.knowledge_base_id == params.knowledge_base_id
                && source.kind == params.kind
            {
                transaction.commit().await?;
                return Ok(EnsuredKnowledgeSource {
                    source,
                    created: false,
                });
            }
            return Err(DbError::Conflict(format!(
                "knowledge source ID {} is already owned by another aggregate",
                params.knowledge_source_id
            )));
        }

        validate_default_parent(
            &mut transaction,
            &params.knowledge_base_id,
            params.default_parent_entry_id.as_ref(),
        )
        .await?;
        let source = sqlx::query_as::<_, KnowledgeSourceRow>(
            "INSERT INTO knowledge_sources (\
                knowledge_source_id, knowledge_base_id, kind, mode, state, revision, \
                default_parent_entry_id, removed_at, created_at, updated_at\
             ) VALUES (?, ?, ?, ?, 'active', 0, ?, NULL, ?, ?) \
             RETURNING *",
        )
        .bind(params.knowledge_source_id.as_str())
        .bind(params.knowledge_base_id.as_str())
        .bind(params.kind.as_str())
        .bind(params.mode.as_str())
        .bind(
            params
                .default_parent_entry_id
                .as_ref()
                .map(KnowledgeEntryId::as_str),
        )
        .bind(params.created_at)
        .bind(params.created_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(source_query_error)?;
        transaction.commit().await?;
        Ok(EnsuredKnowledgeSource {
            source,
            created: true,
        })
    }

    async fn get_source(
        &self,
        knowledge_source_id: &KnowledgeSourceId,
    ) -> Result<Option<KnowledgeSourceRow>, DbError> {
        sqlx::query_as::<_, KnowledgeSourceRow>(
            "SELECT * FROM knowledge_sources WHERE knowledge_source_id = ?",
        )
        .bind(knowledge_source_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn list_sources_for_base(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        include_removed: bool,
    ) -> Result<Vec<KnowledgeSourceRow>, DbError> {
        let sql = if include_removed {
            "SELECT * FROM knowledge_sources WHERE knowledge_base_id = ? \
             ORDER BY created_at, knowledge_source_id"
        } else {
            "SELECT * FROM knowledge_sources \
             WHERE knowledge_base_id = ? AND state <> 'removed' \
             ORDER BY created_at, knowledge_source_id"
        };
        sqlx::query_as::<_, KnowledgeSourceRow>(sql)
            .bind(knowledge_base_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn update_source(
        &self,
        params: &UpdateKnowledgeSourceParams,
    ) -> Result<KnowledgeSourceRow, DbError> {
        validate_revision(params.expected_revision, "expected_revision")?;
        let mut transaction = self.pool.begin().await?;
        let current = lock_source(&mut transaction, &params.knowledge_source_id).await?;
        ensure_expected_revision(
            current.revision,
            params.expected_revision,
            "knowledge source",
        )?;
        validate_transition_timestamp(params.updated_at, current.updated_at, "updated_at")?;
        validate_removed_state(params.state, params.removed_at, current.created_at)?;
        if params.state == KnowledgeSourceState::Removed
            && params.default_parent_entry_id.is_some()
        {
            return Err(DbError::Conflict(
                "removed knowledge sources cannot retain a default parent entry".into(),
            ));
        }
        validate_default_parent(
            &mut transaction,
            &current.knowledge_base_id,
            params.default_parent_entry_id.as_ref(),
        )
        .await?;
        if params.state != KnowledgeSourceState::Active {
            let syncing_items: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM knowledge_source_items \
                 WHERE knowledge_source_id = ? AND sync_status = 'syncing')",
            )
            .bind(params.knowledge_source_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
            if syncing_items {
                return Err(DbError::Conflict(format!(
                    "knowledge source {} has an in-flight sync",
                    params.knowledge_source_id
                )));
            }
        }
        if params.state == KnowledgeSourceState::Removed {
            let live_items: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM knowledge_source_items \
                 WHERE knowledge_source_id = ? AND state <> 'removed')",
            )
            .bind(params.knowledge_source_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
            if live_items {
                return Err(DbError::Conflict(format!(
                    "knowledge source {} still has live items",
                    params.knowledge_source_id
                )));
            }
        }

        let updated = sqlx::query_as::<_, KnowledgeSourceRow>(
            "UPDATE knowledge_sources SET \
                mode = ?, state = ?, default_parent_entry_id = ?, removed_at = ?, \
                revision = revision + 1, updated_at = ? \
             WHERE knowledge_source_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(params.mode.as_str())
        .bind(params.state.as_str())
        .bind(
            params
                .default_parent_entry_id
                .as_ref()
                .map(KnowledgeEntryId::as_str),
        )
        .bind(params.removed_at)
        .bind(params.updated_at)
        .bind(params.knowledge_source_id.as_str())
        .bind(params.expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge source {} changed during update",
                params.knowledge_source_id
            ))
        })?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn create_source_item(
        &self,
        params: &CreateKnowledgeSourceItemParams,
    ) -> Result<KnowledgeSourceItemRow, DbError> {
        validate_item_values(ItemValues {
            requested_url: &params.requested_url,
            normalized_url: &params.normalized_url,
            final_url: params.final_url.as_deref(),
            title: params.title.as_deref(),
            ordinal: params.ordinal,
            state: params.state,
            sync_status: params.sync_status,
            etag: params.etag.as_deref(),
            http_last_modified: params.http_last_modified.as_deref(),
            last_attempt_at: params.last_attempt_at,
            last_success_at: params.last_success_at,
            last_error: params.last_error.as_deref(),
            last_published_hash: params.last_published_hash.as_deref(),
            pending_published_hash: params.pending_published_hash.as_deref(),
            pending_final_url: params.pending_final_url.as_deref(),
            pending_title: params.pending_title.as_deref(),
            pending_publication_at: params.pending_publication_at,
            removed_at: params.removed_at,
            created_at: params.created_at,
        })?;
        let mut transaction = self.pool.begin().await?;
        let source = lock_source(&mut transaction, &params.knowledge_source_id).await?;
        if source.is_removed() {
            return Err(DbError::Conflict(format!(
                "cannot add an item to removed knowledge source {}",
                source.knowledge_source_id
            )));
        }

        let item = sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "INSERT INTO knowledge_source_items (\
                knowledge_source_item_id, knowledge_source_id, requested_url, normalized_url, \
                final_url, rendered, title, ordinal, state, sync_status, revision, etag, \
                http_last_modified, last_attempt_at, last_success_at, last_error, \
                last_published_hash, pending_published_hash, pending_final_url, pending_title, \
                pending_publication_at, removed_at, created_at, updated_at\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING *",
        )
        .bind(params.knowledge_source_item_id.as_str())
        .bind(params.knowledge_source_id.as_str())
        .bind(&params.requested_url)
        .bind(&params.normalized_url)
        .bind(&params.final_url)
        .bind(params.rendered)
        .bind(&params.title)
        .bind(params.ordinal)
        .bind(params.state.as_str())
        .bind(params.sync_status.as_str())
        .bind(&params.etag)
        .bind(&params.http_last_modified)
        .bind(params.last_attempt_at)
        .bind(params.last_success_at)
        .bind(&params.last_error)
        .bind(&params.last_published_hash)
        .bind(&params.pending_published_hash)
        .bind(&params.pending_final_url)
        .bind(&params.pending_title)
        .bind(params.pending_publication_at)
        .bind(params.removed_at)
        .bind(params.created_at)
        .bind(params.created_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(source_query_error)?;
        transaction.commit().await?;
        Ok(item)
    }

    async fn get_source_item(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
    ) -> Result<Option<KnowledgeSourceItemRow>, DbError> {
        sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "SELECT * FROM knowledge_source_items WHERE knowledge_source_item_id = ?",
        )
        .bind(knowledge_source_item_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn get_live_source_item_by_url(
        &self,
        knowledge_source_id: &KnowledgeSourceId,
        normalized_url: &str,
    ) -> Result<Option<KnowledgeSourceItemRow>, DbError> {
        validate_required_text(normalized_url, "normalized_url", MAX_URL_LEN)?;
        sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "SELECT * FROM knowledge_source_items \
             WHERE knowledge_source_id = ? AND normalized_url = ? AND state <> 'removed'",
        )
        .bind(knowledge_source_id.as_str())
        .bind(normalized_url)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn list_source_items(
        &self,
        knowledge_source_id: &KnowledgeSourceId,
        include_removed: bool,
    ) -> Result<Vec<KnowledgeSourceItemRow>, DbError> {
        let sql = if include_removed {
            "SELECT * FROM knowledge_source_items WHERE knowledge_source_id = ? \
             ORDER BY ordinal, knowledge_source_item_id"
        } else {
            "SELECT * FROM knowledge_source_items \
             WHERE knowledge_source_id = ? AND state <> 'removed' \
             ORDER BY ordinal, knowledge_source_item_id"
        };
        sqlx::query_as::<_, KnowledgeSourceItemRow>(sql)
            .bind(knowledge_source_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn update_source_item(
        &self,
        params: &UpdateKnowledgeSourceItemParams,
    ) -> Result<KnowledgeSourceItemRow, DbError> {
        validate_revision(params.expected_revision, "expected_revision")?;
        let mut transaction = self.pool.begin().await?;
        let current = lock_item(&mut transaction, &params.knowledge_source_item_id).await?;
        ensure_expected_revision(
            current.revision,
            params.expected_revision,
            "knowledge source item",
        )?;
        validate_transition_timestamp(params.updated_at, current.updated_at, "updated_at")?;
        if current.sync_status == KnowledgeSourceItemSyncStatus::Syncing {
            return Err(DbError::Conflict(format!(
                "knowledge source item {} is currently syncing",
                params.knowledge_source_item_id
            )));
        }
        if params.sync_status != current.sync_status
            || params.final_url != current.final_url
            || params.etag != current.etag
            || params.http_last_modified != current.http_last_modified
            || params.last_attempt_at != current.last_attempt_at
            || params.last_success_at != current.last_success_at
            || params.last_error != current.last_error
            || params.last_published_hash != current.last_published_hash
            || params.pending_published_hash != current.pending_published_hash
            || params.pending_final_url != current.pending_final_url
            || params.pending_title != current.pending_title
            || params.pending_publication_at != current.pending_publication_at
        {
            return Err(DbError::Conflict(
                "knowledge source sync metadata must change through the dedicated sync-result methods"
                    .into(),
            ));
        }
        if params.state == KnowledgeSourceState::Removed && !current.is_removed() {
            return Err(DbError::Conflict(
                "knowledge source items must be removed through remove_source_item".into(),
            ));
        }
        validate_item_values(ItemValues {
            requested_url: &params.requested_url,
            normalized_url: &params.normalized_url,
            final_url: params.final_url.as_deref(),
            title: params.title.as_deref(),
            ordinal: params.ordinal,
            state: params.state,
            sync_status: params.sync_status,
            etag: params.etag.as_deref(),
            http_last_modified: params.http_last_modified.as_deref(),
            last_attempt_at: params.last_attempt_at,
            last_success_at: params.last_success_at,
            last_error: params.last_error.as_deref(),
            last_published_hash: params.last_published_hash.as_deref(),
            pending_published_hash: params.pending_published_hash.as_deref(),
            pending_final_url: params.pending_final_url.as_deref(),
            pending_title: params.pending_title.as_deref(),
            pending_publication_at: params.pending_publication_at,
            removed_at: params.removed_at,
            created_at: current.created_at,
        })?;
        let source = lock_source(&mut transaction, &current.knowledge_source_id).await?;
        if source.is_removed() && params.state != KnowledgeSourceState::Removed {
            return Err(DbError::Conflict(
                "cannot restore an item while its knowledge source is removed".into(),
            ));
        }
        if params.state == KnowledgeSourceState::Removed {
            ensure_no_managed_provenance(
                &mut transaction,
                &params.knowledge_source_item_id,
            )
            .await?;
        }

        let updated = sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "UPDATE knowledge_source_items SET \
                requested_url = ?, normalized_url = ?, final_url = ?, rendered = ?, title = ?, \
                ordinal = ?, state = ?, sync_status = ?, revision = revision + 1, etag = ?, \
                http_last_modified = ?, last_attempt_at = ?, last_success_at = ?, last_error = ?, \
                last_published_hash = ?, pending_published_hash = ?, pending_final_url = ?, \
                pending_title = ?, pending_publication_at = ?, removed_at = ?, updated_at = ? \
             WHERE knowledge_source_item_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(&params.requested_url)
        .bind(&params.normalized_url)
        .bind(&params.final_url)
        .bind(params.rendered)
        .bind(&params.title)
        .bind(params.ordinal)
        .bind(params.state.as_str())
        .bind(params.sync_status.as_str())
        .bind(&params.etag)
        .bind(&params.http_last_modified)
        .bind(params.last_attempt_at)
        .bind(params.last_success_at)
        .bind(&params.last_error)
        .bind(&params.last_published_hash)
        .bind(&params.pending_published_hash)
        .bind(&params.pending_final_url)
        .bind(&params.pending_title)
        .bind(params.pending_publication_at)
        .bind(params.removed_at)
        .bind(params.updated_at)
        .bind(params.knowledge_source_item_id.as_str())
        .bind(params.expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge source item {} changed during update",
                params.knowledge_source_item_id
            ))
        })?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn record_sync_attempt(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
        expected_revision: i64,
        attempted_at: TimestampMs,
    ) -> Result<KnowledgeSourceItemRow, DbError> {
        validate_revision(expected_revision, "expected_revision")?;
        let mut transaction = self.pool.begin().await?;
        let current = lock_item(&mut transaction, knowledge_source_item_id).await?;
        ensure_expected_revision(
            current.revision,
            expected_revision,
            "knowledge source item",
        )?;
        validate_transition_timestamp(attempted_at, current.updated_at, "attempted_at")?;
        if current.state != KnowledgeSourceState::Active {
            return Err(DbError::Conflict(format!(
                "knowledge source item {knowledge_source_item_id} must be active before sync"
            )));
        }
        let source = lock_source(&mut transaction, &current.knowledge_source_id).await?;
        if source.state != KnowledgeSourceState::Active {
            return Err(DbError::Conflict(format!(
                "knowledge source {} must be active before sync",
                source.knowledge_source_id
            )));
        }

        let updated = sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "UPDATE knowledge_source_items SET \
                sync_status = 'syncing', last_attempt_at = ?, last_error = NULL, \
                pending_published_hash = NULL, pending_final_url = NULL, \
                pending_title = NULL, pending_publication_at = NULL, \
                revision = revision + 1, updated_at = ? \
             WHERE knowledge_source_item_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(attempted_at)
        .bind(attempted_at)
        .bind(knowledge_source_item_id.as_str())
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge source item {knowledge_source_item_id} changed before sync started"
            ))
        })?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn stage_sync_publication(
        &self,
        params: &StageKnowledgeSourcePublicationParams,
    ) -> Result<KnowledgeSourceItemRow, DbError> {
        validate_revision(params.expected_revision, "expected_revision")?;
        validate_hash(Some(&params.pending_published_hash))?;
        validate_optional_text(
            params.pending_final_url.as_deref(),
            "pending_final_url",
            MAX_URL_LEN,
            true,
        )?;
        validate_optional_text(
            params.pending_title.as_deref(),
            "pending_title",
            MAX_TITLE_LEN,
            true,
        )?;

        let mut transaction = self.pool.begin().await?;
        let current = lock_item(&mut transaction, &params.knowledge_source_item_id).await?;
        ensure_expected_revision(
            current.revision,
            params.expected_revision,
            "knowledge source item",
        )?;
        validate_transition_timestamp(params.staged_at, current.updated_at, "staged_at")?;
        if current.state != KnowledgeSourceState::Active
            || current.sync_status != KnowledgeSourceItemSyncStatus::Syncing
        {
            return Err(DbError::Conflict(format!(
                "knowledge source item {} cannot stage publication from {}/{}",
                params.knowledge_source_item_id,
                current.state.as_str(),
                current.sync_status.as_str()
            )));
        }
        if current.pending_published_hash.is_some() {
            return Err(DbError::Conflict(format!(
                "knowledge source item {} already has a staged publication",
                params.knowledge_source_item_id
            )));
        }
        let source = lock_source(&mut transaction, &current.knowledge_source_id).await?;
        if source.state != KnowledgeSourceState::Active {
            return Err(DbError::Conflict(format!(
                "knowledge source {} is no longer active",
                source.knowledge_source_id
            )));
        }

        let staged = sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "UPDATE knowledge_source_items SET \
                pending_published_hash = ?, pending_final_url = ?, pending_title = ?, \
                pending_publication_at = ?, revision = revision + 1, updated_at = ? \
             WHERE knowledge_source_item_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(&params.pending_published_hash)
        .bind(&params.pending_final_url)
        .bind(&params.pending_title)
        .bind(params.staged_at)
        .bind(params.staged_at)
        .bind(params.knowledge_source_item_id.as_str())
        .bind(params.expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge source item {} changed while publication was staged",
                params.knowledge_source_item_id
            ))
        })?;
        transaction.commit().await?;
        Ok(staged)
    }

    async fn record_sync_success(
        &self,
        params: &RecordKnowledgeSourceSyncSuccessParams,
    ) -> Result<KnowledgeSourceItemRow, DbError> {
        validate_revision(params.expected_revision, "expected_revision")?;
        validate_optional_text(params.final_url.as_deref(), "final_url", MAX_URL_LEN, true)?;
        validate_optional_text(params.title.as_deref(), "title", MAX_TITLE_LEN, true)?;
        validate_optional_text(params.etag.as_deref(), "etag", MAX_ETAG_LEN, false)?;
        validate_optional_text(
            params.http_last_modified.as_deref(),
            "http_last_modified",
            MAX_HTTP_LAST_MODIFIED_LEN,
            false,
        )?;
        validate_hash(Some(&params.last_published_hash))?;

        let mut transaction = self.pool.begin().await?;
        let current = lock_item(&mut transaction, &params.knowledge_source_item_id).await?;
        ensure_expected_revision(
            current.revision,
            params.expected_revision,
            "knowledge source item",
        )?;
        validate_transition_timestamp(params.succeeded_at, current.updated_at, "succeeded_at")?;
        if current.state != KnowledgeSourceState::Active
            || current.sync_status != KnowledgeSourceItemSyncStatus::Syncing
        {
            return Err(DbError::Conflict(format!(
                "knowledge source item {} cannot complete sync from {}/{}",
                params.knowledge_source_item_id,
                current.state.as_str(),
                current.sync_status.as_str()
            )));
        }
        match current.pending_published_hash.as_deref() {
            Some(pending_hash) => {
                if pending_hash != params.last_published_hash
                    || current.pending_final_url != params.final_url
                    || current.pending_title != params.title
                {
                    return Err(DbError::Conflict(format!(
                        "knowledge source item {} success does not match its staged publication",
                        params.knowledge_source_item_id
                    )));
                }
            }
            None if current.last_published_hash.is_none() => {
                // Compatibility seam for a pre-055 first publication. Once an
                // item has any committed baseline hash, every later success
                // must pass through `stage_sync_publication`.
            }
            None => {
                return Err(DbError::Conflict(format!(
                    "knowledge source item {} has no staged publication",
                    params.knowledge_source_item_id
                )));
            }
        }
        let source = lock_source(&mut transaction, &current.knowledge_source_id).await?;
        if source.state != KnowledgeSourceState::Active {
            return Err(DbError::Conflict(format!(
                "knowledge source {} is no longer active",
                source.knowledge_source_id
            )));
        }

        let updated = sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "UPDATE knowledge_source_items SET \
                final_url = ?, title = ?, sync_status = 'synced', etag = ?, \
                http_last_modified = ?, last_success_at = ?, last_error = NULL, \
                last_published_hash = ?, pending_published_hash = NULL, \
                pending_final_url = NULL, pending_title = NULL, pending_publication_at = NULL, \
                revision = revision + 1, updated_at = ? \
             WHERE knowledge_source_item_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(&params.final_url)
        .bind(&params.title)
        .bind(&params.etag)
        .bind(&params.http_last_modified)
        .bind(params.succeeded_at)
        .bind(&params.last_published_hash)
        .bind(params.succeeded_at)
        .bind(params.knowledge_source_item_id.as_str())
        .bind(params.expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge source item {} changed while sync was completing",
                params.knowledge_source_item_id
            ))
        })?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn record_sync_failure(
        &self,
        params: &RecordKnowledgeSourceSyncFailureParams,
    ) -> Result<KnowledgeSourceItemRow, DbError> {
        validate_revision(params.expected_revision, "expected_revision")?;
        if !matches!(
            params.status,
            KnowledgeSourceItemSyncStatus::Failed
                | KnowledgeSourceItemSyncStatus::Conflicted
                | KnowledgeSourceItemSyncStatus::Missing
        ) {
            return Err(DbError::Conflict(
                "knowledge source sync failure status must be failed, conflicted, or missing"
                    .into(),
            ));
        }
        validate_optional_text(
            Some(&params.error),
            "last_error",
            MAX_SYNC_ERROR_LEN,
            false,
        )?;

        let mut transaction = self.pool.begin().await?;
        let current = lock_item(&mut transaction, &params.knowledge_source_item_id).await?;
        ensure_expected_revision(
            current.revision,
            params.expected_revision,
            "knowledge source item",
        )?;
        validate_transition_timestamp(params.failed_at, current.updated_at, "failed_at")?;
        if current.state != KnowledgeSourceState::Active
            || current.sync_status != KnowledgeSourceItemSyncStatus::Syncing
        {
            return Err(DbError::Conflict(format!(
                "knowledge source item {} cannot fail sync from {}/{}",
                params.knowledge_source_item_id,
                current.state.as_str(),
                current.sync_status.as_str()
            )));
        }
        let source = lock_source(&mut transaction, &current.knowledge_source_id).await?;
        if source.state != KnowledgeSourceState::Active {
            return Err(DbError::Conflict(format!(
                "knowledge source {} is no longer active",
                source.knowledge_source_id
            )));
        }

        let updated = sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "UPDATE knowledge_source_items SET \
                sync_status = ?, last_error = ?, pending_published_hash = NULL, \
                pending_final_url = NULL, pending_title = NULL, pending_publication_at = NULL, \
                revision = revision + 1, updated_at = ? \
             WHERE knowledge_source_item_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(params.status.as_str())
        .bind(&params.error)
        .bind(params.failed_at)
        .bind(params.knowledge_source_item_id.as_str())
        .bind(params.expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge source item {} changed while sync failure was recorded",
                params.knowledge_source_item_id
            ))
        })?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn remove_source_item(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
        expected_revision: i64,
        removed_at: TimestampMs,
    ) -> Result<KnowledgeSourceItemRow, DbError> {
        validate_revision(expected_revision, "expected_revision")?;
        let mut transaction = self.pool.begin().await?;
        let current = lock_item(&mut transaction, knowledge_source_item_id).await?;
        ensure_expected_revision(
            current.revision,
            expected_revision,
            "knowledge source item",
        )?;
        validate_transition_timestamp(removed_at, current.updated_at, "removed_at")?;
        if current.is_removed() {
            transaction.commit().await?;
            return Ok(current);
        }
        ensure_no_managed_provenance(&mut transaction, knowledge_source_item_id).await?;

        let updated = sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "UPDATE knowledge_source_items SET \
                state = 'removed', sync_status = CASE \
                    WHEN sync_status = 'syncing' THEN 'missing' ELSE sync_status END, \
                removed_at = ?, pending_published_hash = NULL, \
                pending_final_url = NULL, pending_title = NULL, pending_publication_at = NULL, \
                revision = revision + 1, updated_at = ? \
             WHERE knowledge_source_item_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(removed_at)
        .bind(removed_at)
        .bind(knowledge_source_item_id.as_str())
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge source item {knowledge_source_item_id} changed during removal"
            ))
        })?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn bind_managed_entry(
        &self,
        params: &BindManagedKnowledgeEntryParams,
    ) -> Result<KnowledgeEntryProvenanceRow, DbError> {
        validate_timestamp(params.created_at, "provenance created_at")?;
        let mut transaction = self.pool.begin().await?;
        let item = lock_item(&mut transaction, &params.knowledge_source_item_id).await?;
        if item.is_removed() {
            return Err(DbError::Conflict(format!(
                "cannot bind a managed entry to removed source item {}",
                params.knowledge_source_item_id
            )));
        }
        validate_entry_and_item_scope(
            &mut transaction,
            &params.knowledge_entry_id,
            &params.knowledge_source_item_id,
            true,
        )
        .await?;

        if let Some(existing) = fetch_provenance(&mut transaction, &params.knowledge_entry_id).await?
        {
            if existing.knowledge_source_item_id == params.knowledge_source_item_id
                && existing.relationship == KnowledgeEntryProvenanceRelationship::Managed
            {
                transaction.commit().await?;
                return Ok(existing);
            }
            return Err(DbError::Conflict(format!(
                "knowledge entry {} already has different source provenance",
                params.knowledge_entry_id
            )));
        }

        let provenance = sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
            "INSERT INTO knowledge_entry_provenance (\
                knowledge_entry_id, knowledge_source_item_id, relationship, derived_from_entry_id, \
                revision, detached_at, created_at, updated_at\
             ) VALUES (?, ?, 'managed', NULL, 0, NULL, ?, ?) \
             RETURNING *",
        )
        .bind(params.knowledge_entry_id.as_str())
        .bind(params.knowledge_source_item_id.as_str())
        .bind(params.created_at)
        .bind(params.created_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(source_query_error)?;
        transaction.commit().await?;
        Ok(provenance)
    }

    async fn detach_managed_entry(
        &self,
        knowledge_entry_id: &KnowledgeEntryId,
        expected_revision: i64,
        detached_at: TimestampMs,
    ) -> Result<KnowledgeEntryProvenanceRow, DbError> {
        validate_revision(expected_revision, "expected_revision")?;
        let mut transaction = self.pool.begin().await?;
        let current = lock_provenance(&mut transaction, knowledge_entry_id).await?;
        ensure_expected_revision(
            current.revision,
            expected_revision,
            "knowledge entry provenance",
        )?;
        if current.relationship != KnowledgeEntryProvenanceRelationship::Managed {
            return Err(DbError::Conflict(format!(
                "knowledge entry {knowledge_entry_id} is not source-managed"
            )));
        }
        let item = lock_item(&mut transaction, &current.knowledge_source_item_id).await?;
        validate_transition_timestamp(
            detached_at,
            current.updated_at.max(item.updated_at),
            "detached_at",
        )?;
        if item.is_removed() {
            return Err(DbError::Conflict(format!(
                "knowledge source item {} is already removed",
                item.knowledge_source_item_id
            )));
        }
        if item.sync_status == KnowledgeSourceItemSyncStatus::Syncing {
            return Err(DbError::Conflict(format!(
                "knowledge source item {} is currently syncing",
                item.knowledge_source_item_id
            )));
        }

        sqlx::query(
            "UPDATE knowledge_source_items SET \
                state = 'paused', revision = revision + 1, updated_at = ? \
             WHERE knowledge_source_item_id = ?",
        )
        .bind(detached_at)
        .bind(item.knowledge_source_item_id.as_str())
        .execute(&mut *transaction)
        .await?;
        let detached = sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
            "UPDATE knowledge_entry_provenance SET \
                relationship = 'detached', detached_at = ?, revision = revision + 1, updated_at = ? \
             WHERE knowledge_entry_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(detached_at)
        .bind(detached_at)
        .bind(knowledge_entry_id.as_str())
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge entry provenance {knowledge_entry_id} changed during detach"
            ))
        })?;
        transaction.commit().await?;
        Ok(detached)
    }

    async fn remove_managed_source_item(
        &self,
        knowledge_entry_id: &KnowledgeEntryId,
        expected_provenance_revision: i64,
        expected_item_revision: i64,
        removed_at: TimestampMs,
    ) -> Result<(KnowledgeEntryProvenanceRow, KnowledgeSourceItemRow), DbError> {
        validate_revision(
            expected_provenance_revision,
            "expected_provenance_revision",
        )?;
        validate_revision(expected_item_revision, "expected_item_revision")?;
        validate_timestamp(removed_at, "removed_at")?;

        let mut transaction = self.pool.begin().await?;
        let provenance = lock_provenance(&mut transaction, knowledge_entry_id).await?;
        let item = lock_item(
            &mut transaction,
            &provenance.knowledge_source_item_id,
        )
        .await?;

        // Fully committed replay: the original expected revisions are exactly
        // one behind both terminal rows. Also accept callers that persisted the
        // terminal revisions before losing the response.
        if provenance.relationship == KnowledgeEntryProvenanceRelationship::Detached
            && item.state == KnowledgeSourceState::Removed
            && item.removed_at == Some(removed_at)
            && is_expected_or_one_ahead(
                provenance.revision,
                expected_provenance_revision,
            )
            && is_expected_or_one_ahead(item.revision, expected_item_revision)
        {
            transaction.commit().await?;
            return Ok((provenance, item));
        }

        if !matches!(
            provenance.relationship,
            KnowledgeEntryProvenanceRelationship::Managed
                | KnowledgeEntryProvenanceRelationship::Detached
        ) {
            return Err(DbError::Conflict(format!(
                "knowledge entry {knowledge_entry_id} is not a removable managed source document"
            )));
        }
        let provenance_was_managed =
            provenance.relationship == KnowledgeEntryProvenanceRelationship::Managed;
        if provenance_was_managed {
            ensure_expected_revision(
                provenance.revision,
                expected_provenance_revision,
                "knowledge entry provenance",
            )?;
        } else if !is_expected_or_one_ahead(
            provenance.revision,
            expected_provenance_revision,
        ) {
            return Err(DbError::Conflict(format!(
                "knowledge entry provenance revision conflict: expected {expected_provenance_revision} (or its completed detach), current {}",
                provenance.revision
            )));
        }
        ensure_expected_revision(
            item.revision,
            expected_item_revision,
            "knowledge source item",
        )?;
        validate_transition_timestamp(
            removed_at,
            provenance.updated_at.max(item.updated_at),
            "removed_at",
        )?;
        if item.is_removed() {
            return Err(DbError::Conflict(format!(
                "knowledge source item {} was removed by another transition",
                item.knowledge_source_item_id
            )));
        }
        let provenance = if provenance_was_managed {
            sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
                "UPDATE knowledge_entry_provenance SET \
                    relationship = 'detached', detached_at = ?, revision = revision + 1, \
                    updated_at = ? \
                 WHERE knowledge_entry_id = ? AND revision = ? \
                 RETURNING *",
            )
            .bind(removed_at)
            .bind(removed_at)
            .bind(knowledge_entry_id.as_str())
            .bind(expected_provenance_revision)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(source_query_error)?
            .ok_or_else(|| {
                DbError::Conflict(format!(
                    "knowledge entry provenance {knowledge_entry_id} changed during source removal"
                ))
            })?
        } else {
            provenance
        };
        let item = sqlx::query_as::<_, KnowledgeSourceItemRow>(
            "UPDATE knowledge_source_items SET \
                state = 'removed', sync_status = CASE \
                    WHEN sync_status = 'syncing' THEN 'missing' ELSE sync_status END, \
                removed_at = ?, pending_published_hash = NULL, \
                pending_final_url = NULL, pending_title = NULL, pending_publication_at = NULL, \
                revision = revision + 1, updated_at = ? \
             WHERE knowledge_source_item_id = ? AND revision = ? \
             RETURNING *",
        )
        .bind(removed_at)
        .bind(removed_at)
        .bind(item.knowledge_source_item_id.as_str())
        .bind(expected_item_revision)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(source_query_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge source item {} changed during source removal",
                item.knowledge_source_item_id
            ))
        })?;
        transaction.commit().await?;
        Ok((provenance, item))
    }

    async fn record_entry_copy(
        &self,
        params: &RecordKnowledgeEntryCopyParams,
    ) -> Result<KnowledgeEntryProvenanceRow, DbError> {
        validate_timestamp(params.created_at, "copy created_at")?;
        if params.knowledge_entry_id == params.derived_from_entry_id {
            return Err(DbError::Conflict(
                "knowledge entry copy cannot derive from itself".into(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let item = lock_item(&mut transaction, &params.knowledge_source_item_id).await?;
        if item.is_removed() {
            return Err(DbError::Conflict(format!(
                "cannot create a copy from removed source item {}",
                params.knowledge_source_item_id
            )));
        }
        validate_entry_and_item_scope(
            &mut transaction,
            &params.knowledge_entry_id,
            &params.knowledge_source_item_id,
            true,
        )
        .await?;
        validate_entry_and_item_scope(
            &mut transaction,
            &params.derived_from_entry_id,
            &params.knowledge_source_item_id,
            true,
        )
        .await?;
        let managed_origin: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM knowledge_entry_provenance \
             WHERE knowledge_entry_id = ? AND knowledge_source_item_id = ? \
               AND relationship = 'managed')",
        )
        .bind(params.derived_from_entry_id.as_str())
        .bind(params.knowledge_source_item_id.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        if !managed_origin {
            return Err(DbError::Conflict(format!(
                "knowledge copy origin {} is not the managed entry for source item {}",
                params.derived_from_entry_id, params.knowledge_source_item_id
            )));
        }

        if let Some(existing) = fetch_provenance(&mut transaction, &params.knowledge_entry_id).await?
        {
            if existing.knowledge_source_item_id == params.knowledge_source_item_id
                && existing.relationship == KnowledgeEntryProvenanceRelationship::Copy
                && existing.derived_from_entry_id.as_ref() == Some(&params.derived_from_entry_id)
            {
                transaction.commit().await?;
                return Ok(existing);
            }
            return Err(DbError::Conflict(format!(
                "knowledge entry {} already has different source provenance",
                params.knowledge_entry_id
            )));
        }

        let provenance = sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
            "INSERT INTO knowledge_entry_provenance (\
                knowledge_entry_id, knowledge_source_item_id, relationship, derived_from_entry_id, \
                revision, detached_at, created_at, updated_at\
             ) VALUES (?, ?, 'copy', ?, 0, NULL, ?, ?) \
             RETURNING *",
        )
        .bind(params.knowledge_entry_id.as_str())
        .bind(params.knowledge_source_item_id.as_str())
        .bind(params.derived_from_entry_id.as_str())
        .bind(params.created_at)
        .bind(params.created_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(source_query_error)?;
        transaction.commit().await?;
        Ok(provenance)
    }

    async fn get_entry_provenance(
        &self,
        knowledge_entry_id: &KnowledgeEntryId,
    ) -> Result<Option<KnowledgeEntryProvenanceRow>, DbError> {
        sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
            "SELECT * FROM knowledge_entry_provenance WHERE knowledge_entry_id = ?",
        )
        .bind(knowledge_entry_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn get_managed_entry_provenance(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
    ) -> Result<Option<KnowledgeEntryProvenanceRow>, DbError> {
        sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
            "SELECT * FROM knowledge_entry_provenance \
             WHERE knowledge_source_item_id = ? AND relationship = 'managed'",
        )
        .bind(knowledge_source_item_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn list_entry_provenance_for_source(
        &self,
        knowledge_source_id: &KnowledgeSourceId,
    ) -> Result<Vec<KnowledgeEntryProvenanceRow>, DbError> {
        sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
            "SELECT provenance.* FROM knowledge_entry_provenance provenance \
             JOIN knowledge_source_items item \
               ON item.knowledge_source_item_id = provenance.knowledge_source_item_id \
             WHERE item.knowledge_source_id = ? \
             ORDER BY item.ordinal, provenance.relationship, provenance.knowledge_entry_id",
        )
        .bind(knowledge_source_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn list_entry_provenance_for_item(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
    ) -> Result<Vec<KnowledgeEntryProvenanceRow>, DbError> {
        sqlx::query_as::<_, KnowledgeEntryProvenanceRow>(
            "SELECT * FROM knowledge_entry_provenance \
             WHERE knowledge_source_item_id = ? \
             ORDER BY relationship, knowledge_entry_id",
        )
        .bind(knowledge_source_item_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }
}
