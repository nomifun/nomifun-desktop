//! Atomic publication of learned memories against the snapshot used by the LLM.

use std::collections::BTreeSet;

use nomifun_common::{AppError, CompanionEventId, CompanionId, CompanionMemoryId, now_ms};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection};

use crate::store::{
    CompanionMemoryMessageSource, CompanionStore, MEMORY_KINDS, fts_index_insert,
    memory_contents_similar, memory_message_retraction_key, validate_memory_message_sources,
};

#[derive(Debug, Clone)]
pub struct LearnedMemoryCandidate {
    pub kind: String,
    pub content: String,
    pub tags: Vec<String>,
    pub importance: f64,
    pub source_event_ids: Vec<String>,
    pub source_message_refs: Vec<CompanionMemoryMessageSource>,
}

/// Host-resolved confirmation of an existing memory. The revision is from the
/// snapshot shown to the model; canonical message references come from the
/// trusted collected events selected by the model, never from model output.
#[derive(Debug, Clone)]
pub struct LearnedMemoryReinforcement {
    pub memory_id: String,
    pub expected_revision: i64,
    pub source_event_ids: Vec<String>,
    pub source_message_refs: Vec<CompanionMemoryMessageSource>,
}

fn database_error(error: sqlx::Error) -> AppError {
    AppError::Internal(format!("companion learning commit database: {error}"))
}

fn validate_owner(owner: &str) -> Result<(), AppError> {
    CompanionId::try_from(owner)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid learning companion id: {error}")))
}

fn retracted_source_key(event_id: &str) -> Result<String, AppError> {
    CompanionEventId::try_from(event_id)
        .map_err(|error| AppError::BadRequest(format!("invalid memory source event id: {error}")))?;
    Ok(format!("memory_retracted_source:{}", hex::encode(Sha256::digest(event_id.as_bytes()))))
}

/// Both queries run on the caller's transaction, so the fingerprint cannot mix
/// a memory snapshot from before a correction with markers from after it.
async fn generation_on(connection: &mut SqliteConnection, owner: &str) -> Result<String, AppError> {
    type MemoryVersion = (String, i64, String, Option<String>, Option<String>, String, String);
    let memories: Vec<MemoryVersion> = sqlx::query_as(
        "SELECT memory_id, revision, status, source_session_id, source_operation_id, source_event_ids, source_message_refs \
         FROM companion_memories WHERE companion_id = ? ORDER BY memory_id",
    )
    .bind(owner)
    .fetch_all(&mut *connection)
    .await
    .map_err(database_error)?;
    let retractions: Vec<(String, String)> = sqlx::query_as(
        "SELECT state_key, value FROM companion_runtime_state \
         WHERE companion_id = ? AND (state_key GLOB 'memory_retracted_source:*' OR state_key = 'memory_owner_epoch') ORDER BY state_key",
    )
    .bind(owner)
    .fetch_all(&mut *connection)
    .await
    .map_err(database_error)?;
    let encoded = serde_json::to_vec(&("companion-learning-generation-v1", owner, memories, retractions))
        .map_err(|error| AppError::Internal(format!("serialize memory generation: {error}")))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

async fn source_is_retracted_on(
    connection: &mut SqliteConnection,
    owner: &str,
    event_id: &str,
) -> Result<bool, AppError> {
    let key = retracted_source_key(event_id)?;
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM companion_runtime_state WHERE companion_id = ? AND state_key = ?)",
    )
    .bind(owner)
    .bind(key)
    .fetch_one(connection)
    .await
    .map_err(database_error)
}

async fn message_is_retracted_on(
    connection: &mut SqliteConnection,
    owner: &str,
    session_id: &str,
    message_id: &str,
) -> Result<bool, AppError> {
    let key = memory_message_retraction_key(session_id, message_id)?;
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM companion_runtime_state WHERE companion_id = ? AND state_key = ?)",
    )
    .bind(owner)
    .bind(key)
    .fetch_one(connection)
    .await
    .map_err(database_error)
}

