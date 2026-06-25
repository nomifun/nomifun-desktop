use crate::error::DbError;
use crate::models::IdmmInterventionRow;

/// Data access for `idmm_interventions`. Aggressive eviction lives here:
/// `insert` prunes the target down to PER_TARGET_CAP after writing.
#[async_trait::async_trait]
pub trait IIdmmInterventionRepository: Send + Sync {
    /// Insert one record, then prune this target to the most-recent PER_TARGET_CAP.
    async fn insert(&self, row: &IdmmInterventionRow) -> Result<(), DbError>;

    /// Most-recent-first, capped at `limit`.
    async fn list_for_target(
        &self,
        target_kind: &str,
        target_id: &str,
        limit: i64,
    ) -> Result<Vec<IdmmInterventionRow>, DbError>;

    /// Delete all records for a target (manual clear + session-delete cascade). Returns count.
    async fn delete_for_target(&self, target_kind: &str, target_id: &str) -> Result<u64, DbError>;

    /// Most-recent-first across ALL targets, capped at `limit` (cross-session feed).
    async fn list_recent(&self, limit: i64) -> Result<Vec<IdmmInterventionRow>, DbError>;

    /// Delete every record across all targets. Returns count.
    async fn clear_all(&self) -> Result<u64, DbError>;

    /// TTL sweep: delete rows older than `cutoff_ms` + enforce global hard cap. Returns count.
    async fn sweep(&self, cutoff_ms: i64, global_cap: i64) -> Result<u64, DbError>;
}

/// Keep only the newest 30 records per target (data is disposable).
pub const PER_TARGET_CAP: i64 = 30;
/// TTL: 48 hours.
pub const TTL_MS: i64 = 48 * 60 * 60 * 1000;
/// Global backstop.
pub const GLOBAL_CAP: i64 = 2000;
