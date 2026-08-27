use std::collections::{HashMap, HashSet};

use nomifun_common::{KnowledgeBaseId, KnowledgeEntryId};
use sqlx::{Sqlite, Transaction};

use crate::error::DbError;
use crate::models::{
    KNOWLEDGE_ENTRY_KIND_DIRECTORY, KNOWLEDGE_ENTRY_KIND_FILE,
    KNOWLEDGE_ENTRY_ORIGIN_GENERATED, KNOWLEDGE_ENTRY_ORIGIN_URL_SNAPSHOT,
    KNOWLEDGE_ENTRY_ORIGIN_USER, KnowledgeEntryRow,
};
use crate::repository::knowledge_entry::{
    IKnowledgeEntryRepository, KnowledgeEntryMutation, KnowledgeProjectionReplacement,
    RelocateKnowledgeEntryProjectionParams, UpsertKnowledgeEntryParams,
};
use crate::repository::sqlite_knowledge::SqliteKnowledgeRepository;

fn projection_error(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(database_error)
            if database_error
                .message()
                .to_ascii_lowercase()
                .contains("constraint failed") =>
        {
            DbError::Conflict(format!(
                "knowledge entry projection violates a uniqueness or value constraint: {}",
                database_error.message()
            ))
        }
        _ => DbError::Query(error),
    }
}

fn validate_name(name: &str) -> Result<(), DbError> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', '\0'])
    {
        return Err(DbError::Conflict(
            "knowledge entry name must be one non-empty path segment".into(),
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
            "knowledge entry {label} must be a canonical non-empty relative path"
        )));
    }
    Ok(())
}

fn validate_entry_values(params: &UpsertKnowledgeEntryParams) -> Result<(), DbError> {
    validate_name(&params.name)?;
    validate_rel_path(&params.rel_path, "rel_path")?;
    validate_rel_path(&params.portable_rel_path, "portable_rel_path")?;
    if params.rel_path.rsplit('/').next() != Some(params.name.as_str()) {
        return Err(DbError::Conflict(
            "knowledge entry rel_path must end with its exact name".into(),
        ));
    }
    if !matches!(
        params.kind.as_str(),
        KNOWLEDGE_ENTRY_KIND_FILE | KNOWLEDGE_ENTRY_KIND_DIRECTORY
    ) {
        return Err(DbError::Conflict(format!(
            "unsupported knowledge entry kind '{}'",
            params.kind
        )));
    }
    if !matches!(
        params.origin.as_str(),
        KNOWLEDGE_ENTRY_ORIGIN_USER
            | KNOWLEDGE_ENTRY_ORIGIN_URL_SNAPSHOT
            | KNOWLEDGE_ENTRY_ORIGIN_GENERATED
    ) {
        return Err(DbError::Conflict(format!(
            "unsupported knowledge entry origin '{}'",
            params.origin
        )));
    }
    if params.revision < 0 {
        return Err(DbError::Conflict(
            "knowledge entry revision must be non-negative".into(),
        ));
    }
    if params.updated_at < params.created_at
        || params
            .deleted_at
            .is_some_and(|deleted_at| deleted_at < params.created_at)
    {
        return Err(DbError::Conflict(
            "knowledge entry timestamps are inconsistent".into(),
        ));
    }
    if params.parent_entry_id.as_ref() == Some(&params.knowledge_entry_id) {
        return Err(DbError::Conflict(
            "knowledge entry cannot be its own parent".into(),
        ));
    }
    for (label, value) in [
        ("fs_identity", params.fs_identity.as_deref()),
        ("content_hash", params.content_hash.as_deref()),
    ] {
        if value.is_some_and(str::is_empty) {
            return Err(DbError::Conflict(format!(
                "knowledge entry {label} cannot be empty when present"
            )));
        }
    }
    Ok(())
}

fn child_path_matches(parent_path: &str, child_path: &str, child_name: &str) -> bool {
    child_path
        .strip_prefix(parent_path)
        .is_some_and(|suffix| suffix == format!("/{child_name}"))
}

fn portable_child_path_matches(parent_path: &str, child_path: &str) -> bool {
    child_path
        .strip_prefix(parent_path)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .is_some_and(|segment| !segment.is_empty() && !segment.contains('/'))
}

