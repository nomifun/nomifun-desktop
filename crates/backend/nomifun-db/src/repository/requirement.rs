use nomifun_common::TimestampMs;

use crate::error::DbError;
use crate::models::{RequirementRow, RequirementRowUpdate, RequirementTagRow};

/// Filters + pagination for listing requirements.
#[derive(Debug, Clone, Default)]
pub struct ListRequirementsParams {
    pub tag: Option<String>,
    pub status: Option<String>,
    /// Filter by the executing session (a conversation or terminal id). Matches
    /// the `owner_session_id` column (`idx_requirements_owner`).
    pub owner_session_id: Option<i64>,
    /// Filter by the owner domain (`"conversation"` | `"terminal"`). Paired with
    /// `owner_session_id` it disambiguates the dual-domain owner column — after
    /// integerization a conversation and a terminal can share a numeric id, so a
    /// session-scoped query (e.g. clearing a deleted session's requirements) MUST
    /// constrain `owner_kind` too, or it crosses domains (spec §2.2).
    pub owner_kind: Option<String>,
    /// Substring search over title + content (case-insensitive).
    pub q: Option<String>,
    /// 1-based page index. Defaults to 1 when None.
    pub page: Option<u32>,
    /// Page size. Defaults to 20 when None.
    pub page_size: Option<u32>,
}

/// Data access abstraction for the `requirements` table.
#[async_trait::async_trait]
pub trait IRequirementRepository: Send + Sync {
    /// Insert a new requirement row. The `id` field of `row` is ignored: the id
    /// is allocated by SQLite (INTEGER PK AUTOINCREMENT) and returned.
    async fn insert(&self, row: &RequirementRow) -> Result<i64, DbError>;

    /// Partial update by ID. Returns `DbError::NotFound` if absent.
    async fn update(&self, id: i64, params: &RequirementRowUpdate) -> Result<(), DbError>;

    /// Delete by ID. Returns `DbError::NotFound` if absent.
    async fn delete(&self, id: i64) -> Result<(), DbError>;

    /// Fetch a single requirement by ID.
    async fn get_by_id(&self, id: i64) -> Result<Option<RequirementRow>, DbError>;

    /// List with filters + pagination. Returns `(rows, total_matching)`.
    async fn list(&self, params: &ListRequirementsParams) -> Result<(Vec<RequirementRow>, u64), DbError>;

    /// All requirements for a tag, ordered by `sort_seq ASC, priority DESC, created_at ASC`.
    async fn list_by_tag(&self, tag: &str) -> Result<Vec<RequirementRow>, DbError>;

    /// Distinct tags with per-status counts. Returns rows of `(tag, status, count)`.
    async fn tag_status_counts(&self) -> Result<Vec<(String, String, i64)>, DbError>;

    /// Atomically claim the next pending requirement for `tag`.
    ///
    /// Single `UPDATE … WHERE id = (SELECT … LIMIT 1) RETURNING *` — SQLite's
    /// single-writer guarantee makes this the entire idempotent allocator.
    /// Records the executing session as `owner_session_id` + `owner_kind`
    /// (`'conversation'` | `'terminal'`), set together to satisfy the table's
    /// paired-NULL CHECK. Returns the claimed row, or `None` when the tag has
    /// no pending requirements.
    async fn claim_next(
        &self,
        tag: &str,
        owner_session_id: i64,
        owner_kind: &str,
        lease_ms: i64,
        now: TimestampMs,
    ) -> Result<Option<RequirementRow>, DbError>;

    /// Renew the lease for a requirement currently claimed by `owner` (matched
    /// against `owner_session_id`). Returns true if a row was renewed.
    async fn renew_lease(&self, id: i64, owner: i64, lease_ms: i64, now: TimestampMs) -> Result<bool, DbError>;

    /// Re-pend in_progress requirements whose lease expired and whose owning
    /// session is no longer active. Each active entry is a `(owner_kind,
    /// owner_session_id)` pair — both are matched together, because the
    /// integer owner id is dual-domain (a conversation and a terminal can share
    /// a number), so a kind-less match would wrongly treat an active `conv#5`
    /// as keeping a stale `term#5` claim alive (spec §2.2). Returns the count reset.
    async fn sweep_expired_leases(
        &self,
        active_sessions: &[(String, i64)],
        now: TimestampMs,
    ) -> Result<u64, DbError>;

    // ── AutoWork tag-level pause (Step 1) ──────────────────────────────

    /// Pause a tag (lazily upserts the row). Idempotent: re-pausing updates the
    /// reason / triggering requirement. After this, `claim_next(tag, …)` yields
    /// `None` until `resume_tag`.
    async fn pause_tag(
        &self,
        tag: &str,
        reason: &str,
        req_id: Option<i64>,
        now: TimestampMs,
    ) -> Result<(), DbError>;

    /// Resume a paused tag (clears the paused flag). No-op if the tag has no row
    /// (absent = not paused).
    async fn resume_tag(&self, tag: &str) -> Result<(), DbError>;

    /// Whether `tag` is currently paused.
    async fn is_tag_paused(&self, tag: &str) -> Result<bool, DbError>;

    /// Full pause state for a tag, if a row exists (`None` = never paused).
    async fn get_tag_state(&self, tag: &str) -> Result<Option<RequirementTagRow>, DbError>;

    /// Revert a claim WITHOUT consuming an attempt — e.g. the inject was
    /// rejected because the session was busy. Resets the row to `pending`,
    /// decrements `attempt_count` (floored at 0), and clears the owner fields
    /// (`owner_session_id` + `owner_kind`, paired). Guarded by
    /// `status='in_progress' AND owner_session_id=owner`. Returns whether a
    /// row was reverted. Distinct from the error re-pend path, which DOES keep
    /// the consumed attempt.
    async fn unclaim(&self, id: i64, owner: i64) -> Result<bool, DbError>;
}
