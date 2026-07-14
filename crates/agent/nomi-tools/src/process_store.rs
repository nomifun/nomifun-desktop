//! Numeric-session adapter shared by `exec_command` and `write_stdin`.
//!
//! The shared process supervisor owns every process and transport. This store
//! retains only owner-qualified session identities plus incremental output
//! cursor metadata; it never owns a PTY or OS process.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use nomi_process_runtime::{
    ProcessOwner, OutputCursor, OutputSnapshot, SessionId, Transport,
};
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

/// Upper bound on retained numeric session mappings.
///
/// This matches the shared supervisor's default capacity. Reaching the cap is
/// an explicit error: silently evicting a live mapping would leave an owned
/// process running without any numeric identifier through which the command
/// tools could address it.
pub const MAX_PROCESSES: usize = 64;

/// Immutable data installed when a running supervisor session is assigned a
/// numeric identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumericSessionBinding {
    pub owner: ProcessOwner,
    pub session_id: SessionId,
    pub cursor: OutputCursor,
    pub dropped_bytes: u64,
    pub transport: Transport,
}

impl NumericSessionBinding {
    pub fn new(
        owner: ProcessOwner,
        session_id: SessionId,
        cursor: OutputCursor,
        dropped_bytes: u64,
        transport: Transport,
    ) -> Self {
        Self {
            owner,
            session_id,
            cursor,
            dropped_bytes,
            transport,
        }
    }

    /// Build a binding after the initial `exec_command` poll has already
    /// delivered `output` to the caller.
    pub fn after_output(
        owner: ProcessOwner,
        session_id: SessionId,
        transport: Transport,
        output: &OutputSnapshot,
    ) -> Self {
        Self::new(
            owner,
            session_id,
            output.next_cursor,
            output.dropped_bytes,
            transport,
        )
    }
}

/// One owner-qualified supervisor identity behind a numeric id.
///
/// `state` is deliberately per-entry. A `write_stdin` call may hold this mutex
/// across a long poll without blocking lookups or operations on other sessions,
/// while concurrent calls for the same numeric id cannot race the durable
/// output cursor backwards.
#[derive(Debug)]
pub struct NumericSessionEntry {
    owner: ProcessOwner,
    session_id: SessionId,
    transport: Transport,
    state: AsyncMutex<NumericSessionState>,
}

impl NumericSessionEntry {
    fn new(binding: NumericSessionBinding) -> Self {
        Self {
            owner: binding.owner,
            session_id: binding.session_id,
            transport: binding.transport,
            state: AsyncMutex::new(NumericSessionState {
                cursor: binding.cursor,
                dropped_bytes: binding.dropped_bytes,
            }),
        }
    }

    pub fn owner(&self) -> &ProcessOwner {
        &self.owner
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub const fn transport(&self) -> Transport {
        self.transport
    }

    pub async fn lock_state(&self) -> AsyncMutexGuard<'_, NumericSessionState> {
        self.state.lock().await
    }
}

/// Durable per-call progress for one numeric session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumericSessionState {
    cursor: OutputCursor,
    dropped_bytes: u64,
}

impl NumericSessionState {
    pub const fn cursor(&self) -> OutputCursor {
        self.cursor
    }

    /// Lifetime-cumulative bytes discarded by the supervisor output buffer as
    /// of the last successfully delivered tool result.
    pub const fn dropped_bytes(&self) -> u64 {
        self.dropped_bytes
    }

    /// Commit a supervisor snapshot after its corresponding tool result is
    /// ready to be returned.
    ///
    /// `OutputSnapshot::dropped_bytes` is lifetime-cumulative and therefore is
    /// not the number of bytes this caller missed. The exact cursor gap is:
    ///
    /// `max(snapshot retained-base - previous cursor, 0)`.
    ///
    /// This distinction matters when output already consumed by an earlier call
    /// is later evicted: cumulative dropped bytes increase, but the caller did
    /// not lose those already-consumed bytes.
    pub fn record_output(&mut self, output: &OutputSnapshot) -> OutputObservation {
        let previous_cursor = self.cursor;
        let previous_dropped_bytes = self.dropped_bytes;
        let missed_bytes = missed_bytes(output, previous_cursor);

        // A stale or malformed snapshot must never move durable progress
        // backwards. Normal supervisor snapshots are strictly monotonic.
        self.cursor = self.cursor.max(output.next_cursor);
        self.dropped_bytes = self.dropped_bytes.max(output.dropped_bytes);

        OutputObservation {
            previous_cursor,
            next_cursor: self.cursor,
            missed_bytes,
            newly_dropped_bytes: self
                .dropped_bytes
                .saturating_sub(previous_dropped_bytes),
            cumulative_dropped_bytes: self.dropped_bytes,
        }
    }
}