async fn lock_base(
    tx: &mut Transaction<'_, Sqlite>,
    knowledge_base_id: &KnowledgeBaseId,
) -> Result<i64, DbError> {
    sqlx::query_scalar::<_, i64>(
        "UPDATE knowledge_bases SET tree_revision = tree_revision \
         WHERE knowledge_base_id = ? RETURNING tree_revision",
    )
    .bind(knowledge_base_id.as_str())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        DbError::NotFound(format!("knowledge base '{}'", knowledge_base_id.as_str()))
    })
}

async fn bump_tree_revision(
    tx: &mut Transaction<'_, Sqlite>,
    knowledge_base_id: &KnowledgeBaseId,
) -> Result<i64, DbError> {
    sqlx::query_scalar::<_, i64>(
        "UPDATE knowledge_bases SET tree_revision = tree_revision + 1 \
         WHERE knowledge_base_id = ? AND tree_revision < 9223372036854775807 \
         RETURNING tree_revision",
    )
    .bind(knowledge_base_id.as_str())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        DbError::Conflict(format!(
            "knowledge base '{}' tree revision overflowed or disappeared",
            knowledge_base_id.as_str()
        ))
    })
}

async fn fetch_entry(
    tx: &mut Transaction<'_, Sqlite>,
    knowledge_base_id: &KnowledgeBaseId,
    knowledge_entry_id: &KnowledgeEntryId,
) -> Result<Option<KnowledgeEntryRow>, DbError> {
    sqlx::query_as::<_, KnowledgeEntryRow>(
        "SELECT * FROM knowledge_entries \
         WHERE knowledge_base_id = ? AND knowledge_entry_id = ?",
    )
    .bind(knowledge_base_id.as_str())
    .bind(knowledge_entry_id.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::Query)
}

async fn validate_live_parent(
    tx: &mut Transaction<'_, Sqlite>,
    knowledge_base_id: &KnowledgeBaseId,
    parent_entry_id: Option<&KnowledgeEntryId>,
    name: &str,
    rel_path: &str,
    portable_rel_path: &str,
) -> Result<Option<KnowledgeEntryRow>, DbError> {
    let Some(parent_entry_id) = parent_entry_id else {
        if rel_path != name || portable_rel_path.contains('/') {
            return Err(DbError::Conflict(
                "a root knowledge entry path must contain exactly its name".into(),
            ));
        }
        return Ok(None);
    };
    let parent = fetch_entry(tx, knowledge_base_id, parent_entry_id)
        .await?
        .filter(|entry| !entry.is_deleted())
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "destination parent knowledge entry '{}' is absent or deleted",
                parent_entry_id.as_str()
            ))
        })?;
    if !parent.is_directory() {
        return Err(DbError::Conflict(format!(
            "destination knowledge entry '{}' is not a directory",
            parent_entry_id.as_str()
        )));
    }
    if !child_path_matches(&parent.rel_path, rel_path, name)
        || !portable_child_path_matches(&parent.portable_rel_path, portable_rel_path)
    {
        return Err(DbError::Conflict(
            "knowledge entry path does not match its parent and name".into(),
        ));
    }
    Ok(Some(parent))
}

async fn upsert_projection_row(
    tx: &mut Transaction<'_, Sqlite>,
    params: &UpsertKnowledgeEntryParams,
) -> Result<KnowledgeEntryRow, DbError> {
    sqlx::query_as::<_, KnowledgeEntryRow>(
        "INSERT INTO knowledge_entries (\
            knowledge_entry_id, knowledge_base_id, parent_entry_id, name, kind, origin, \
            rel_path, portable_rel_path, fs_identity, content_hash, revision, deleted_at, \
            created_at, updated_at\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(knowledge_entry_id) DO UPDATE SET \
            parent_entry_id = excluded.parent_entry_id, name = excluded.name, \
            kind = excluded.kind, origin = excluded.origin, rel_path = excluded.rel_path, \
            portable_rel_path = excluded.portable_rel_path, \
            fs_identity = excluded.fs_identity, content_hash = excluded.content_hash, \
            revision = excluded.revision, deleted_at = excluded.deleted_at, \
            created_at = MIN(knowledge_entries.created_at, excluded.created_at), \
            updated_at = excluded.updated_at \
         WHERE knowledge_entries.knowledge_base_id = excluded.knowledge_base_id \
         RETURNING *",
    )
    .bind(params.knowledge_entry_id.as_str())
    .bind(params.knowledge_base_id.as_str())
    .bind(params.parent_entry_id.as_ref().map(KnowledgeEntryId::as_str))
    .bind(&params.name)
    .bind(&params.kind)
    .bind(&params.origin)
    .bind(&params.rel_path)
    .bind(&params.portable_rel_path)
    .bind(&params.fs_identity)
    .bind(&params.content_hash)
    .bind(params.revision)
    .bind(params.deleted_at)
    .bind(params.created_at)
    .bind(params.updated_at)
    .fetch_one(&mut **tx)
    .await
    .map_err(projection_error)
}