fn checked_candidate(candidate: &LearnedMemoryCandidate) -> Result<LearnedMemoryCandidate, AppError> {
    if !MEMORY_KINDS.contains(&candidate.kind.as_str()) {
        return Err(AppError::BadRequest(format!("invalid learned memory kind '{}'", candidate.kind)));
    }
    let content = candidate.content.trim();
    if content.is_empty() {
        return Err(AppError::BadRequest("learned memory content is empty".into()));
    }
    if !candidate.importance.is_finite() || !(0.0..=1.0).contains(&candidate.importance) {
        return Err(AppError::BadRequest("learned memory importance must be between zero and one".into()));
    }
    let mut tags = BTreeSet::new();
    for tag in &candidate.tags {
        if tag.trim().is_empty() || tag.trim() != tag || tag.chars().any(char::is_control) {
            return Err(AppError::BadRequest("invalid learned memory tag".into()));
        }
        tags.insert(tag.clone());
    }
    let mut source_event_ids = BTreeSet::new();
    if candidate.source_event_ids.is_empty() {
        return Err(AppError::BadRequest("learned memory requires a source event".into()));
    }
    for event_id in &candidate.source_event_ids {
        CompanionEventId::try_from(event_id.as_str())
            .map_err(|error| AppError::BadRequest(format!("invalid memory source event id: {error}")))?;
        source_event_ids.insert(event_id.clone());
    }
    validate_memory_message_sources(&candidate.source_message_refs)?;
    Ok(LearnedMemoryCandidate {
        kind: candidate.kind.clone(),
        content: nomi_redact::redact_secrets(content).into_owned(),
        tags: tags.into_iter().collect(),
        importance: candidate.importance,
        source_event_ids: source_event_ids.into_iter().collect(),
        source_message_refs: candidate.source_message_refs.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect(),
    })
}

fn checked_reinforcement(reinforcement: &LearnedMemoryReinforcement) -> Result<LearnedMemoryReinforcement, AppError> {
    CompanionMemoryId::try_from(reinforcement.memory_id.as_str())
        .map_err(|error| AppError::BadRequest(format!("invalid learned memory id: {error}")))?;
    if reinforcement.expected_revision < 1 {
        return Err(AppError::BadRequest("expected learned memory revision must be positive".into()));
    }
    if reinforcement.source_event_ids.is_empty() || reinforcement.source_event_ids.len() > 64 {
        return Err(AppError::BadRequest("learned reinforcement requires one to 64 source events".into()));
    }
    let mut source_event_ids = BTreeSet::new();
    for event_id in &reinforcement.source_event_ids {
        CompanionEventId::try_from(event_id.as_str())
            .map_err(|error| AppError::BadRequest(format!("invalid reinforcement source event id: {error}")))?;
        source_event_ids.insert(event_id.clone());
    }
    validate_memory_message_sources(&reinforcement.source_message_refs)?;
    Ok(LearnedMemoryReinforcement {
        memory_id: reinforcement.memory_id.clone(),
        expected_revision: reinforcement.expected_revision,
        source_event_ids: source_event_ids.into_iter().collect(),
        source_message_refs: reinforcement.source_message_refs.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect(),
    })
}

fn checked_stored_sources(encoded: &str) -> Result<BTreeSet<String>, AppError> {
    let sources: Vec<String> = serde_json::from_str(encoded)
        .map_err(|error| AppError::Internal(format!("invalid stored memory source event ids: {error}")))?;
    let mut checked = BTreeSet::new();
    for source in sources {
        CompanionEventId::try_from(source.as_str())
            .map_err(|error| AppError::Internal(format!("invalid stored memory source event id: {error}")))?;
        checked.insert(source);
    }
    Ok(checked)
}

fn checked_stored_message_sources(encoded: &str) -> Result<BTreeSet<CompanionMemoryMessageSource>, AppError> {
    let sources: Vec<CompanionMemoryMessageSource> = serde_json::from_str(encoded)
        .map_err(|error| AppError::Internal(format!("invalid stored memory message sources: {error}")))?;
    validate_memory_message_sources(&sources)
        .map_err(|error| AppError::Internal(format!("invalid stored memory message source: {error}")))?;
    Ok(sources.into_iter().collect())
}

impl CompanionStore {
    pub async fn memory_generation(&self, owner: &str) -> Result<String, AppError> {
        validate_owner(owner)?;
        let mut tx = self.pool().begin().await.map_err(database_error)?;
        let generation = generation_on(&mut tx, owner).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(generation)
    }

