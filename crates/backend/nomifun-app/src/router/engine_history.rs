//! Raw, bounded history from the Conversation owner. No engine codec, replay
//! algorithm or automatic context-selection strategy lives here. The owner's
//! explicit clear-context floor bounds all model-history reads. A receipt's status is
//! not a process-exit proof, and event JSON is untrusted historical data.
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};

use super::engine_session_host::EngineTurnReceipt;

pub struct EngineHistoryRecord {
    pub sequence: i64,
    pub event_json: String,
    pub model_operation_id: Option<String>,
    pub model_claimed: bool,
}

pub struct EngineHistoryTurn {
    pub operation_id: String,
    pub root_message_id: String,
    pub receipt_status: String,
    pub root_content_json: String,
    pub request_payload_json: String,
    pub records: Vec<EngineHistoryRecord>,
    pub serialized_bytes: usize,
}

pub struct EngineHistoryWindow {
    /// Newest first; engines choose their own chronological/context projection.
    pub turns: Vec<EngineHistoryTurn>,
    /// Older eligible turns were omitted by limits, not by a user context clear.
    pub has_older: bool,
}

/// Data-only compatibility projection for Sessions without engine events
/// (including imported/forked history). Engines still own role interpretation.
pub struct EngineHistoryMessage {
    pub kind: String,
    pub position: Option<String>,
    pub content_json: String,
}
pub struct EngineMessageHistoryWindow {
    /// Newest first. No current root, hidden message or foreign Session data.
    pub messages: Vec<EngineHistoryMessage>,
    pub has_older: bool,
}

pub(super) async fn load_messages(
    pool: &SqlitePool,
    receipt: &EngineTurnReceipt,
    limit: usize,
    byte_limit: usize,
) -> Result<EngineMessageHistoryWindow, AppError> {
    load_messages_before(pool, receipt, limit, byte_limit, None).await
}

pub(super) async fn load_messages_before(
    pool: &SqlitePool,
    receipt: &EngineTurnReceipt,
    limit: usize,
    byte_limit: usize,
    before_operation: Option<&str>,
) -> Result<EngineMessageHistoryWindow, AppError> {
    if !(1..=4096).contains(&limit) || !(1..=8 * 1024 * 1024).contains(&byte_limit) {
        return Err(failure("invalid compatibility history budget"));
    }
    let session = &receipt.session().session().conversation_id;
    let mut tx = pool.begin().await.map_err(failure)?;
    let floor = context_floor(&mut tx, receipt).await?;
    let before_id = message_bound(&mut tx, receipt, floor, before_operation).await?;
    // Query sizes first, including role/type strings. No large content is
    // loaded just to discover that the window cannot afford it.
    let heads: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT m.id, length(CAST(m.content AS BLOB)) + length(CAST(m.type AS BLOB)) + COALESCE(length(CAST(m.position AS BLOB)), 0) \
         FROM messages m JOIN conversations c ON c.conversation_id = m.conversation_id \
         WHERE c.conversation_id = ? AND c.user_id = ? AND m.hidden = 0 AND m.type IN ('text', 'tool_call') \
         AND m.id > ? AND m.id < ? ORDER BY m.id DESC LIMIT ?")
        .bind(session).bind(&receipt.session().principal().principal_id).bind(floor)
        .bind(before_id).bind((limit + 1) as i64)
        .fetch_all(&mut *tx).await.map_err(failure)?;
    let mut window = EngineMessageHistoryWindow {
        messages: Vec::new(),
        has_older: heads.len() > limit,
    };
    let mut bytes = 0usize;
    for (id, size) in heads.into_iter().take(limit) {
        let next = bytes.saturating_add(usize::try_from(size).map_err(failure)?);
        if next > byte_limit {
            // A requested prefix is older optional context. Its first row may
            // exceed the budget left after native history; report truncation
            // instead of poisoning otherwise usable newer turns.
            if window.messages.is_empty() && before_operation.is_none() {
                return Err(failure("latest compatibility message exceeds budget"));
            }
            window.has_older = true;
            break;
        }
        let (kind, position, content_json): (String, Option<String>, String) = sqlx::query_as(
            "SELECT type, position, content FROM messages WHERE id = ? AND conversation_id = ?",
        )
        .bind(id)
        .bind(session)
        .fetch_one(&mut *tx)
        .await
        .map_err(failure)?;
        window.messages.push(EngineHistoryMessage {
            kind,
            position,
            content_json,
        });
        bytes = next;
    }
    tx.commit().await.map_err(failure)?;
    Ok(window)
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine history: {message}"))
}

