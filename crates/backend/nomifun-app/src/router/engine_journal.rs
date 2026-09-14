//! Bounded engine-neutral journal on the existing Conversation delivery owner.
//! Event codecs belong to engines; this component owns durable sequencing,
//! write lifetime and one-shot model-operation claims, not effect authority.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use nomifun_agent_contracts::ResolvedSnapshotRef;
use nomifun_chat_model_broker::{
    ChatCausality, ChatCausalityGate, ChatModelError, ChatModelErrorCode, ChatRetryDirective,
};
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use super::engine_session_host::EngineTurnReceipt;

/// The trusted host classifies a write. Cleanup/Terminal reserve journal space
/// but do NOT certify cleanup; only actual effect owners can supply that proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineJournalWrite {
    Progress,
    /// A result of an already admitted operation. Does not reopen progress
    /// after cancellation or itself assert that a process tree has exited.
    Settlement,
    Cleanup,
    Terminal,
}

#[derive(Default)]
struct Cursor {
    sequence: i64,
    bytes: usize,
    draining: bool,
    terminal: bool,
    uncertain: bool,
}

pub(super) struct Journal {
    pool: SqlitePool,
    user: String,
    conversation: String,
    operation: String,
    root: String,
    epoch: i64,
    snapshot: ResolvedSnapshotRef,
    route: Option<nomifun_chat_model_broker::ChatRouteSelection>,
    cancellation: CancellationToken,
    cursor: Mutex<Cursor>,
    sequence: AtomicU64,
    pending: Arc<Semaphore>,
    pending_bytes: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct EngineTurnJournal(Arc<Journal>);

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine journal: {message}"))
}

impl EngineTurnJournal {
    /// A resource read must follow a model operation already claimed by the
    /// Broker. This does not claim it again or grant any resource capability.
    pub(super) async fn require_claimed_model(&self, causality: &ChatCausality) -> Result<(), AppError> {
        let journal = &self.0;
        let cursor = journal.cursor.lock().await;
        if cursor.uncertain || cursor.terminal || cursor.draining || journal.cancellation.is_cancelled()
            || causality.agent_session_id.as_ref() != journal.conversation
            || causality.turn_operation_id.as_ref() != journal.operation
            || causality.causation_event_id.as_ref() != journal.root
            || causality.resolved_snapshot_ref != journal.snapshot
            || Some(&causality.route_identity) != journal.route.as_ref() {
            return Err(failure("resource request differs from the live model turn"));
        }
        let (valid,): (i64,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM conversations c JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id \
            WHERE c.conversation_id = ? AND c.user_id = ? AND c.admission_epoch = ? AND c.active_turn_operation_id = ? AND c.status = 'running' \
            AND r.user_id = c.user_id AND r.conversation_id = c.conversation_id AND r.message_id = ? AND r.kind = 'turn' AND r.status = 'accepted' \
            AND EXISTS(SELECT 1 FROM conversation_runtime_events e WHERE e.conversation_id = c.conversation_id AND e.turn_operation_id = r.operation_id AND e.model_operation_id = ? AND e.model_claimed = 1))")
            .bind(&journal.conversation).bind(&journal.user).bind(journal.epoch).bind(&journal.operation).bind(&journal.root)
            .bind(causality.operation_id.as_ref()).fetch_one(&journal.pool).await.map_err(|_| failure("resource model authority unavailable"))?;
        if valid != 1 { return Err(failure("resource request has no claimed model operation")); }
        Ok(())
    }

    pub(super) fn validate_receipt(&self, receipt: &EngineTurnReceipt) -> Result<(), AppError> {
        if self.0.conversation != receipt.session().session().conversation_id
            || self.0.operation != receipt.operation_id()
        {
            return Err(failure("journal belongs to another resource turn"));
        }
        Self::from_existing(self.0.clone(), receipt).map(|_| ())
    }
    /// Identity check only. The actual live-turn fence is the subsequent
    /// Progress insert; Kernel still independently admits the capability.
    pub(super) fn matches_tool(
        &self,
        invocation: &nomifun_engine_core::EngineToolInvocation,
    ) -> bool {
        invocation.agent_session_id.as_ref() == self.0.conversation
            && invocation.principal.principal_kind == "user"
            && invocation.principal.principal_id == self.0.user
            && invocation.resolved_snapshot_ref == self.0.snapshot
            && invocation.turn_operation_id.as_ref() == self.0.operation
    }