    pub async fn memory_source_is_retracted(&self, owner: &str, event_id: &str) -> Result<bool, AppError> {
        validate_owner(owner)?;
        let mut connection = self.pool().acquire().await.map_err(database_error)?;
        source_is_retracted_on(&mut connection, owner, event_id).await
    }

    /// Host canonical readers use the exact Session/message pair; an opaque
    /// source operation is never interpreted as a canonical message ID.
    pub async fn memory_message_is_retracted(&self, owner: &str, session_id: &str, message_id: &str) -> Result<bool, AppError> {
        validate_owner(owner)?;
        let mut connection = self.pool().acquire().await.map_err(database_error)?;
        message_is_retracted_on(&mut connection, owner, session_id, message_id).await
    }

    /// The returned count includes newly inserted rows. Reinforcement, source
    /// unions and supersession change existing rows without inflating it.
    pub async fn apply_learning_batch(
        &self,
        owner: &str,
        expected_generation: &str,
        reinforce: &[LearnedMemoryReinforcement],
        supersede: &[(String, i64)],
        new_memories: &[LearnedMemoryCandidate],
    ) -> Result<usize, AppError> {
        validate_owner(owner)?;
        for (memory_id, revision) in supersede {
            CompanionMemoryId::try_from(memory_id.as_str())
                .map_err(|error| AppError::BadRequest(format!("invalid learned memory id: {error}")))?;
            if *revision < 1 {
                return Err(AppError::BadRequest("expected learned memory revision must be positive".into()));
            }
        }
        let reinforcements = reinforce.iter().map(checked_reinforcement).collect::<Result<Vec<_>, _>>()?;
        let candidates = new_memories.iter().map(checked_candidate).collect::<Result<Vec<_>, _>>()?;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await.map_err(database_error)?;
        if generation_on(&mut tx, owner).await? != expected_generation {
            return Err(AppError::Conflict("companion memories changed while learning; retry from the current snapshot".into()));
        }
        let now = now_ms();
        for reinforcement in reinforcements {
            let existing = sqlx::query(
                "SELECT source_event_ids, source_message_refs FROM companion_memories \
                 WHERE memory_id = ? AND companion_id = ? AND status = 'active' AND revision = ?",
            )
            .bind(&reinforcement.memory_id).bind(owner).bind(reinforcement.expected_revision)
            .fetch_optional(&mut *tx).await.map_err(database_error)?
            .ok_or_else(|| AppError::Conflict("a learned reinforcement no longer addresses an active owned memory revision".into()))?;
            for source in &reinforcement.source_event_ids {
                if source_is_retracted_on(&mut tx, owner, source).await? {
                    return Err(AppError::Conflict("a learned reinforcement source was retracted".into()));
                }
            }
            for source in &reinforcement.source_message_refs {
                if message_is_retracted_on(&mut tx, owner, &source.session_id, &source.message_id).await? {
                    return Err(AppError::Conflict("a learned reinforcement canonical source was retracted".into()));
                }
            }
            let mut sources = checked_stored_sources(existing.get("source_event_ids"))?;
            sources.extend(reinforcement.source_event_ids);
            let mut message_sources = checked_stored_message_sources(existing.get("source_message_refs"))?;
            message_sources.extend(reinforcement.source_message_refs);
            let sources = serde_json::to_string(&sources)
                .map_err(|error| AppError::Internal(format!("serialize reinforced memory sources: {error}")))?;
            let message_sources = serde_json::to_string(&message_sources)
                .map_err(|error| AppError::Internal(format!("serialize reinforced memory message sources: {error}")))?;
            let result = sqlx::query(
                "UPDATE companion_memories SET strength = MIN(1.0, strength + 0.2), \
                 source_event_ids = ?, source_message_refs = ?, last_reinforced_at = ?, updated_at = ?, revision = revision + 1 \
                 WHERE memory_id = ? AND companion_id = ? AND status = 'active' AND revision = ?",
            )
            .bind(sources).bind(message_sources).bind(now).bind(now)
            .bind(&reinforcement.memory_id).bind(owner).bind(reinforcement.expected_revision)
            .execute(&mut *tx).await.map_err(database_error)?;
            if result.rows_affected() != 1 {
                return Err(AppError::Conflict("a learned reinforcement no longer addresses an active owned memory revision".into()));
            }
        }
        for (memory_id, revision) in supersede {
            let result = sqlx::query(
                "UPDATE companion_memories SET status = 'archived', updated_at = ?, revision = revision + 1 \
                 WHERE memory_id = ? AND companion_id = ? AND status = 'active' AND revision = ?",
            )
            .bind(now).bind(memory_id).bind(owner).bind(revision)
            .execute(&mut *tx).await.map_err(database_error)?;
            if result.rows_affected() != 1 {
                return Err(AppError::Conflict("a learned supersession no longer addresses an active owned memory revision".into()));
            }
        }

        let mut inserted = 0;
        for candidate in candidates {
            let mut retracted = false;
            for source in &candidate.source_event_ids {
                if source_is_retracted_on(&mut tx, owner, source).await? {
                    retracted = true;
                    break;
                }
            }
            for source in &candidate.source_message_refs {
                if message_is_retracted_on(&mut tx, owner, &source.session_id, &source.message_id).await? {
                    retracted = true;
                    break;
                }
            }
            if retracted { continue; }
            let existing = sqlx::query(
                "SELECT memory_id, content, revision, source_event_ids, source_message_refs FROM companion_memories \
                 WHERE companion_id = ? AND kind = ? AND status = 'active' ORDER BY memory_id",
            )
            .bind(owner).bind(&candidate.kind)
            .fetch_all(&mut *tx).await.map_err(database_error)?;
            if let Some(existing) = existing.iter().find(|row| {
                let content: &str = row.get("content");
                memory_contents_similar(&candidate.content, content)
            }) {
                let encoded: &str = existing.get("source_event_ids");
                let mut sources = checked_stored_sources(encoded)?;
                let previous_len = sources.len();
                sources.extend(candidate.source_event_ids);
                let mut message_sources = checked_stored_message_sources(existing.get("source_message_refs"))?;
                let previous_message_len = message_sources.len();
                message_sources.extend(candidate.source_message_refs);
                if sources.len() != previous_len || message_sources.len() != previous_message_len {
                    let sources = serde_json::to_string(&sources)
                        .map_err(|error| AppError::Internal(format!("serialize learned memory sources: {error}")))?;
                    let message_sources = serde_json::to_string(&message_sources)
                        .map_err(|error| AppError::Internal(format!("serialize learned memory message sources: {error}")))?;
                    let memory_id: &str = existing.get("memory_id");
                    let revision: i64 = existing.get("revision");
                    let result = sqlx::query(
                        "UPDATE companion_memories SET source_event_ids = ?, source_message_refs = ?, updated_at = ?, revision = revision + 1 \
                         WHERE memory_id = ? AND companion_id = ? AND status = 'active' AND revision = ?",
                    )
                    .bind(sources).bind(message_sources).bind(now).bind(memory_id).bind(owner).bind(revision)
                    .execute(&mut *tx).await.map_err(database_error)?;
                    if result.rows_affected() != 1 {
                        return Err(AppError::Conflict("learned memory source union lost its active revision".into()));
                    }
                }
                continue;
            }
            let memory_id = CompanionMemoryId::new().into_string();
            let tags = serde_json::to_string(&candidate.tags)
                .map_err(|error| AppError::Internal(format!("serialize learned memory tags: {error}")))?;
            let sources = serde_json::to_string(&candidate.source_event_ids)
                .map_err(|error| AppError::Internal(format!("serialize learned memory sources: {error}")))?;
            let message_sources = serde_json::to_string(&candidate.source_message_refs)
                .map_err(|error| AppError::Internal(format!("serialize learned memory message sources: {error}")))?;
            let rowid: i64 = sqlx::query_scalar(
                "INSERT INTO companion_memories(memory_id, kind, content, tags, importance, strength, pinned, source, \
                 status, created_at, updated_at, last_reinforced_at, companion_id, revision, source_session_id, \
                 source_operation_id, source_event_ids, source_message_refs) \
                 VALUES(?,?,?,?,?,?,0,'learn','active',?,?,?,?,1,NULL,NULL,?,?) RETURNING id",
            )
            .bind(memory_id).bind(&candidate.kind).bind(&candidate.content).bind(tags)
            .bind(candidate.importance).bind(candidate.importance)
            .bind(now).bind(now).bind(now).bind(owner).bind(sources).bind(message_sources)
            .fetch_one(&mut *tx).await.map_err(database_error)?;
            fts_index_insert(&mut *tx, rowid, &candidate.content).await?;
            inserted += 1;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(inserted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryActor;

    fn candidate(event_id: &str) -> LearnedMemoryCandidate {
        LearnedMemoryCandidate {
            kind: "preference".into(), content: "主人喜欢清晨喝深烘焙咖啡".into(),
            tags: vec!["咖啡".into()], importance: 0.8, source_event_ids: vec![event_id.into()],
            source_message_refs: vec![],
        }
    }

    fn reinforcement(memory_id: &str, expected_revision: i64, event_id: &str) -> LearnedMemoryReinforcement {
        LearnedMemoryReinforcement {
            memory_id: memory_id.to_owned(), expected_revision,
            source_event_ids: vec![event_id.to_owned()], source_message_refs: vec![],
        }
    }

    #[tokio::test]
    async fn correction_during_learning_rejects_the_entire_batch() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = CompanionId::new().into_string();
        let memory = store.insert_memory_scoped("preference", "原来的咖啡偏好", &[], 0.7, "manual", Some(&owner)).await.unwrap();
        let generation = store.memory_generation(&owner).await.unwrap();
        store.update_memory_cas(&memory.memory_id, 1, Some("当前确认喝茶"), None, None, &MemoryActor::Companion(owner.clone())).await.unwrap();
        let event = CompanionEventId::new().into_string();
        assert!(matches!(store.apply_learning_batch(&owner, &generation, &[reinforcement(&memory.memory_id, 1, &event)], &[], &[candidate(&event)]).await, Err(AppError::Conflict(_))));
        assert_eq!(store.get_memory(&memory.memory_id).await.unwrap().unwrap().content, "当前确认喝茶");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companion_memories").fetch_one(store.pool()).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn source_tombstones_change_generation_and_prevent_reextraction() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = CompanionId::new().into_string();
        let event = CompanionEventId::new().into_string();
        let generation = store.memory_generation(&owner).await.unwrap();
        assert!(!store.memory_source_is_retracted(&owner, &event).await.unwrap());
        store.set_companion_state(&owner, &retracted_source_key(&event).unwrap(), "1").await.unwrap();
        assert!(store.memory_source_is_retracted(&owner, &event).await.unwrap());
        assert!(matches!(store.apply_learning_batch(&owner, &generation, &[], &[], &[candidate(&event)]).await, Err(AppError::Conflict(_))));
        let current = store.memory_generation(&owner).await.unwrap();
        assert_ne!(generation, current);
        assert_eq!(store.apply_learning_batch(&owner, &current, &[], &[], &[candidate(&event)]).await.unwrap(), 0);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companion_memories").fetch_one(store.pool()).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn foreign_and_archived_memories_cannot_be_reinforced() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = CompanionId::new().into_string();
        let other_owner = CompanionId::new().into_string();
        let event = CompanionEventId::new().into_string();
        let memory = store.insert_memory_scoped("knowledge", "属于另一个伙伴的事实", &[], 0.6, "manual", Some(&other_owner)).await.unwrap();
        let generation = store.memory_generation(&owner).await.unwrap();
        assert!(matches!(store.apply_learning_batch(&owner, &generation, &[reinforcement(&memory.memory_id, 1, &event)], &[], &[]).await, Err(AppError::Conflict(_))));
        let archived = store.update_memory_cas(&memory.memory_id, 1, None, None, Some("archived"), &MemoryActor::Companion(other_owner.clone())).await.unwrap();
        let generation = store.memory_generation(&other_owner).await.unwrap();
        assert!(matches!(store.apply_learning_batch(&other_owner, &generation, &[reinforcement(&memory.memory_id, archived.revision, &event)], &[], &[]).await, Err(AppError::Conflict(_))));
        assert_eq!(store.get_memory(&memory.memory_id).await.unwrap().unwrap(), archived);
    }

    #[tokio::test]
    async fn stale_generation_does_not_union_confirmation_sources() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = CompanionId::new().into_string();
        let first_event = CompanionEventId::new().into_string();
        let first_source = CompanionMemoryMessageSource { session_id: nomifun_common::generate_id(), message_id: nomifun_common::generate_id() };
        let mut first = candidate(&first_event);
        first.source_message_refs = vec![first_source.clone()];
        let generation = store.memory_generation(&owner).await.unwrap();
        store.apply_learning_batch(&owner, &generation, &[], &[], &[first]).await.unwrap();
        let before = store.dump_memories_all().await.unwrap().remove(0);
        let stale_generation = store.memory_generation(&owner).await.unwrap();
        let after = store.update_memory_cas(&before.memory_id, before.revision, None, Some(true), None,
            &MemoryActor::Companion(owner.clone())).await.unwrap();
        let next_event = CompanionEventId::new().into_string();
        let next_source = CompanionMemoryMessageSource { session_id: nomifun_common::generate_id(), message_id: nomifun_common::generate_id() };
        let mut confirmation = reinforcement(&before.memory_id, before.revision, &next_event);
        confirmation.source_message_refs = vec![next_source];
        assert!(matches!(store.apply_learning_batch(&owner, &stale_generation, &[confirmation], &[], &[]).await,
            Err(AppError::Conflict(_))));
        let current = store.get_memory(&before.memory_id).await.unwrap().unwrap();
        assert_eq!(current, after);
        assert_eq!(current.source_event_ids, vec![first_event]);
        assert_eq!(current.source_message_refs, vec![first_source]);
    }

    #[tokio::test]
    async fn retracted_confirmation_sources_reject_the_entire_batch_without_partial_unions() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = CompanionId::new().into_string();
        let first = store.insert_memory_scoped("profile", "主人住北京", &[], 0.5, "manual", Some(&owner)).await.unwrap();
        let second = store.insert_memory_scoped("preference", "主人爱喝茶", &[], 0.5, "manual", Some(&owner)).await.unwrap();
        let blocked_event = CompanionEventId::new().into_string();
        let blocked_source = CompanionMemoryMessageSource { session_id: nomifun_common::generate_id(), message_id: nomifun_common::generate_id() };
        let cut_event_key = retracted_source_key(&blocked_event).unwrap();
        let cut_message_key = memory_message_retraction_key(&blocked_source.session_id, &blocked_source.message_id).unwrap();
        store.set_companion_state(&owner, &cut_event_key, "1").await.unwrap();
        store.set_companion_state(&owner, &cut_message_key, "1").await.unwrap();
        for canonical_cut in [false, true] {
            let clean = reinforcement(&first.memory_id, first.revision, &CompanionEventId::new().into_string());
            let source_event = if canonical_cut { CompanionEventId::new().into_string() } else { blocked_event.clone() };
            let mut blocked = reinforcement(&second.memory_id, second.revision, &source_event);
            if canonical_cut { blocked.source_message_refs = vec![blocked_source.clone()]; }
            let generation = store.memory_generation(&owner).await.unwrap();
            assert!(matches!(store.apply_learning_batch(&owner, &generation, &[clean, blocked], &[], &[]).await,
                Err(AppError::Conflict(_))));
            assert_eq!(store.memory_generation(&owner).await.unwrap(), generation);
            assert_eq!(store.get_memory(&first.memory_id).await.unwrap().unwrap(), first);
            assert_eq!(store.get_memory(&second.memory_id).await.unwrap().unwrap(), second);
        }
    }