fn validate_projection_snapshot(
    knowledge_base_id: &KnowledgeBaseId,
    entries: &[UpsertKnowledgeEntryParams],
) -> Result<(), DbError> {
    let mut by_id = HashMap::with_capacity(entries.len());
    let mut live_paths = HashSet::new();
    let mut live_portable_paths = HashSet::new();
    for entry in entries {
        validate_entry_values(entry)?;
        if &entry.knowledge_base_id != knowledge_base_id {
            return Err(DbError::Conflict(format!(
                "projection entry '{}' belongs to another knowledge base",
                entry.knowledge_entry_id.as_str()
            )));
        }
        if by_id
            .insert(entry.knowledge_entry_id.as_str(), entry)
            .is_some()
        {
            return Err(DbError::Conflict(format!(
                "projection contains duplicate knowledge entry '{}'",
                entry.knowledge_entry_id.as_str()
            )));
        }
        if entry.deleted_at.is_none()
            && (!live_paths.insert(entry.rel_path.as_str())
                || !live_portable_paths.insert(entry.portable_rel_path.as_str()))
        {
            return Err(DbError::Conflict(format!(
                "projection contains a duplicate live path near '{}'",
                entry.rel_path
            )));
        }
    }

    for entry in entries.iter().filter(|entry| entry.deleted_at.is_none()) {
        let Some(parent_entry_id) = entry.parent_entry_id.as_ref() else {
            if entry.rel_path != entry.name || entry.portable_rel_path.contains('/') {
                return Err(DbError::Conflict(format!(
                    "root projection entry '{}' has a nested path",
                    entry.knowledge_entry_id.as_str()
                )));
            }
            continue;
        };
        let parent = by_id
            .get(parent_entry_id.as_str())
            .copied()
            .filter(|parent| parent.deleted_at.is_none())
            .ok_or_else(|| {
                DbError::Conflict(format!(
                    "live projection entry '{}' has a missing or deleted parent",
                    entry.knowledge_entry_id.as_str()
                ))
            })?;
        if parent.kind != KNOWLEDGE_ENTRY_KIND_DIRECTORY
            || !child_path_matches(&parent.rel_path, &entry.rel_path, &entry.name)
            || !portable_child_path_matches(
                &parent.portable_rel_path,
                &entry.portable_rel_path,
            )
        {
            return Err(DbError::Conflict(format!(
                "projection entry '{}' has an invalid parent/path relationship",
                entry.knowledge_entry_id.as_str()
            )));
        }
    }
    Ok(())
}