pub(crate) fn missed_bytes(
    output: &OutputSnapshot,
    previous_cursor: OutputCursor,
) -> u64 {
    let retained_bytes = u64::try_from(output.retained_bytes).unwrap_or(u64::MAX);
    let retained_base = output.next_cursor.offset().saturating_sub(retained_bytes);
    retained_base.saturating_sub(previous_cursor.offset())
}

/// Metadata produced while atomically advancing a numeric session cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputObservation {
    pub previous_cursor: OutputCursor,
    pub next_cursor: OutputCursor,
    /// Bytes the caller could not receive because its previous cursor was older
    /// than the supervisor's retained-output base.
    pub missed_bytes: u64,
    /// Increase in the supervisor's lifetime-cumulative drop counter since this
    /// numeric adapter last committed a result. This is diagnostic metadata and
    /// is not necessarily equal to `missed_bytes`.
    pub newly_dropped_bytes: u64,
    pub cumulative_dropped_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericSessionStoreError {
    CapacityExhausted { max_sessions: usize },
    NumericIdExhausted,
}

impl fmt::Display for NumericSessionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityExhausted { max_sessions } => write!(
                formatter,
                "numeric session capacity is exhausted (max sessions: {max_sessions})"
            ),
            Self::NumericIdExhausted => {
                formatter.write_str("numeric session id space is exhausted")
            }
        }
    }
}

impl Error for NumericSessionStoreError {}

/// Short-lock adapter from numeric ids to owner-qualified supervisor
/// sessions.
pub struct ProcessStore {
    inner: Mutex<HashMap<u64, Arc<NumericSessionEntry>>>,
    next_id: AtomicU64,
}