    #[tokio::test]
    async fn duplicate_learning_preserves_all_sources_without_duplicating_memory() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = CompanionId::new().into_string();
        let event = CompanionEventId::new().into_string();
        let second_event = CompanionEventId::new().into_string();
        let generation = store.memory_generation(&owner).await.unwrap();
        assert_eq!(store.apply_learning_batch(&owner, &generation, &[], &[], &[candidate(&event)]).await.unwrap(), 1);
        let generation = store.memory_generation(&owner).await.unwrap();
        assert_eq!(store.apply_learning_batch(&owner, &generation, &[], &[], &[candidate(&second_event)]).await.unwrap(), 0);
        let rows = sqlx::query("SELECT memory_id, revision, source_event_ids, source_session_id, source_operation_id FROM companion_memories WHERE companion_id = ?")
            .bind(&owner).fetch_all(store.pool()).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<i64, _>("revision"), 2);
        let sources = checked_stored_sources(rows[0].get("source_event_ids")).unwrap();
        assert_eq!(sources, BTreeSet::from([event.clone(), second_event.clone()]));
        assert!(rows[0].get::<Option<String>, _>("source_session_id").is_none());
        assert!(rows[0].get::<Option<String>, _>("source_operation_id").is_none());
        let indexed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companion_memories_fts_docsize")
            .fetch_one(store.pool()).await.unwrap();
        assert_eq!(indexed, 1);
        let memory_id: String = rows[0].get("memory_id");
        store.delete_memory(&memory_id, &MemoryActor::Companion(owner.clone())).await.unwrap();
        assert!(store.memory_source_is_retracted(&owner, &event).await.unwrap());
        assert!(store.memory_source_is_retracted(&owner, &second_event).await.unwrap());
        let generation = store.memory_generation(&owner).await.unwrap();
        assert_eq!(store.apply_learning_batch(&owner, &generation, &[], &[], &[candidate(&event), candidate(&second_event)]).await.unwrap(), 0);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companion_memories")
            .fetch_one(store.pool()).await.unwrap();
        assert_eq!(count, 0, "neither duplicate source may recreate a deleted memory");
    }

    #[tokio::test]
    async fn canonical_message_sources_are_unioned_and_block_relearning_under_fresh_collector_events() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = CompanionId::new().into_string();
        let event = CompanionEventId::new().into_string();
        let source = CompanionMemoryMessageSource {
            session_id: nomifun_common::generate_id(), message_id: nomifun_common::generate_id(),
        };
        let second_source = CompanionMemoryMessageSource {
            session_id: nomifun_common::generate_id(), message_id: nomifun_common::generate_id(),
        };
        let mut first = candidate(&event);
        first.source_message_refs = vec![source.clone()];
        let generation = store.memory_generation(&owner).await.unwrap();
        assert_eq!(store.apply_learning_batch(&owner, &generation, &[], &[], &[first]).await.unwrap(), 1);
        let generation = store.memory_generation(&owner).await.unwrap();
        let mut second = candidate(&event);
        second.source_message_refs = vec![source.clone(), second_source.clone(), second_source.clone()];
        assert_eq!(store.apply_learning_batch(&owner, &generation, &[], &[], &[second]).await.unwrap(), 0);
        let rows = store.dump_memories_all().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].revision, 2, "new canonical provenance changes the row even if collector source is unchanged");
        assert_eq!(rows[0].source_message_refs.iter().cloned().collect::<BTreeSet<_>>(), BTreeSet::from([source.clone(), second_source.clone()]));
        let after_union = store.memory_generation(&owner).await.unwrap();
        assert_ne!(generation, after_union);
        // Fingerprint includes source metadata itself, independent of revision.
        sqlx::query("UPDATE companion_memories SET source_message_refs = '[]' WHERE memory_id = ?")
            .bind(&rows[0].memory_id).execute(store.pool()).await.unwrap();
        assert_ne!(after_union, store.memory_generation(&owner).await.unwrap());
        sqlx::query("UPDATE companion_memories SET source_message_refs = ? WHERE memory_id = ?")
            .bind(serde_json::to_string(&rows[0].source_message_refs).unwrap())
            .bind(&rows[0].memory_id).execute(store.pool()).await.unwrap();
        store.delete_memory(&rows[0].memory_id, &MemoryActor::Companion(owner.clone())).await.unwrap();
        for message in [&source, &second_source] {
            assert!(store.memory_message_is_retracted(&owner, &message.session_id, &message.message_id).await.unwrap());
        }
        let mut retried = candidate(&CompanionEventId::new().into_string());
        retried.source_message_refs = vec![source];
        let generation = store.memory_generation(&owner).await.unwrap();
        assert_eq!(store.apply_learning_batch(&owner, &generation, &[], &[], &[retried]).await.unwrap(), 0);
        assert!(store.dump_memories_all().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn opaque_source_operation_does_not_claim_a_canonical_message() {
        let store = CompanionStore::open_memory().await.unwrap();
        let owner = CompanionId::new().into_string();
        let session = nomifun_common::generate_id();
        let operation = nomifun_common::generate_id();
        let memory = store.insert_memory_scoped_with_provenance(
            "knowledge", "操作来源不等于消息引用", &[], 0.6, "agent", Some(&owner), Some(&session), Some(&operation),
        ).await.unwrap();
        store.delete_memory(&memory.memory_id, &MemoryActor::Companion(owner.clone())).await.unwrap();
        assert!(!store.memory_message_is_retracted(&owner, &session, &operation).await.unwrap());
    }
}