#[async_trait::async_trait]
impl IKnowledgeEntryRepository for SqliteKnowledgeRepository {
    async fn get_entry(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        knowledge_entry_id: &KnowledgeEntryId,
    ) -> Result<Option<KnowledgeEntryRow>, DbError> {
        sqlx::query_as::<_, KnowledgeEntryRow>(
            "SELECT * FROM knowledge_entries \
             WHERE knowledge_base_id = ? AND knowledge_entry_id = ?",
        )
        .bind(knowledge_base_id.as_str())
        .bind(knowledge_entry_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn get_entry_by_path(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        portable_rel_path: &str,
    ) -> Result<Option<KnowledgeEntryRow>, DbError> {
        validate_rel_path(portable_rel_path, "portable_rel_path")?;
        sqlx::query_as::<_, KnowledgeEntryRow>(
            "SELECT * FROM knowledge_entries \
             WHERE knowledge_base_id = ? AND portable_rel_path = ? AND deleted_at IS NULL",
        )
        .bind(knowledge_base_id.as_str())
        .bind(portable_rel_path)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn list_entries_for_base(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        include_deleted: bool,
    ) -> Result<Vec<KnowledgeEntryRow>, DbError> {
        let sql = if include_deleted {
            "SELECT * FROM knowledge_entries WHERE knowledge_base_id = ? \
             ORDER BY portable_rel_path, knowledge_entry_id"
        } else {
            "SELECT * FROM knowledge_entries \
             WHERE knowledge_base_id = ? AND deleted_at IS NULL \
             ORDER BY portable_rel_path, knowledge_entry_id"
        };
        sqlx::query_as::<_, KnowledgeEntryRow>(sql)
            .bind(knowledge_base_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn tree_revision(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
    ) -> Result<i64, DbError> {
        sqlx::query_scalar("SELECT tree_revision FROM knowledge_bases WHERE knowledge_base_id = ?")
            .bind(knowledge_base_id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| {
                DbError::NotFound(format!("knowledge base '{}'", knowledge_base_id.as_str()))
            })
    }

    async fn upsert_entry(
        &self,
        params: &UpsertKnowledgeEntryParams,
    ) -> Result<KnowledgeEntryMutation, DbError> {
        validate_entry_values(params)?;
        let mut tx = self.pool.begin().await?;
        lock_base(&mut tx, &params.knowledge_base_id).await?;
        if params.deleted_at.is_none() {
            validate_live_parent(
                &mut tx,
                &params.knowledge_base_id,
                params.parent_entry_id.as_ref(),
                &params.name,
                &params.rel_path,
                &params.portable_rel_path,
            )
            .await?;
        }

        let existing = fetch_entry(
            &mut tx,
            &params.knowledge_base_id,
            &params.knowledge_entry_id,
        )
        .await?;
        if existing
            .as_ref()
            .is_some_and(|entry| !entry.is_deleted() && entry.rel_path != params.rel_path)
        {
            return Err(DbError::Conflict(
                "an existing knowledge entry path must be changed through relocate_entry".into(),
            ));
        }
        if existing.as_ref().is_some_and(|entry| {
            entry.is_directory() && params.kind != KNOWLEDGE_ENTRY_KIND_DIRECTORY
        }) {
            let has_live_children: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM knowledge_entries \
                 WHERE knowledge_base_id = ? AND parent_entry_id = ? AND deleted_at IS NULL)",
            )
            .bind(params.knowledge_base_id.as_str())
            .bind(params.knowledge_entry_id.as_str())
            .fetch_one(&mut *tx)
            .await?;
            if has_live_children {
                return Err(DbError::Conflict(
                    "a knowledge directory with live children cannot become a file".into(),
                ));
            }
        }

        let entry = sqlx::query_as::<_, KnowledgeEntryRow>(
            "INSERT INTO knowledge_entries (\
                knowledge_entry_id, knowledge_base_id, parent_entry_id, name, kind, origin, \
                rel_path, portable_rel_path, fs_identity, content_hash, revision, deleted_at, \
                created_at, updated_at\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(knowledge_entry_id) DO UPDATE SET \
                parent_entry_id = excluded.parent_entry_id, name = excluded.name, \
                kind = excluded.kind, origin = excluded.origin, rel_path = excluded.rel_path, \
                portable_rel_path = excluded.portable_rel_path, \
                fs_identity = excluded.fs_identity, content_hash = excluded.content_hash, \
                revision = excluded.revision, deleted_at = excluded.deleted_at, \
                created_at = excluded.created_at, updated_at = excluded.updated_at \
             WHERE knowledge_entries.knowledge_base_id = excluded.knowledge_base_id \
             RETURNING *",
        )
        .bind(params.knowledge_entry_id.as_str())
        .bind(params.knowledge_base_id.as_str())
        .bind(params.parent_entry_id.as_ref().map(KnowledgeEntryId::as_str))
        .bind(&params.name)
        .bind(&params.kind)
        .bind(&params.origin)
        .bind(&params.rel_path)
        .bind(&params.portable_rel_path)
        .bind(&params.fs_identity)
        .bind(&params.content_hash)
        .bind(params.revision)
        .bind(params.deleted_at)
        .bind(params.created_at)
        .bind(params.updated_at)
        .fetch_optional(&mut *tx)
        .await
        .map_err(projection_error)?
        .ok_or_else(|| {
            DbError::Conflict(format!(
                "knowledge entry '{}' is already owned by another knowledge base",
                params.knowledge_entry_id.as_str()
            ))
        })?;
        let tree_revision = bump_tree_revision(&mut tx, &params.knowledge_base_id).await?;
        tx.commit().await?;
        Ok(KnowledgeEntryMutation {
            entry,
            affected_entries: 1,
            tree_revision,
        })
    }

    async fn replace_projection(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        expected_tree_revision: Option<i64>,
        entries: &[UpsertKnowledgeEntryParams],
    ) -> Result<KnowledgeProjectionReplacement, DbError> {
        if expected_tree_revision.is_some_and(|revision| revision < 0) {
            return Err(DbError::Conflict(
                "expected knowledge tree revision must be non-negative".into(),
            ));
        }
        validate_projection_snapshot(knowledge_base_id, entries)?;
        let mut tx = self.pool.begin().await?;
        let current_tree_revision = lock_base(&mut tx, knowledge_base_id).await?;
        if expected_tree_revision.is_some_and(|expected| expected != current_tree_revision) {
            return Err(DbError::Conflict(format!(
                "knowledge base '{}' tree revision conflict: expected {:?}, current {}",
                knowledge_base_id.as_str(),
                expected_tree_revision,
                current_tree_revision
            )));
        }
        // The projection is rebuildable, but its stable IDs are also anchors
        // for source provenance and editor sessions. Keep missing rows as
        // tombstones instead of deleting the whole base projection. A later
        // marker/inode/path match can then resurrect the same identity.
        let incoming_ids = entries
            .iter()
            .map(|entry| entry.knowledge_entry_id.as_str())
            .collect::<HashSet<_>>();
        let current_live = sqlx::query_as::<_, KnowledgeEntryRow>(
            "SELECT * FROM knowledge_entries \
             WHERE knowledge_base_id = ? AND deleted_at IS NULL",
        )
        .bind(knowledge_base_id.as_str())
        .fetch_all(&mut *tx)
        .await?;
        let scan_time = entries
            .iter()
            .map(|entry| entry.updated_at)
            .max()
            .unwrap_or_else(nomifun_common::now_ms);
        for missing in current_live.iter().filter(|entry| {
            !incoming_ids.contains(entry.knowledge_entry_id.as_str())
        }) {
            let deleted_at = scan_time.max(missing.updated_at).max(missing.created_at);
            sqlx::query(
                "UPDATE knowledge_entries SET deleted_at = ?, revision = revision + 1, \
                    updated_at = ? \
                 WHERE knowledge_base_id = ? AND knowledge_entry_id = ? AND deleted_at IS NULL",
            )
            .bind(deleted_at)
            .bind(deleted_at)
            .bind(knowledge_base_id.as_str())
            .bind(missing.knowledge_entry_id.as_str())
            .execute(&mut *tx)
            .await?;
            // A source's placement default is a live-directory preference, not
            // historical provenance. Clear it in the same transaction that
            // tombstones the directory so the logical SET_NULL contract never
            // exposes a dangling or deleted parent.
            sqlx::query(
                "UPDATE knowledge_sources SET \
                    default_parent_entry_id = NULL, revision = revision + 1, \
                    updated_at = MAX(updated_at, ?) \
                 WHERE knowledge_base_id = ? AND default_parent_entry_id = ?",
            )
            .bind(deleted_at)
            .bind(knowledge_base_id.as_str())
            .bind(missing.knowledge_entry_id.as_str())
            .execute(&mut *tx)
            .await?;
        }
        for entry in entries {
            upsert_projection_row(&mut tx, entry).await?;
        }
        let tree_revision = bump_tree_revision(&mut tx, knowledge_base_id).await?;
        tx.commit().await?;
        Ok(KnowledgeProjectionReplacement {
            replaced_entries: entries.len() as u64,
            tree_revision,
        })
    }

    async fn relocate_entry(
        &self,
        params: &RelocateKnowledgeEntryProjectionParams,
    ) -> Result<KnowledgeEntryMutation, DbError> {
        validate_name(&params.new_name)?;
        validate_rel_path(&params.new_rel_path, "new_rel_path")?;
        validate_rel_path(&params.new_portable_rel_path, "new_portable_rel_path")?;
        if params.expected_revision < 0 {
            return Err(DbError::Conflict(
                "expected knowledge entry revision must be non-negative".into(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        let tree_revision = lock_base(&mut tx, &params.knowledge_base_id).await?;
        let entry = fetch_entry(
            &mut tx,
            &params.knowledge_base_id,
            &params.knowledge_entry_id,
        )
        .await?
        .filter(|entry| !entry.is_deleted())
        .ok_or_else(|| {
            DbError::NotFound(format!(
                "live knowledge entry '{}'",
                params.knowledge_entry_id.as_str()
            ))
        })?;
        if entry.revision != params.expected_revision {
            return Err(DbError::Conflict(format!(
                "knowledge entry '{}' revision conflict: expected {}, current {}",
                params.knowledge_entry_id.as_str(),
                params.expected_revision,
                entry.revision
            )));
        }
        if params.destination_parent_entry_id.as_ref() == Some(&params.knowledge_entry_id) {
            return Err(DbError::Conflict(
                "knowledge entry cannot be moved into itself".into(),
            ));
        }
        let parent = validate_live_parent(
            &mut tx,
            &params.knowledge_base_id,
            params.destination_parent_entry_id.as_ref(),
            &params.new_name,
            &params.new_rel_path,
            &params.new_portable_rel_path,
        )
        .await?;
        if parent.as_ref().is_some_and(|parent| {
            parent.rel_path == entry.rel_path
                || parent
                    .rel_path
                    .strip_prefix(&entry.rel_path)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }) {
            return Err(DbError::Conflict(
                "knowledge directory cannot be moved into its own subtree".into(),
            ));
        }
        if params.new_rel_path.rsplit('/').next() != Some(params.new_name.as_str()) {
            return Err(DbError::Conflict(
                "new knowledge entry path must end with its exact name".into(),
            ));
        }
        if entry.parent_entry_id == params.destination_parent_entry_id
            && entry.name == params.new_name
            && entry.rel_path == params.new_rel_path
            && entry.portable_rel_path == params.new_portable_rel_path
        {
            tx.commit().await?;
            return Ok(KnowledgeEntryMutation {
                entry,
                affected_entries: 0,
                tree_revision,
            });
        }

        let destination_conflict: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM knowledge_entries \
             WHERE knowledge_base_id = ? AND portable_rel_path = ? \
               AND deleted_at IS NULL AND knowledge_entry_id <> ?)",
        )
        .bind(params.knowledge_base_id.as_str())
        .bind(&params.new_portable_rel_path)
        .bind(params.knowledge_entry_id.as_str())
        .fetch_one(&mut *tx)
        .await?;
        if destination_conflict {
            return Err(DbError::Conflict(format!(
                "knowledge destination '{}' already exists",
                params.new_rel_path
            )));
        }

        let old_rel_path = entry.rel_path.clone();
        let old_portable_rel_path = entry.portable_rel_path.clone();
        let inconsistent_subtree: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
                 SELECT 1 FROM knowledge_entries \
                 WHERE knowledge_base_id = ? AND deleted_at IS NULL AND (\
                     ((rel_path = ? OR substr(rel_path, 1, length(?) + 1) = ? || '/') \
                      AND NOT (portable_rel_path = ? OR substr(portable_rel_path, 1, length(?) + 1) = ? || '/')) \
                     OR \
                     ((portable_rel_path = ? OR substr(portable_rel_path, 1, length(?) + 1) = ? || '/') \
                      AND NOT (rel_path = ? OR substr(rel_path, 1, length(?) + 1) = ? || '/'))\
                 )\
             )",
        )
        .bind(params.knowledge_base_id.as_str())
        .bind(&old_rel_path)
        .bind(&old_rel_path)
        .bind(&old_rel_path)
        .bind(&old_portable_rel_path)
        .bind(&old_portable_rel_path)
        .bind(&old_portable_rel_path)
        .bind(&old_portable_rel_path)
        .bind(&old_portable_rel_path)
        .bind(&old_portable_rel_path)
        .bind(&old_rel_path)
        .bind(&old_rel_path)
        .bind(&old_rel_path)
        .fetch_one(&mut *tx)
        .await?;
        if inconsistent_subtree {
            return Err(DbError::Conflict(
                "knowledge entry projection paths are inconsistent; reconcile before relocating"
                    .into(),
            ));
        }

        let affected_ids = sqlx::query_scalar::<_, String>(
            "UPDATE knowledge_entries SET \
                parent_entry_id = CASE WHEN knowledge_entry_id = ? THEN ? ELSE parent_entry_id END, \
                name = CASE WHEN knowledge_entry_id = ? THEN ? ELSE name END, \
                rel_path = ? || substr(rel_path, length(?) + 1), \
                portable_rel_path = ? || substr(portable_rel_path, length(?) + 1), \
                revision = revision + 1, updated_at = MAX(updated_at, ?) \
             WHERE knowledge_base_id = ? AND deleted_at IS NULL \
               AND (rel_path = ? OR substr(rel_path, 1, length(?) + 1) = ? || '/') \
             RETURNING knowledge_entry_id",
        )
        .bind(params.knowledge_entry_id.as_str())
        .bind(
            params
                .destination_parent_entry_id
                .as_ref()
                .map(KnowledgeEntryId::as_str),
        )
        .bind(params.knowledge_entry_id.as_str())
        .bind(&params.new_name)
        .bind(&params.new_rel_path)
        .bind(&old_rel_path)
        .bind(&params.new_portable_rel_path)
        .bind(&old_portable_rel_path)
        .bind(params.updated_at)
        .bind(params.knowledge_base_id.as_str())
        .bind(&old_rel_path)
        .bind(&old_rel_path)
        .bind(&old_rel_path)
        .fetch_all(&mut *tx)
        .await
        .map_err(projection_error)?;
        if affected_ids.is_empty() {
            return Err(DbError::Conflict(
                "knowledge entry disappeared during projection relocation".into(),
            ));
        }
        let entry = fetch_entry(
            &mut tx,
            &params.knowledge_base_id,
            &params.knowledge_entry_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!(
                "knowledge entry '{}'",
                params.knowledge_entry_id.as_str()
            ))
        })?;
        let tree_revision = bump_tree_revision(&mut tx, &params.knowledge_base_id).await?;
        tx.commit().await?;
        Ok(KnowledgeEntryMutation {
            entry,
            affected_entries: affected_ids.len() as u64,
            tree_revision,
        })
    }

    async fn soft_delete_entry_subtree(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        knowledge_entry_id: &KnowledgeEntryId,
        expected_revision: i64,
        deleted_at: i64,
    ) -> Result<KnowledgeEntryMutation, DbError> {
        if expected_revision < 0 {
            return Err(DbError::Conflict(
                "expected knowledge entry revision must be non-negative".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        lock_base(&mut tx, knowledge_base_id).await?;
        let entry = fetch_entry(&mut tx, knowledge_base_id, knowledge_entry_id)
            .await?
            .filter(|entry| !entry.is_deleted())
            .ok_or_else(|| {
                DbError::NotFound(format!("live knowledge entry '{}'", knowledge_entry_id.as_str()))
            })?;
        if entry.revision != expected_revision {
            return Err(DbError::Conflict(format!(
                "knowledge entry '{}' revision conflict: expected {}, current {}",
                knowledge_entry_id.as_str(),
                expected_revision,
                entry.revision
            )));
        }
        if deleted_at < entry.created_at {
            return Err(DbError::Conflict(
                "knowledge entry deletion time precedes creation".into(),
            ));
        }
        let affected_ids = sqlx::query_scalar::<_, String>(
            "UPDATE knowledge_entries SET deleted_at = ?, revision = revision + 1, \
                updated_at = MAX(updated_at, ?) \
             WHERE knowledge_base_id = ? AND deleted_at IS NULL \
               AND (rel_path = ? OR substr(rel_path, 1, length(?) + 1) = ? || '/') \
             RETURNING knowledge_entry_id",
        )
        .bind(deleted_at)
        .bind(deleted_at)
        .bind(knowledge_base_id.as_str())
        .bind(&entry.rel_path)
        .bind(&entry.rel_path)
        .bind(&entry.rel_path)
        .fetch_all(&mut *tx)
        .await
        .map_err(projection_error)?;
        for affected_id in &affected_ids {
            sqlx::query(
                "UPDATE knowledge_sources SET \
                    default_parent_entry_id = NULL, revision = revision + 1, \
                    updated_at = MAX(updated_at, ?) \
                 WHERE knowledge_base_id = ? AND default_parent_entry_id = ?",
            )
            .bind(deleted_at)
            .bind(knowledge_base_id.as_str())
            .bind(affected_id)
            .execute(&mut *tx)
            .await?;
        }
        let entry = fetch_entry(&mut tx, knowledge_base_id, knowledge_entry_id)
            .await?
            .ok_or_else(|| {
                DbError::NotFound(format!("knowledge entry '{}'", knowledge_entry_id.as_str()))
            })?;
        let tree_revision = bump_tree_revision(&mut tx, knowledge_base_id).await?;
        tx.commit().await?;
        Ok(KnowledgeEntryMutation {
            entry,
            affected_entries: affected_ids.len() as u64,
            tree_revision,
        })
    }
}