async fn message_bound(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    receipt: &EngineTurnReceipt,
    floor: i64,
    before_operation: Option<&str>,
) -> Result<i64, AppError> {
    if before_operation
        .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
    {
        return Err(failure("invalid historical turn cursor"));
    }
    let session = &receipt.session().session().conversation_id;
    let user = &receipt.session().principal().principal_id;
    let root: i64 = sqlx::query_scalar(
        "SELECT m.id FROM messages m JOIN conversations c ON c.conversation_id = m.conversation_id \
         WHERE c.conversation_id = ? AND c.user_id = ? AND m.message_id = ?",
    )
    .bind(session)
    .bind(user)
    .bind(receipt.root_message_id())
    .fetch_one(&mut **tx)
    .await
    .map_err(failure)?;
    if root <= floor {
        return Err(failure(
            "current root precedes the cleared context boundary",
        ));
    }
    let Some(operation) = before_operation else {
        return Ok(root);
    };
    sqlx::query_scalar::<_, i64>(
        "SELECT m.id FROM conversation_delivery_receipts r JOIN messages m \
         ON m.message_id = r.message_id AND m.conversation_id = r.conversation_id \
         WHERE r.conversation_id = ? AND r.user_id = ? AND r.kind = 'turn' \
         AND r.operation_id = ? AND m.id < ? AND m.id > ?",
    )
    .bind(session)
    .bind(user)
    .bind(operation)
    .bind(root)
    .bind(floor)
    .fetch_optional(&mut **tx)
    .await
    .map_err(failure)?
    .ok_or_else(|| failure("historical cursor is outside the current Session context"))
}

async fn context_floor(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    receipt: &EngineTurnReceipt,
) -> Result<i64, AppError> {
    // Read the current durable boundary in the same snapshot as the page,
    // never the runtime constructor's potentially older Session projection.
    let extra: String = sqlx::query_scalar(
        "SELECT extra FROM conversations WHERE conversation_id = ? AND user_id = ? \
         AND status = 'running' AND admission_epoch = ? AND active_turn_operation_id = ?",
    )
    .bind(&receipt.session().session().conversation_id)
    .bind(&receipt.session().principal().principal_id)
    .bind(receipt.admission_epoch())
    .bind(receipt.operation_id())
    .fetch_one(&mut **tx)
    .await
    .map_err(failure)?;
    let extra = serde_json::from_str(&extra).map_err(failure)?;
    nomifun_db::conversation_context::history_floor(&extra).map_err(failure)
}

pub(super) async fn load(
    pool: &SqlitePool,
    receipt: &EngineTurnReceipt,
    limit: usize,
) -> Result<EngineHistoryWindow, AppError> {
    load_before(pool, receipt, limit, None).await
}