    pub(super) fn downgrade(&self) -> std::sync::Weak<Journal> {
        Arc::downgrade(&self.0)
    }
    pub(super) fn from_existing(
        journal: Arc<Journal>,
        receipt: &EngineTurnReceipt,
    ) -> Result<Self, AppError> {
        if journal.user != receipt.session().principal().principal_id
            || journal.epoch != receipt.admission_epoch()
            || journal.root != receipt.root_message_id()
            || journal.snapshot != receipt.session().snapshot().snapshot_ref
        {
            return Err(failure("receipt changed for existing journal"));
        }
        Ok(Self(journal))
    }
    pub(super) fn new(
        pool: SqlitePool,
        receipt: &EngineTurnReceipt,
        cancellation: CancellationToken,
    ) -> Self {
        Self(Arc::new(Journal {
            pool,
            user: receipt.session().principal().principal_id.clone(),
            conversation: receipt.session().session().conversation_id.clone(),
            operation: receipt.operation_id().into(),
            root: receipt.root_message_id().into(),
            epoch: receipt.admission_epoch(),
            snapshot: receipt.session().snapshot().snapshot_ref.clone(),
            route: receipt
                .session()
                .snapshot()
                .content
                .chat_route_identity
                .clone(),
            cancellation,
            cursor: Mutex::new(Cursor::default()),
            sequence: AtomicU64::new(0),
            pending: Arc::new(Semaphore::new(64)),
            pending_bytes: Arc::new(Semaphore::new(8 * 1024 * 1024)),
        }))
    }

    /// Last committed sequence. This observation alone grants no authority.
    pub fn sequence(&self) -> u64 {
        self.0.sequence.load(Ordering::Acquire)
    }

    /// The write task survives a dropped waiter. SQL, sequence accounting and
    /// phase updates complete together under the journal lock. Admission of an
    /// effect must await success; starting this write is not sufficient.
    pub async fn append(
        &self,
        payload: String,
        model_operation: Option<String>,
        kind: EngineJournalWrite,
    ) -> Result<(), AppError> {
        if payload.len() > 8 * 1024 * 1024 {
            return Err(failure("record exceeds hard byte limit"));
        }
        if model_operation
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 1024)
            || (model_operation.is_some() && kind != EngineJournalWrite::Progress)
        {
            return Err(failure("invalid model-operation admission"));
        }
        let permit = self
            .0
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| failure("pending write bound reached"))?;
        let byte_permit = self
            .0
            .pending_bytes
            .clone()
            .try_acquire_many_owned(payload.len() as u32)
            .map_err(|_| failure("pending write byte bound reached"))?;
        serde_json::from_str::<serde_json::Value>(&payload).map_err(failure)?;
        let executor = tokio::runtime::Handle::try_current().map_err(failure)?;
        let journal = self.0.clone();
        let task = executor.spawn(async move {
            let _permit = permit;
            let _byte_permit = byte_permit;
            let mut cursor = journal.cursor.lock().await;
            if cursor.uncertain || cursor.terminal { return Err(failure("journal is closed or uncertain")); }
            if kind == EngineJournalWrite::Progress && (cursor.draining || journal.cancellation.is_cancelled()) {
                return Err(failure("turn no longer admits progress"));
            }
            if kind == EngineJournalWrite::Terminal && !cursor.draining {
                return Err(failure("terminal requires host cleanup phase"));
            }
            let reserved = matches!(kind, EngineJournalWrite::Cleanup | EngineJournalWrite::Terminal)
                || (kind == EngineJournalWrite::Settlement && (cursor.draining || journal.cancellation.is_cancelled()));
            let (records, bytes) = if reserved { (4095, 8 * 1024 * 1024) } else { (3200, 4 * 1024 * 1024) };
            let next_bytes = cursor.bytes.saturating_add(payload.len());
            if cursor.sequence >= records || next_bytes > bytes { return Err(failure("bounded evidence journal exhausted")); }
            // Uncertainty sticks if SQL errors or the task panics. Never reuse
            // a possibly committed sequence or continue admitting effects.
            cursor.uncertain = true;
            let inserted = sqlx::query(
                "INSERT INTO conversation_runtime_events (conversation_id, turn_operation_id, sequence, event_json, model_operation_id, created_at) \
                 SELECT ?, ?, ?, ?, ?, ? WHERE EXISTS(SELECT 1 FROM conversations c JOIN conversation_delivery_receipts r \
                 ON r.conversation_id = c.conversation_id AND r.user_id = c.user_id \
                 WHERE c.conversation_id = ? AND c.user_id = ? AND r.operation_id = ? AND r.kind = 'turn' AND r.message_id = ? \
                 AND (? = 1 OR (c.status = 'running' AND c.admission_epoch = ? AND c.active_turn_operation_id = r.operation_id AND r.status = 'accepted')))")
                .bind(&journal.conversation).bind(&journal.operation).bind(cursor.sequence + 1).bind(payload).bind(model_operation)
                .bind(nomifun_common::now_ms()).bind(&journal.conversation).bind(&journal.user).bind(&journal.operation).bind(&journal.root)
                .bind(i64::from(kind != EngineJournalWrite::Progress)).bind(journal.epoch)
                .execute(&journal.pool).await.map_err(failure)?;
            if inserted.rows_affected() != 1 { return Err(failure("Conversation owner fenced this write")); }
            cursor.sequence += 1;
            cursor.bytes = next_bytes;
            cursor.draining |= matches!(kind, EngineJournalWrite::Cleanup | EngineJournalWrite::Terminal);
            cursor.terminal = kind == EngineJournalWrite::Terminal;
            cursor.uncertain = false;
            journal.sequence.store(cursor.sequence as u64, Ordering::Release);
            Ok(())
        });
        task.await.map_err(failure)?
    }
}