impl Default for ProcessStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            // Zero was historically used as a missing/invalid id in tool
            // descriptions and diagnostics, so valid ids start at one.
            next_id: AtomicU64::new(1),
        }
    }

    pub fn insert(
        &self,
        binding: NumericSessionBinding,
    ) -> Result<u64, NumericSessionStoreError> {
        let mut sessions = self
            .inner
            .lock()
            .expect("numeric session store mutex is poisoned");
        if sessions.len() >= MAX_PROCESSES {
            return Err(NumericSessionStoreError::CapacityExhausted {
                max_sessions: MAX_PROCESSES,
            });
        }

        let id = self
            .next_available_id(&sessions)
            .ok_or(NumericSessionStoreError::NumericIdExhausted)?;
        let replaced = sessions.insert(id, Arc::new(NumericSessionEntry::new(binding)));
        debug_assert!(replaced.is_none(), "numeric session id was allocated twice");
        Ok(id)
    }

    pub fn get(&self, id: u64) -> Option<Arc<NumericSessionEntry>> {
        self.inner
            .lock()
            .expect("numeric session store mutex is poisoned")
            .get(&id)
            .cloned()
    }

    pub fn remove(&self, id: u64) -> Option<Arc<NumericSessionEntry>> {
        self.inner
            .lock()
            .expect("numeric session store mutex is poisoned")
            .remove(&id)
    }

    /// Remove `id` only if it still refers to the exact entry previously
    /// returned by `get`.
    pub fn remove_if_same(&self, id: u64, expected: &Arc<NumericSessionEntry>) -> bool {
        let mut sessions = self
            .inner
            .lock()
            .expect("numeric session store mutex is poisoned");
        let should_remove = sessions
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, expected));
        if should_remove {
            sessions.remove(&id);
        }
        should_remove
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("numeric session store mutex is poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, id: u64) -> bool {
        self.inner
            .lock()
            .expect("numeric session store mutex is poisoned")
            .contains_key(&id)
    }

    pub fn entries(&self) -> Vec<(u64, Arc<NumericSessionEntry>)> {
        self.inner
            .lock()
            .expect("numeric session store mutex is poisoned")
            .iter()
            .map(|(id, entry)| (*id, Arc::clone(entry)))
            .collect()
    }

    fn next_available_id(
        &self,
        sessions: &HashMap<u64, Arc<NumericSessionEntry>>,
    ) -> Option<u64> {
        // At most 64 ids are live, so MAX_PROCESSES + 1 probes are enough to
        // skip zero and any collision after the atomic counter wraps.
        for _ in 0..=MAX_PROCESSES {
            let candidate = self.next_id.fetch_add(1, Ordering::Relaxed);
            if candidate != 0 && !sessions.contains_key(&candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_process_runtime::{EncodingMetadata, OutputChunk, OutputStream};
    use uuid::Uuid;

    fn owner() -> ProcessOwner {
        ProcessOwner::new(Uuid::now_v7(), Uuid::now_v7())
    }

    fn binding(cursor: u64, dropped_bytes: u64, transport: Transport) -> NumericSessionBinding {
        NumericSessionBinding::new(
            owner(),
            SessionId::new(),
            OutputCursor::new(cursor),
            dropped_bytes,
            transport,
        )
    }

    fn output(
        next_cursor: u64,
        retained_bytes: usize,
        dropped_bytes: u64,
    ) -> OutputSnapshot {
        OutputSnapshot {
            chunks: Vec::<OutputChunk>::new(),
            next_cursor: OutputCursor::new(next_cursor),
            retained_bytes,
            dropped_bytes,
            encoding: EncodingMetadata::default(),
        }
    }

    #[test]
    fn numeric_ids_are_monotonic_and_lookup_preserves_identity() {
        let store = ProcessStore::new();
        let first_binding = binding(3, 1, Transport::Pipe);
        let expected_owner = first_binding.owner.clone();
        let expected_session = first_binding.session_id;
        let first = store.insert(first_binding).expect("first insert");
        let second = store
            .insert(binding(
                0,
                0,
                Transport::Pty { cols: 120, rows: 30 },
            ))
            .expect("second insert");

        assert_eq!(first, 1);
        assert_eq!(second, 2);
        let entry = store.get(first).expect("inserted mapping");
        assert_eq!(entry.owner(), &expected_owner);
        assert_eq!(entry.session_id(), expected_session);
        assert_eq!(entry.transport(), Transport::Pipe);
        assert_eq!(store.len(), 2);
        assert!(store.contains(second));
    }

    #[test]
    fn binding_after_output_starts_after_the_delivered_snapshot() {
        let delivered = output(42, 10, 32);
        let owner = owner();
        let session_id = SessionId::new();
        let binding = NumericSessionBinding::after_output(
            owner.clone(),
            session_id,
            Transport::Pipe,
            &delivered,
        );

        assert_eq!(binding.owner, owner);
        assert_eq!(binding.session_id, session_id);
        assert_eq!(binding.cursor, OutputCursor::new(42));
        assert_eq!(binding.dropped_bytes, 32);
        assert_eq!(binding.transport, Transport::Pipe);
    }

    #[tokio::test]
    async fn record_output_reports_exact_cursor_gap_not_cumulative_drop_delta() {
        let entry = NumericSessionEntry::new(binding(5, 2, Transport::Pipe));
        let mut state = entry.lock_state().await;

        // next=10, retained=4 => retained base is 6. Cursor 5 missed one byte,
        // while the cumulative dropped counter increased by four.
        let first = state.record_output(&output(10, 4, 6));
        assert_eq!(first.previous_cursor, OutputCursor::new(5));
        assert_eq!(first.next_cursor, OutputCursor::new(10));
        assert_eq!(first.missed_bytes, 1);
        assert_eq!(first.newly_dropped_bytes, 4);
        assert_eq!(first.cumulative_dropped_bytes, 6);

        // next=12, retained=4 => base 8, which is behind the committed cursor
        // 10. Two more lifetime bytes were evicted, but they were already
        // consumed and therefore this caller missed nothing.
        let second = state.record_output(&output(12, 4, 8));
        assert_eq!(second.previous_cursor, OutputCursor::new(10));
        assert_eq!(second.next_cursor, OutputCursor::new(12));
        assert_eq!(second.missed_bytes, 0);
        assert_eq!(second.newly_dropped_bytes, 2);
        assert_eq!(second.cumulative_dropped_bytes, 8);
        assert_eq!(state.cursor(), OutputCursor::new(12));
        assert_eq!(state.dropped_bytes(), 8);
    }

    #[tokio::test]
    async fn replaying_the_same_snapshot_does_not_repeat_drop_metadata() {
        let entry = NumericSessionEntry::new(binding(0, 0, Transport::Pipe));
        let mut state = entry.lock_state().await;
        let snapshot = output(10, 4, 6);

        let first = state.record_output(&snapshot);
        let repeated = state.record_output(&snapshot);

        assert_eq!(first.missed_bytes, 6);
        assert_eq!(first.newly_dropped_bytes, 6);
        assert_eq!(repeated.missed_bytes, 0);
        assert_eq!(repeated.newly_dropped_bytes, 0);
        assert_eq!(repeated.next_cursor, OutputCursor::new(10));
    }

    #[tokio::test]
    async fn stale_snapshot_cannot_move_cursor_or_drop_counter_backwards() {
        let entry = NumericSessionEntry::new(binding(10, 7, Transport::Pipe));
        let mut state = entry.lock_state().await;

        let observed = state.record_output(&output(8, 4, 5));

        assert_eq!(observed.previous_cursor, OutputCursor::new(10));
        assert_eq!(observed.next_cursor, OutputCursor::new(10));
        assert_eq!(observed.missed_bytes, 0);
        assert_eq!(observed.newly_dropped_bytes, 0);
        assert_eq!(observed.cumulative_dropped_bytes, 7);
        assert_eq!(state.cursor(), OutputCursor::new(10));
        assert_eq!(state.dropped_bytes(), 7);
    }

    #[tokio::test]
    async fn each_entry_serializes_cursor_mutation_independently() {
        let store = ProcessStore::new();
        let id = store
            .insert(binding(0, 0, Transport::Pipe))
            .expect("insert");
        let entry = store.get(id).expect("entry");
        let guard = entry.lock_state().await;

        let contender = Arc::clone(&entry);
        let (attempted_tx, attempted_rx) = tokio::sync::oneshot::channel();
        let (acquired_tx, mut acquired_rx) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            attempted_tx.send(()).expect("attempt receiver");
            let _guard = contender.lock_state().await;
            acquired_tx.send(()).expect("acquired receiver");
        });

        attempted_rx.await.expect("contender started");
        tokio::task::yield_now().await;
        assert!(
            matches!(
                acquired_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "a second call for the same numeric id must wait for the cursor guard"
        );

        drop(guard);
        acquired_rx.await.expect("contender acquired after release");
        worker.await.expect("contender task");
    }

    #[test]
    fn capacity_failure_never_evicts_an_existing_mapping() {
        let store = ProcessStore::new();
        let mut ids = Vec::new();
        for _ in 0..MAX_PROCESSES {
            ids.push(
                store
                    .insert(binding(0, 0, Transport::Pipe))
                    .expect("mapping below capacity"),
            );
        }

        let error = store
            .insert(binding(0, 0, Transport::Pipe))
            .expect_err("mapping above capacity must fail");

        assert_eq!(
            error,
            NumericSessionStoreError::CapacityExhausted {
                max_sessions: MAX_PROCESSES
            }
        );
        assert_eq!(store.len(), MAX_PROCESSES);
        assert!(ids.into_iter().all(|id| store.contains(id)));
    }

    #[test]
    fn remove_if_same_cannot_remove_a_different_entry() {
        let store = ProcessStore::new();
        let first = store
            .insert(binding(0, 0, Transport::Pipe))
            .expect("first");
        let second = store
            .insert(binding(0, 0, Transport::Pipe))
            .expect("second");
        let first_entry = store.get(first).expect("first entry");
        let second_entry = store.get(second).expect("second entry");

        assert!(!store.remove_if_same(first, &second_entry));
        assert!(store.contains(first));
        assert!(store.remove_if_same(first, &first_entry));
        assert!(!store.contains(first));
        assert!(store.contains(second));
    }

    #[test]
    fn remove_and_empty_queries_are_process_free() {
        let store = ProcessStore::new();
        assert!(store.is_empty());
        let id = store
            .insert(binding(
                0,
                0,
                Transport::Pty { cols: 80, rows: 24 },
            ))
            .expect("insert");
        let removed = store.remove(id).expect("remove");

        assert_eq!(removed.transport(), Transport::Pty { cols: 80, rows: 24 });
        assert!(store.is_empty());
        assert!(store.get(id).is_none());
    }

    #[test]
    fn test_snapshot_helper_uses_no_stream_or_process_state() {
        let snapshot = OutputSnapshot {
            chunks: vec![OutputChunk {
                seq: 0,
                start: 0,
                stream: OutputStream::Stdout,
                bytes: b"ignored by adapter".to_vec(),
                text: "ignored by adapter".to_owned(),
            }],
            next_cursor: OutputCursor::new(18),
            retained_bytes: 18,
            dropped_bytes: 0,
            encoding: EncodingMetadata::default(),
        };
        let binding = NumericSessionBinding::after_output(
            owner(),
            SessionId::new(),
            Transport::Pipe,
            &snapshot,
        );

        assert_eq!(binding.cursor, OutputCursor::new(18));
        assert_eq!(binding.dropped_bytes, 0);
    }
}