pub(super) async fn load_before(
    pool: &SqlitePool,
    receipt: &EngineTurnReceipt,
    limit: usize,
    before_operation: Option<&str>,
) -> Result<EngineHistoryWindow, AppError> {
    if !(1..=32).contains(&limit) {
        return Err(failure("turn limit must be 1..32"));
    }
    let session = &receipt.session().session().conversation_id;
    let user = &receipt.session().principal().principal_id;
    // One read snapshot: an interrupted turn cannot grow between the budget
    // query and fetching its records. No oversized blob is loaded speculatively.
    let mut tx = pool.begin().await.map_err(failure)?;
    let floor = context_floor(&mut tx, receipt).await?;
    let before_id = message_bound(&mut tx, receipt, floor, before_operation).await?;
    let heads: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT r.operation_id, r.message_id, r.status, length(CAST(m.content AS BLOB)) + length(CAST(r.request_payload AS BLOB)) \
         + length(CAST(r.operation_id AS BLOB)) + length(CAST(r.message_id AS BLOB)) + length(CAST(r.status AS BLOB)) \
         FROM conversation_delivery_receipts r JOIN messages m ON m.message_id = r.message_id AND m.conversation_id = r.conversation_id \
         JOIN conversations c ON c.conversation_id = r.conversation_id AND c.user_id = r.user_id \
         WHERE r.conversation_id = ? AND r.user_id = ? AND r.kind = 'turn' \
         AND m.id < ? AND m.id > ? \
         ORDER BY m.id DESC LIMIT ?")
        .bind(session).bind(user).bind(before_id).bind(floor).bind((limit + 1) as i64)
        .fetch_all(&mut *tx).await.map_err(failure)?;
    let mut window = EngineHistoryWindow {
        has_older: heads.len() > limit,
        turns: Vec::new(),
    };
    let mut total = 0usize;
    for (operation_id, root_message_id, receipt_status, root_bytes) in heads.into_iter().take(limit)
    {
        let (count, event_bytes): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(length(CAST(event_json AS BLOB)) + COALESCE(length(CAST(model_operation_id AS BLOB)), 0)), 0) FROM conversation_runtime_events WHERE conversation_id = ? AND turn_operation_id = ?")
            .bind(session).bind(&operation_id).fetch_one(&mut *tx).await.map_err(failure)?;
        let bytes = usize::try_from(root_bytes)
            .map_err(failure)?
            .saturating_add(usize::try_from(event_bytes).map_err(failure)?);
        if count > 4096
            || event_bytes > 8 * 1024 * 1024
            || total.saturating_add(bytes) > 16 * 1024 * 1024
        {
            if window.turns.is_empty() {
                return Err(failure("latest turn exceeds the history budget"));
            }
            window.has_older = true;
            break;
        }
        let (root_content_json, request_payload_json): (String, String) = sqlx::query_as(
            "SELECT m.content, r.request_payload FROM conversation_delivery_receipts r JOIN messages m \
             ON m.message_id = r.message_id AND m.conversation_id = r.conversation_id \
             WHERE r.operation_id = ? AND r.conversation_id = ? AND r.user_id = ?")
            .bind(&operation_id).bind(session).bind(user).fetch_one(&mut *tx).await.map_err(failure)?;
        let raw: Vec<(i64, String, Option<String>, bool)> = sqlx::query_as(
            "SELECT sequence, event_json, model_operation_id, model_claimed FROM conversation_runtime_events \
             WHERE conversation_id = ? AND turn_operation_id = ? ORDER BY sequence LIMIT 4096")
            .bind(session).bind(&operation_id).fetch_all(&mut *tx).await.map_err(failure)?;
        let mut records = Vec::with_capacity(raw.len());
        for (index, (sequence, event_json, model_operation_id, model_claimed)) in
            raw.into_iter().enumerate()
        {
            if sequence != index as i64 + 1 {
                return Err(failure("journal sequence is incomplete"));
            }
            records.push(EngineHistoryRecord {
                sequence,
                event_json,
                model_operation_id,
                model_claimed,
            });
        }
        total = total.saturating_add(bytes);
        window.turns.push(EngineHistoryTurn {
            operation_id,
            root_message_id,
            receipt_status,
            root_content_json,
            request_payload_json,
            records,
            serialized_bytes: bytes,
        });
    }
    tx.commit().await.map_err(failure)?;
    Ok(window)
}