#[async_trait]
impl ChatCausalityGate for EngineTurnJournal {
    async fn authorize(&self, causality: &ChatCausality) -> Result<(), ChatModelError> {
        let reject = |reason: &str| {
            ChatModelError::new(
                ChatModelErrorCode::CausalityRejected,
                reason,
                ChatRetryDirective::Never,
            )
        };
        let journal = &self.0;
        let cursor = journal.cursor.lock().await;
        if cursor.uncertain
            || cursor.terminal
            || cursor.draining
            || journal.cancellation.is_cancelled()
            || causality.agent_session_id.as_ref() != journal.conversation
            || causality.turn_operation_id.as_ref() != journal.operation
            || causality.causation_event_id.as_ref() != journal.root
            || causality.resolved_snapshot_ref != journal.snapshot
            || Some(&causality.route_identity) != journal.route.as_ref()
        {
            return Err(reject("model request differs from admitted live turn"));
        }
        let claimed = sqlx::query("UPDATE conversation_runtime_events SET model_claimed = 1 \
            WHERE conversation_id = ? AND turn_operation_id = ? AND model_operation_id = ? AND model_claimed = 0 \
            AND EXISTS(SELECT 1 FROM conversations c JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id \
                WHERE c.conversation_id = ? AND c.user_id = ? AND c.status = 'running' AND c.admission_epoch = ? \
                AND c.active_turn_operation_id = ? AND r.conversation_id = c.conversation_id AND r.user_id = c.user_id \
                AND r.kind = 'turn' AND r.status = 'accepted' AND r.message_id = ? \
                AND NOT EXISTS(SELECT 1 FROM conversation_hosted_effects h WHERE h.user_id = c.user_id \
                    AND h.conversation_id = c.conversation_id AND h.state = 'pending') \
                AND NOT EXISTS(SELECT 1 FROM conversation_mcp_effects m WHERE m.user_id = c.user_id \
                    AND m.conversation_id = c.conversation_id AND m.state = 'pending'))")
            .bind(&journal.conversation).bind(&journal.operation).bind(causality.operation_id.as_ref())
            .bind(&journal.conversation).bind(&journal.user).bind(journal.epoch).bind(&journal.operation).bind(&journal.root)
            .execute(&journal.pool).await.map_err(|_| reject("cannot establish durable model authority"))?;
        if claimed.rows_affected() != 1 {
            return Err(reject(
                "model operation already claimed or generation fenced",
            ));
        }
        Ok(())
    }
}

// Existing tool-ownership tests use the actual journal SQL with a minimal
// in-memory owner fixture. This constructor is absent from production builds.
#[cfg(test)]
pub(super) async fn test_fixture() -> (EngineTurnJournal, SqlitePool) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for sql in [
        "CREATE TABLE conversations (conversation_id TEXT, user_id TEXT, status TEXT, admission_epoch INTEGER, active_turn_operation_id TEXT)",
        "CREATE TABLE conversation_delivery_receipts (conversation_id TEXT, user_id TEXT, operation_id TEXT, kind TEXT, message_id TEXT, status TEXT)",
        "CREATE TABLE conversation_runtime_events (conversation_id TEXT, turn_operation_id TEXT, sequence INTEGER, event_json TEXT, model_operation_id TEXT UNIQUE, model_claimed INTEGER DEFAULT 0, created_at INTEGER, UNIQUE(conversation_id, turn_operation_id, sequence))",
        "CREATE TABLE conversation_hosted_effects (user_id TEXT, conversation_id TEXT, state TEXT)",
        "CREATE TABLE conversation_mcp_effects (user_id TEXT, conversation_id TEXT, state TEXT)",
        "INSERT INTO conversations VALUES ('session', 'owner', 'running', 1, 'turn')",
        "INSERT INTO conversation_delivery_receipts VALUES ('session', 'owner', 'turn', 'turn', 'root', 'accepted')",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    let journal = EngineTurnJournal(Arc::new(Journal {
        pool: pool.clone(),
        user: "owner".into(),
        conversation: "session".into(),
        operation: "turn".into(),
        root: "root".into(),
        epoch: 1,
        snapshot: ResolvedSnapshotRef {
            snapshot_id: "snapshot".into(),
            snapshot_digest: "a".repeat(64).into(),
        },
        route: None,
        cancellation: CancellationToken::new(),
        cursor: Mutex::new(Cursor::default()),
        sequence: AtomicU64::new(0),
        pending: Arc::new(Semaphore::new(64)),
        pending_bytes: Arc::new(Semaphore::new(8 * 1024 * 1024)),
    }));
    (journal, pool)
}
