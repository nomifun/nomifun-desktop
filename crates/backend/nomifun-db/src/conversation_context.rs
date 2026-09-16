//! Conversation-owned model-history boundary. This hides no UI messages and
//! erases no effect/recovery evidence. Engines opt in through source policy.
use crate::{DbError, SqlitePool, TurnLifecycleTransition, sqlx};
use nomifun_common::TimestampMs;

pub const ENGINE_CONTEXT_AFTER_MESSAGE_ID: &str = "engine_context_after_message_id";

pub fn history_floor(extra: &serde_json::Value) -> Result<i64, DbError> {
    match extra.get(ENGINE_CONTEXT_AFTER_MESSAGE_ID) {
        None => Ok(0),
        Some(value) => value
            .as_i64()
            .filter(|id| *id >= 0)
            .ok_or_else(|| DbError::Conflict("Invalid persisted Engine context boundary".into())),
    }
}

/// Ordinary configuration writers cannot clear/advance this owner boundary,
/// including a stale whole-extra merge racing a context-clear transaction.
pub(crate) fn ensure_unchanged(current: &str, replacement: &str) -> Result<(), DbError> {
    let current: serde_json::Value =
        serde_json::from_str(current).map_err(|error| DbError::Conflict(error.to_string()))?;
    let replacement: serde_json::Value =
        serde_json::from_str(replacement).map_err(|error| DbError::Conflict(error.to_string()))?;
    history_floor(&current)?;
    history_floor(&replacement)?;
    if current.get(ENGINE_CONTEXT_AFTER_MESSAGE_ID)
        != replacement.get(ENGINE_CONTEXT_AFTER_MESSAGE_ID)
    {
        return Err(DbError::Conflict(
            "Engine context boundary changed; reload Session configuration".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn preserve_in_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    conversation: &str,
    replacement: &str,
) -> Result<(), DbError> {
    let current: String =
        sqlx::query_scalar("SELECT extra FROM conversations WHERE conversation_id = ?")
            .bind(conversation)
            .fetch_one(&mut **tx)
            .await?;
    ensure_unchanged(&current, replacement)
}

pub(crate) async fn clear(
    pool: &SqlitePool,
    user: &str,
    conversation: &str,
    expected_extra: &str,
    created_at: TimestampMs,
    updated_at: TimestampMs,
) -> Result<TurnLifecycleTransition, DbError> {
    let mut tx = pool.begin().await?;
    // Lock and compare the exact Session configuration that the service used
    // to select its compiled Engine policy, after runtime teardown.
    let locked = sqlx::query(
        "UPDATE conversations SET updated_at = updated_at WHERE conversation_id = ? AND user_id = ? \
         AND created_at = ? AND extra = ? AND status IN ('pending', 'finished') \
         AND active_turn_operation_id IS NULL AND admission_epoch < 9223372036854775807")
        .bind(conversation).bind(user).bind(created_at).bind(expected_extra)
        .execute(&mut *tx).await?;
    if locked.rows_affected() != 1 {
        tx.rollback().await?;
        return Ok(TurnLifecycleTransition::Stale);
    }
    let retained: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id = ? AND relation = 'attempt') \
         OR EXISTS(SELECT 1 FROM conversation_delivery_receipts WHERE conversation_id = ? AND status = 'accepted')")
        .bind(conversation).bind(conversation).fetch_one(&mut *tx).await?;
    if retained {
        return Err(DbError::Conflict(
            "Cannot clear retained or unsettled Engine context".into(),
        ));
    }
    let mut extra: serde_json::Value = serde_json::from_str(expected_extra)
        .map_err(|error| DbError::Conflict(error.to_string()))?;
    let previous = history_floor(&extra)?;
    let latest: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM messages WHERE conversation_id = ?")
            .bind(conversation)
            .fetch_one(&mut *tx)
            .await?;
    extra
        .as_object_mut()
        .ok_or_else(|| DbError::Conflict("Session extra must be an object".into()))?
        .insert(
            ENGINE_CONTEXT_AFTER_MESSAGE_ID.into(),
            serde_json::json!(previous.max(latest)),
        );
    let extra =
        serde_json::to_string(&extra).map_err(|error| DbError::Conflict(error.to_string()))?;
    sqlx::query(
        "UPDATE conversations SET extra = ?, admission_epoch = admission_epoch + 1, \
        updated_at = MAX(updated_at, ?) WHERE conversation_id = ? AND user_id = ?",
    )
    .bind(extra)
    .bind(updated_at)
    .bind(conversation)
    .bind(user)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(TurnLifecycleTransition::Committed)
}
