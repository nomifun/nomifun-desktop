//! Read-only observation of database operation scopes. No pool/driver hooks,
//! SQLite calls, admission authority or persistent diagnostic ledger.
use nomifun_common::DatabaseUnavailableReason;
use sqlx::SqlitePool;
use sqlx::Connection;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::Span;

const SLOW_OPERATION: Duration = Duration::from_secs(2);
const REPORT_INTERVAL: Duration = Duration::from_secs(5);
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);
static OPERATIONS: OnceLock<Mutex<BTreeMap<u64, Arc<Operation>>>> = OnceLock::new();
static LAST_REPORT: Mutex<Option<Instant>> = Mutex::new(None);

/// Observe the existing serialized Agent writer without holding a connection
/// during caller backoff. The returned guard follows the actual transaction,
/// rather than ending at BEGIN and mistaking its wait for write-lock duration.
#[track_caller]
pub fn begin_agent_write(pool:&SqlitePool)
    -> impl std::future::Future<Output=Result<(sqlx::Transaction<'static,sqlx::Sqlite>,DatabaseOperationGuard),sqlx::Error>> + '_ {
    let observed=DatabaseOperationGuard::new(pool);
    async move {
        for _ in 0..=pool.options().get_max_connections() {
            let mut connection=pool.acquire().await?;
            observed.acquired();
            // SQLITE_FULL may roll back inside SQLite before SQLx observes it.
            // Drain normal queued rollback; evict only this poisoned connection.
            if connection.is_in_transaction() {
                connection.ping().await?;
                if connection.is_in_transaction() {connection.close().await?;continue;}
            }
            observed.beginning_transaction();
            let transaction=sqlx::Transaction::begin(connection,Some("BEGIN IMMEDIATE".into())).await?;
            observed.transaction_started();
            return Ok((transaction,observed));
        }
        Err(sqlx::Error::InvalidSavePointStatement)
    }
}

#[derive(Debug, Default)]
struct Phase {
    acquired: Option<Instant>, begin_requested: Option<Instant>, transaction_started: Option<Instant>,
    completing: Option<Instant>, released: Option<Instant>,
}

#[derive(Debug)]
struct Operation {
    id: u64,
    started: Instant,
    pool: SqlitePool,
    span: Span,
    caller: &'static std::panic::Location<'static>,
    phase: Mutex<Phase>,
}

/// Observes a repository/transaction boundary without touching SQLite or adding
/// an await to pool acquisition/release. Drop only removes diagnostics; it never
/// releases a connection, changes a receipt or starts work.
#[derive(Debug)]
pub struct DatabaseOperationGuard(Arc<Operation>);

impl DatabaseOperationGuard {
    #[track_caller]
    pub fn new(pool: &SqlitePool) -> Self {
        let operation = Arc::new(Operation {
            id: NEXT_OPERATION.fetch_add(1, Ordering::Relaxed), started: Instant::now(),
            pool: pool.clone(), span: Span::current(), caller: std::panic::Location::caller(), phase: Mutex::new(Phase::default()),
        });
        OPERATIONS.get_or_init(Mutex::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(operation.id, operation.clone());
        Self(operation)
    }

    /// Call only after the connection/transaction was actually obtained.
    pub fn acquired(&self) {
        *self.0.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Phase { acquired: Some(Instant::now()), ..Phase::default() };
    }

    /// BEGIN succeeded: only this point proves acquisition of the SQLite writer.
    pub fn beginning_transaction(&self) {
        self.0.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner).begin_requested=Some(Instant::now());
    }

    pub fn transaction_started(&self) {
        self.0.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner).transaction_started=Some(Instant::now());
    }

    pub async fn commit(self, transaction:sqlx::Transaction<'_,sqlx::Sqlite>)->Result<(),sqlx::Error> {
        self.0.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner).completing=Some(Instant::now());
        let result=transaction.commit().await;
        if result.is_ok() {self.released();}
        result
    }

    pub async fn rollback(self, transaction:sqlx::Transaction<'_,sqlx::Sqlite>)->Result<(),sqlx::Error> {
        self.0.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner).completing=Some(Instant::now());
        let result=transaction.rollback().await;
        if result.is_ok() {self.released();}
        result
    }

    /// Call after an acknowledged commit/rollback, before unrelated CPU work.
    /// SQLx may still return its connection asynchronously.
    pub fn released(&self) {
        self.0.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner).released = Some(Instant::now());
    }
}

impl Drop for DatabaseOperationGuard {
    fn drop(&mut self) {
        OPERATIONS.get_or_init(Mutex::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&self.0.id);
        if self.0.started.elapsed() >= SLOW_OPERATION { self.0.warn("finished_slow_operation"); }
    }
}

impl Operation {
    fn warn(&self, stage: &'static str) {
        let phase = self.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (admission_ms, connection_scope_ms) = match phase.acquired {
            Some(acquired) => (acquired.duration_since(self.started).as_millis() as u64,
                Some(phase.released.unwrap_or_else(Instant::now).duration_since(acquired).as_millis() as u64)),
            None => (self.started.elapsed().as_millis() as u64, None),
        };
        let now=Instant::now();
        let begin_wait_ms=phase.begin_requested.map(|started|phase.transaction_started.unwrap_or(now).duration_since(started).as_millis() as u64);
        let transaction_ms=phase.transaction_started.map(|started|phase.released.unwrap_or(now).duration_since(started).as_millis() as u64);
        let completion_wait_ms=phase.completing.map(|started|phase.released.unwrap_or(now).duration_since(started).as_millis() as u64);
        let completion_acknowledged=phase.released.is_some();
        drop(phase);
        self.span.in_scope(|| tracing::warn!(target: "nomifun_db::pool",
            database_operation_id = self.id, stage, admission_ms, connection_scope_ms,
            begin_wait_ms,transaction_ms,completion_wait_ms,completion_acknowledged,
            pool_size = self.pool.size(), pool_idle = self.pool.num_idle(),
            operation = self.span.metadata().map_or("unscoped", |metadata| metadata.name()),
            source = %self.caller,
            "database operation scope observation"));
    }
}

pub(crate) fn warn_contended_pools(reason: DatabaseUnavailableReason) {
    let mut last = LAST_REPORT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if last.is_some_and(|last| last.elapsed() < REPORT_INTERVAL) { return; }
    *last = Some(Instant::now());
    drop(last);
    let operations = OPERATIONS.get_or_init(Mutex::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .values().filter(|operation| operation.started.elapsed() >= SLOW_OPERATION).cloned().collect::<Vec<_>>();
    tracing::warn!(target: "nomifun_db::pool", ?reason, slow_operations = operations.len(),
        "temporary database admission failure; scope observations are not proof of a connection leak");
    for operation in operations { operation.warn("admission_failure"); }
}

#[cfg(test)]
#[path = "pool_diagnostics_tests.rs"]
mod tests;
