//! The companion's dedicated sqlite store (`{companion_dir}/memory.db`): memories,
//! companion-chat history, and a small
//! key-value state table (xp/mood/cursor/rolling chat summary).
//!
//! Deliberately a separate db file from the main app database so "clear all
//! companion data" stays a file-scoped operation and companion writes never contend with
//! conversation traffic.

use std::path::Path;

use nomifun_common::{
    AppError, CompanionId, CompanionMemoryId,
    CompanionSessionWindowId, CompanionSkillPatternId, ConversationId,
    TimestampMs, now_ms, validate_uuidv7,
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

/// Memory kinds — the six-dimension taxonomy from the design doc.
pub const MEMORY_KINDS: [&str; 6] = ["profile", "preference", "knowledge", "episode", "task", "affective"];

/// Per-kind decay half-life in days. `profile` does not decay.
fn half_life_days(kind: &str) -> Option<f64> {
    match kind {
        "episode" => Some(7.0),
        "task" => Some(14.0),
        "affective" => Some(21.0),
        "knowledge" | "preference" => Some(60.0),
        _ => None, // profile
    }
}

/// Below this strength a memory is auto-archived (still restorable in the UI).
const ARCHIVE_THRESHOLD: f64 = 0.05;

/// Validate the owner of a row about to be written: `Some` must be a canonical
/// companion id, `None` (not yet owned) is always legal.
///
/// 共享记忆 was removed as a product concept: every memory belongs to exactly one
/// companion, so ownership is ONE nullable column (`companion_memories.companion_id`)
/// and no writer creates an unowned row from scratch. `None` stays legal at the DB
/// level because a zero-companion install is a supported state and because
/// pre-re-homing rows must survive verbatim until the boot migration assigns them
/// an owner. Those rows are readable by every companion — unowned means "not yet
/// assigned", never "shared on purpose".
fn validate_row_owner(owner: Option<&str>) -> Result<(), AppError> {
    match owner {
        None => Ok(()),
        Some(id) => validate_companion_id(id, "memory companion_id"),
    }
}

/// Who is mutating a memory row — the enforcement side of "a memory can only be
/// changed by its owner". Every mutator that addresses a row by `memory_id` takes
/// one, and it is REQUIRED: an `Option<&str>` owner would read as "no check"
/// wherever it is `None`, which is precisely how the invariant stayed unenforced
/// while the callers happened to only know their own ids.
///
/// The cross-companion escape is therefore a named variant, not an absent value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryActor {
    /// One companion. It may only mutate rows it can READ — its own plus the
    /// vestigial unowned ones — i.e. exactly [`MEMORY_VISIBILITY_PREDICATE`]:
    /// a companion that can see a row in its own list can act on it, and can
    /// touch nothing else.
    Companion(String),
    /// Every row, whoever owns it. The machine owner's administrative surface
    /// only (`nomi_memory_*` over MCP), which has no companion identity to scope
    /// to and lists across the whole install by design. Never reachable from a
    /// companion workspace.
    AnyOwner,
}

impl MemoryActor {
    /// Validate the embedded id once, at the entry of each public mutator.
    fn validate(&self) -> Result<(), AppError> {
        match self {
            MemoryActor::AnyOwner => Ok(()),
            MemoryActor::Companion(id) => validate_companion_id(id, "memory actor companion_id"),
        }
    }

    /// Row-level twin of [`MEMORY_VISIBILITY_PREDICATE`]: true iff this actor may
    /// reach a row carrying this owner. `visibility_rule_matches_sql` keeps the
    /// two in lockstep.
    fn can_reach(&self, companion_id: Option<&str>) -> bool {
        match self {
            MemoryActor::AnyOwner => true,
            MemoryActor::Companion(id) => {
                companion_id.is_none() || companion_id == Some(id.as_str())
            }
        }
    }
}

/// The one error a memory address failure produces — whether the row is absent
/// or owned by somebody else. `NotFound` rather than `Forbidden` on purpose: a
/// companion has no business learning that another companion's row exists, so
/// the response can never be used as an existence oracle. What it must never be
/// is a silent no-op reported as success.
fn memory_not_found(memory_id: &str) -> AppError {
    AppError::NotFound(format!("memory '{memory_id}' not found"))
}

/// Address one memory row for mutation by `actor`, enforcing the ownership
/// invariant, and hand back what the FTS mirror needs (`rowid` + the OLD
/// indexed content, verbatim, for the `delete` command).
///
/// - `Ok(Some(..))` — the row exists and this actor may mutate it.
/// - `Ok(None)` — no such row on the install at all. Each caller decides what
///   that means (an error for update/batch/merge, a no-op for delete).
/// - `Err(NotFound)` — the row exists but belongs to another companion.
async fn locate_memory_for_mutation<'e, E>(
    executor: E,
    memory_id: &str,
    actor: &MemoryActor,
) -> Result<Option<(i64, String)>, AppError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, content, companion_id FROM companion_memories WHERE memory_id = ?",
    )
    .bind(memory_id)
    .fetch_optional(executor)
    .await
    .map_err(db_err)?;
    let Some(row) = row else { return Ok(None) };
    let companion_id: Option<String> = row.get("companion_id");
    if !actor.can_reach(companion_id.as_deref()) {
        return Err(memory_not_found(memory_id));
    }
    Ok(Some((row.get("id"), row.get("content"))))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionMemory {
    pub memory_id: String,
    pub kind: String,
    pub content: String,
    pub tags: Vec<String>,
    pub importance: f64,
    pub strength: f64,
    pub pinned: bool,
    pub source: String,
    pub status: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    pub last_reinforced_at: TimestampMs,
    /// The owning companion (`companion_memories.companion_id`); `None` only for
    /// a vestigial row the boot migration has not re-homed yet.
    ///
    /// Serialized under the column's own name: the field used to travel as
    /// `scope_companion_id` next to a retired `scope_kind` discriminator, and
    /// both retired names are now gone from the wire. [`crate::export`] accepts
    /// and translates them when importing a bundle written by an older build.
    pub companion_id: Option<String>,
}

/// One registered companion chat thread (a real `type='nomi'` conversation
/// owned by the main conversation domain; the companion only tracks membership).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionThread {
    pub conversation_id: String,
    /// Owning canonical companion UUIDv7. Ownerless rows are invalid.
    pub companion_id: String,
    pub title: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// One archived (or currently open) companion session window — a bounded span
/// of the companion's single chat thread. Closed on ≥`idle_minutes` of
/// inactivity, compressed into a day-partitioned `digest`, after which the live
/// engine context is reset (`clear_context`) so the next window starts small.
/// `session_day` is the window's LOCAL start day (`YYYYMMDD`) — the partition key
/// for "去年今日" recall, so a cross-midnight session stays attributed to the day
/// it began.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWindow {
    pub session_window_id: String,
    pub companion_id: String,
    pub conversation_id: String,
    pub session_day: String,
    pub started_at: TimestampMs,
    pub last_activity_at: TimestampMs,
    pub closed_at: Option<TimestampMs>,
    /// `open` | `archived` | `skipped` (too little content to summarize).
    pub status: String,
    pub message_count: i64,
    /// Only messages with `created_at > boundary_ts` belong to this window.
    pub boundary_ts: TimestampMs,
    pub digest: Option<String>,
    /// JSON blob of structured highlights (topics/decisions/mood/todos).
    pub highlights: Option<String>,
    pub token_estimate: i64,
}

/// One durable mined-pattern sample. This fixed JSON structure replaces the
/// historical delimiter-concatenated pseudo-ID representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatternExample {
    conversation_id: ConversationId,
    #[serde(deserialize_with = "deserialize_uuidv7_string")]
    event_id: String,
}

fn deserialize_uuidv7_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_uuidv7(&value).map_err(serde::de::Error::custom)?;
    Ok(value)
}

fn deserialize_uuidv7_strings<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    for value in &values {
        validate_uuidv7(value).map_err(serde::de::Error::custom)?;
    }
    Ok(values)
}

fn deserialize_optional_uuidv7_string<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if let Some(value) = value.as_deref() {
        validate_uuidv7(value).map_err(serde::de::Error::custom)?;
    }
    Ok(value)
}

/// Filter for `list_memories`.
#[derive(Debug, Default, Clone)]
pub struct MemoryFilter {
    pub kind: Option<String>,
    pub q: Option<String>,
    pub status: Option<String>,
    /// When set, return only memories this companion may read: its own plus
    /// the vestigial unowned (`companion_id IS NULL`) rows the boot migration has
    /// not re-homed yet. `None` returns every memory (the owner-agent
    /// administrative view).
    pub companion_id: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

/// One page of memories and the number of rows matching the same filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPage {
    pub items: Vec<CompanionMemory>,
    pub total: i64,
}

/// Sort order for the paged memory list (non-FTS path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryListSort {
    /// Legacy default: pinned first, then strength, then recency.
    Default,
    /// Pure recency (`updated_at DESC`).
    Time,
    /// Pinned first, then importance.
    Importance,
}

fn memory_filter_clause(filter: &MemoryFilter) -> String {
    let mut sql = String::from(" WHERE 1=1");
    if filter.kind.is_some() {
        sql.push_str(" AND kind = ?");
    }
    if filter.q.is_some() {
        sql.push_str(" AND content LIKE ?");
    }
    if filter.status.is_some() {
        sql.push_str(" AND status = ?");
    }
    if filter.companion_id.is_some() {
        sql.push_str(MEMORY_VISIBILITY_PREDICATE);
    }
    sql
}

/// The single visibility predicate every companion-facing read uses: the
/// companion's own memories plus the vestigial unowned rows
/// (`companion_id IS NULL`) that the boot migration re-homes as soon as a
/// companion exists. Keeping the unowned half is deliberate — a row the
/// migration has not reached yet must stay readable rather than silently vanish
/// from every prompt.
pub(crate) const MEMORY_VISIBILITY_PREDICATE: &str =
    " AND (companion_id IS NULL OR companion_id = ?)";

/// The normalized-similarity predicate shared by the write-path dedup guard
/// (`find_similar_active`) and the merge-assistant grouping: equal after
/// trim+lowercase, or containment in either direction when the two are close
/// in length (≥ 0.6 short/long char ratio).
pub(crate) fn memory_contents_similar(a: &str, b: &str) -> bool {
    const CONTAINMENT_MIN_RATIO: f64 = 0.6;
    let norm_a = a.trim().to_lowercase();
    let norm_b = b.trim().to_lowercase();
    if norm_a == norm_b {
        return true;
    }
    let (short_len, long_len) = {
        let la = norm_a.chars().count();
        let lb = norm_b.chars().count();
        (la.min(lb), la.max(lb))
    };
    let close_in_length = long_len > 0 && (short_len as f64 / long_len as f64) >= CONTAINMENT_MIN_RATIO;
    close_in_length && (norm_a.contains(&norm_b) || norm_b.contains(&norm_a))
}

/// One batched memory operation, applied atomically to a set of ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryBatchAction {
    Archive,
    Restore,
    Delete,
    /// Move the memories to another of the six kinds.
    Reclassify { kind: String },
}

#[derive(Clone)]
pub struct CompanionStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryImportStats {
    pub imported: u64,
    pub skipped_duplicates: u64,
}

/// An open SQLite transaction containing a fully validated memory-bundle
/// merge. The export/import layer publishes staged event files only after this
/// object has been created; it then commits the DB transaction. Dropping or
/// explicitly rolling back this value leaves the existing store unchanged.
pub(crate) struct MemoryImportTransaction<'a> {
    tx: sqlx::Transaction<'a, sqlx::Sqlite>,
    stats: MemoryImportStats,
}

impl MemoryImportTransaction<'_> {
    pub(crate) fn stats(&self) -> MemoryImportStats {
        self.stats
    }

    pub(crate) async fn commit(self) -> Result<MemoryImportStats, AppError> {
        self.tx.commit().await.map_err(db_err)?;
        Ok(self.stats)
    }

    pub(crate) async fn rollback(self) -> Result<(), AppError> {
        self.tx.rollback().await.map_err(db_err)
    }
}

impl CompanionStore {
    /// Validate and stage a complete memory-bundle merge in one SQLite
    /// transaction. No row is visible to other connections until the returned
    /// transaction is committed.
    pub(crate) async fn begin_memory_import(
        &self,
        memories: &[CompanionMemory],
    ) -> Result<MemoryImportTransaction<'_>, AppError> {
        for memory in memories {
            CompanionMemoryId::try_from(memory.memory_id.as_str())
                .map_err(|error| AppError::BadRequest(format!("invalid imported memory id: {error}")))?;
            validate_row_owner(memory.companion_id.as_deref())?;
        }
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let mut imported = 0u64;
        let mut skipped_duplicates = 0u64;

        for memory in memories {
            let existing = sqlx::query("SELECT * FROM companion_memories WHERE memory_id = ?")
                .bind(&memory.memory_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_err)?;
            if let Some(row) = existing {
                let local = row_to_memory(&row)?;
                if local == *memory {
                    skipped_duplicates += 1;
                    continue;
                }
                return Err(AppError::Conflict(format!(
                    "memory import ID collision for {}: local and imported content differ",
                    memory.memory_id
                )));
            }

            if memory.status == "active" {
                // Content dedup is OWNER-scoped, exactly like the write-path
                // guard [`CompanionStore::find_similar_active`]: 共享记忆已删除，
                // 一条记忆只在它自己的主人名下才可能"重复"。装机级去重会让一条
                // 要落到甲名下的导入记忆，因为**乙**恰好有一条相似记忆而被静默
                // 丢弃 —— 甲永远拿不到它，而那条"重复"记忆根本不属于甲。
                //
                // An unowned import (empty roster) is compared only against the
                // other unowned rows, which is all there is in that state.
                let mut sql = String::from(
                    "SELECT memory_id, content FROM companion_memories WHERE kind = ? AND status = 'active'",
                );
                match memory.companion_id.as_deref() {
                    Some(_) => sql.push_str(MEMORY_VISIBILITY_PREDICATE),
                    None => sql.push_str(" AND companion_id IS NULL"),
                }
                let mut similar_query = sqlx::query(&sql).bind(&memory.kind);
                if let Some(owner) = memory.companion_id.as_deref() {
                    similar_query = similar_query.bind(owner);
                }
                let similar = similar_query.fetch_all(&mut *tx).await.map_err(db_err)?;
                let normalized = memory.content.clone();
                let duplicate = similar.into_iter().any(|row| {
                    let existing_content: String = row.get("content");
                    memory_contents_similar(&normalized, &existing_content)
                });
                if duplicate {
                    skipped_duplicates += 1;
                    continue;
                }
            }

            let row = sqlx::query(
                "INSERT INTO companion_memories(memory_id, kind, content, tags, importance, strength, pinned, source, status, created_at, updated_at, last_reinforced_at, companion_id)
                 VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)
                 RETURNING id",
            )
            .bind(&memory.memory_id)
            .bind(&memory.kind)
            .bind(&memory.content)
            .bind(serde_json::to_string(&memory.tags).map_err(|error| {
                AppError::BadRequest(format!("invalid imported memory tags: {error}"))
            })?)
            .bind(memory.importance)
            .bind(memory.strength)
            .bind(memory.pinned as i64)
            .bind(&memory.source)
            .bind(&memory.status)
            .bind(memory.created_at)
            .bind(memory.updated_at)
            .bind(memory.last_reinforced_at)
            .bind(&memory.companion_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
            fts_index_insert(&mut *tx, row.get("id"), &memory.content).await?;
            imported += 1;
        }

        Ok(MemoryImportTransaction {
            tx,
            stats: MemoryImportStats {
                imported,
                skipped_duplicates,
            },
        })
    }
}

/// Runtime state that used to be a single global row in `companion_state` and is
/// per companion since 学习 / 进化 became per-companion (2026-08). Copied onto every
/// companion once by [`CompanionStore::seed_companion_state_from_global`], which
/// then DELETES the global rows: they have no reader left, and keeping them "in
/// case of a rollback" was never true — the same upgrade drops
/// `companion_suggestions`, and an older build validates an exact table set, so a
/// downgrade fails at boot no matter what these rows say.
pub const MIGRATED_GLOBAL_STATE_KEYS: &[&str] = &[
    // How far each companion's loops have consumed the shared raw event spool.
    // The retention watermark reads these, so a wrong value here deletes events.
    crate::collector::LEARN_CURSOR_KEY,
    crate::collector::EVOLVE_CURSOR_KEY,
    // Schedule stamps: seeded so the whole roster does not fire at once on the
    // first tick after the upgrade.
    "last_learn_ts",
    "last_evolve_ts",
    // Give-up counter for a model that keeps returning unparseable learn output.
    "learn_parse_fail_streak",
    // The companion's current mood word.
    MOOD_KEY,
];

/// `companion_runtime_state` key for one companion's mood word.
pub const MOOD_KEY: &str = "mood";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS companion_memories (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  memory_id TEXT NOT NULL UNIQUE CHECK (
    length(memory_id) = 36
    AND lower(memory_id) = memory_id
    AND memory_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(memory_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  kind TEXT NOT NULL,
  content TEXT NOT NULL,
  tags TEXT NOT NULL DEFAULT '[]',
  importance REAL NOT NULL DEFAULT 0.5,
  strength REAL NOT NULL DEFAULT 0.5,
  pinned INTEGER NOT NULL DEFAULT 0,
  source TEXT NOT NULL DEFAULT 'learn',
  status TEXT NOT NULL DEFAULT 'active',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  last_reinforced_at INTEGER NOT NULL,
  companion_id TEXT CHECK (
    companion_id IS NULL
    OR (
      length(companion_id) = 36
      AND lower(companion_id) = companion_id
      AND companion_id GLOB '????????-????-7???-[89ab]???-????????????'
      AND replace(companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
  ),
  embedding BLOB,
  embedding_model TEXT
);
CREATE INDEX IF NOT EXISTS idx_companion_memories_kind ON companion_memories(kind, status, strength DESC);

CREATE TABLE IF NOT EXISTS companion_state (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  state_key TEXT NOT NULL UNIQUE,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS companion_threads (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  conversation_id TEXT NOT NULL UNIQUE CHECK (
    length(conversation_id) = 36
    AND lower(conversation_id) = conversation_id
    AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  companion_id TEXT NOT NULL UNIQUE CHECK (
    length(companion_id) = 36
    AND lower(companion_id) = companion_id
    AND companion_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  title TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS companion_runtime_state (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  companion_id TEXT NOT NULL CHECK (
    length(companion_id) = 36
    AND lower(companion_id) = companion_id
    AND companion_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  state_key TEXT NOT NULL,
  value TEXT NOT NULL,
  UNIQUE(companion_id, state_key)
);

CREATE TABLE IF NOT EXISTS companion_skills (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  companion_skill_id TEXT NOT NULL UNIQUE CHECK (
    length(companion_skill_id) = 36
    AND lower(companion_skill_id) = companion_skill_id
    AND companion_skill_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(companion_skill_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  skill_name TEXT NOT NULL,
  companion_id TEXT CHECK (
    companion_id IS NULL
    OR (
      length(companion_id) = 36
      AND lower(companion_id) = companion_id
      AND companion_id GLOB '????????-????-7???-[89ab]???-????????????'
      AND replace(companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
  ),
  status TEXT NOT NULL DEFAULT 'draft',
  source TEXT NOT NULL DEFAULT 'mined',
  confidence REAL NOT NULL DEFAULT 0.0,
  provenance_event_ids TEXT NOT NULL DEFAULT '[]',
  strength REAL NOT NULL DEFAULT 1.0,
  version INTEGER NOT NULL DEFAULT 1,
  skill_pattern_id TEXT CHECK (
    skill_pattern_id IS NULL OR (
      length(skill_pattern_id) = 36
      AND lower(skill_pattern_id) = skill_pattern_id
      AND skill_pattern_id GLOB '????????-????-7???-[89ab]???-????????????'
      AND replace(skill_pattern_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
  ),
  usage_count INTEGER NOT NULL DEFAULT 0,
  last_used_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  signature TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_companion_skills_owner ON companion_skills(companion_id, status, strength DESC);
-- Kept on purpose after 共享技能 was removed as a product concept: no writer
-- creates an unowned row any more, but a zero-companion install has no legal
-- owner to re-home the legacy ones onto (see backfill_skill_owner), and while
-- they sit there this is the only thing keeping two of them from claiming the
-- same {user_skills_dir}/shared/{name} directory.
CREATE UNIQUE INDEX IF NOT EXISTS idx_companion_skills_shared_name ON companion_skills(skill_name) WHERE companion_id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_companion_skills_private_owner_name ON companion_skills(companion_id, skill_name) WHERE companion_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS skill_pattern_stats (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  skill_pattern_id TEXT NOT NULL UNIQUE CHECK (
    length(skill_pattern_id) = 36
    AND lower(skill_pattern_id) = skill_pattern_id
    AND skill_pattern_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(skill_pattern_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  signature TEXT NOT NULL,
  occurrence_count INTEGER NOT NULL DEFAULT 0,
  distinct_sessions INTEGER NOT NULL DEFAULT 0,
  examples TEXT NOT NULL DEFAULT '[]',
  status TEXT NOT NULL DEFAULT 'open',
  last_seen INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS evolution_feedback (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  feedback_id TEXT NOT NULL UNIQUE CHECK (
    length(feedback_id) = 36
    AND lower(feedback_id) = feedback_id
    AND feedback_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(feedback_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  companion_skill_id TEXT NOT NULL CHECK (
    length(companion_skill_id) = 36
    AND lower(companion_skill_id) = companion_skill_id
    AND companion_skill_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(companion_skill_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  skill_name_snapshot TEXT NOT NULL,
  skill_pattern_id TEXT CHECK (
    skill_pattern_id IS NULL OR (
      length(skill_pattern_id) = 36
      AND lower(skill_pattern_id) = skill_pattern_id
      AND skill_pattern_id GLOB '????????-????-7???-[89ab]???-????????????'
      AND replace(skill_pattern_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
  ),
  signature_snapshot TEXT,
  decision TEXT NOT NULL,
  reason TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_skill_pattern_signature ON skill_pattern_stats(signature);
CREATE INDEX IF NOT EXISTS idx_evolution_feedback_skill ON evolution_feedback(companion_skill_id);
CREATE INDEX IF NOT EXISTS idx_evolution_feedback_pattern ON evolution_feedback(skill_pattern_id);

CREATE TABLE IF NOT EXISTS companion_session_windows (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_window_id TEXT NOT NULL UNIQUE CHECK (
    length(session_window_id) = 36
    AND lower(session_window_id) = session_window_id
    AND session_window_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(session_window_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  companion_id TEXT NOT NULL CHECK (
    length(companion_id) = 36
    AND lower(companion_id) = companion_id
    AND companion_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  conversation_id TEXT NOT NULL CHECK (
    length(conversation_id) = 36
    AND lower(conversation_id) = conversation_id
    AND conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
    AND replace(conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
  ),
  session_day TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  last_activity_at INTEGER NOT NULL,
  closed_at INTEGER,
  status TEXT NOT NULL DEFAULT 'open',
  message_count INTEGER NOT NULL DEFAULT 0,
  boundary_ts INTEGER NOT NULL DEFAULT 0,
  digest TEXT,
  highlights TEXT,
  token_estimate INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_csw_companion_day ON companion_session_windows(companion_id, session_day);
CREATE INDEX IF NOT EXISTS idx_csw_status ON companion_session_windows(companion_id, status, last_activity_at);
"#;

/// External-content FTS5 index over `companion_memories.content` (trigram
/// tokenizer → CJK substring search). Kept out of `SCHEMA` so the legacy-layout
/// upgrade tests can express "current schema minus the FTS stanza" precisely.
/// The write paths below maintain the index in code (v3 forbids triggers).
const FTS_SCHEMA: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS companion_memories_fts USING fts5(
  content, content='companion_memories', content_rowid='id', tokenize='trigram'
);
"#;

/// The FTS virtual table plus the shadow tables SQLite materializes for it.
/// They belong to the v3 baseline table set but carry no per-table contract of
/// their own (their shape is owned by SQLite).
const FTS_TABLE: &str = "companion_memories_fts";
const FTS_SHADOW_TABLES: &[&str] = &[
    "companion_memories_fts_data",
    "companion_memories_fts_idx",
    "companion_memories_fts_docsize",
    "companion_memories_fts_config",
];

pub(crate) fn db_err(e: sqlx::Error) -> AppError {
    AppError::Internal(format!("companion store: {e}"))
}

/// External-content FTS5 maintenance: index one `companion_memories` row.
/// (v3 forbids triggers, so every write path calls these in code.)
async fn fts_index_insert<'e, E>(executor: E, rowid: i64, content: &str) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query("INSERT INTO companion_memories_fts(rowid, content) VALUES(?, ?)")
        .bind(rowid)
        .bind(content)
        .execute(executor)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// External-content FTS5 maintenance: drop one row from the index. The OLD
/// content must match what was indexed (the fts5 'delete' command contract).
async fn fts_index_delete<'e, E>(executor: E, rowid: i64, content: &str) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO companion_memories_fts(companion_memories_fts, rowid, content) VALUES('delete', ?, ?)",
    )
    .bind(rowid)
    .bind(content)
    .execute(executor)
    .await
    .map_err(db_err)?;
    Ok(())
}

fn validate_companion_id(value: &str, field: &str) -> Result<(), AppError> {
    CompanionId::try_from(value)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid {field}: {error}")))
}

fn validate_conversation_id(value: &str, field: &str) -> Result<(), AppError> {
    ConversationId::try_from(value)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid {field}: {error}")))
}

fn invalid_disk_id(field: &str, value: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!(
        "companion store contains non-canonical {field} {value:?}: {error}"
    ))
}

/// Companion side-store v3 is a hard baseline. The app-level factory reset
/// removes any non-v3 dataset before this crate starts, so this crate creates
/// only the current schema and never transforms existing rows.
const STORE_VERSION: i64 = 3;

#[derive(Debug, Clone, Copy)]
struct ColumnContract {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    primary_key_position: i64,
}

#[derive(Debug, Clone, Copy)]
struct UniqueIndexContract {
    columns: &'static [&'static str],
    origin: &'static str,
    partial: bool,
}

#[derive(Debug, Clone, Copy)]
struct TableContract {
    name: &'static str,
    columns: &'static [ColumnContract],
    uuidv7_columns: &'static [&'static str],
    unique_indexes: &'static [UniqueIndexContract],
    required_sql_fragments: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct IndexColumnContract {
    name: &'static str,
    descending: bool,
}

#[derive(Debug, Clone, Copy)]
struct NamedIndexContract {
    name: &'static str,
    table: &'static str,
    unique: bool,
    partial: bool,
    columns: &'static [IndexColumnContract],
    where_fragment: Option<&'static str>,
}

const BASELINE_TABLES: &[TableContract] = &[
    TableContract {
        name: "companion_memories",
        columns: &[
            ColumnContract { name: "id", declared_type: "INTEGER", not_null: false, primary_key_position: 1 },
            ColumnContract { name: "memory_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "kind", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "content", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "tags", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "importance", declared_type: "REAL", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "strength", declared_type: "REAL", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "pinned", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "source", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "status", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "created_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "updated_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "last_reinforced_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "companion_id", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "embedding", declared_type: "BLOB", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "embedding_model", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
        ],
        uuidv7_columns: &["memory_id", "companion_id"],
        unique_indexes: &[UniqueIndexContract { columns: &["memory_id"], origin: "u", partial: false }],
        // Ownership is ONE nullable column now, so there is no cross-column
        // CHECK left to pin down: `uuidv7_columns` already asserts the shape of
        // `companion_id`, and NULL (not yet owned) needs no discriminator.
        required_sql_fragments: &[],
    },
    TableContract {
        name: "companion_state",
        columns: &[
            ColumnContract { name: "id", declared_type: "INTEGER", not_null: false, primary_key_position: 1 },
            ColumnContract { name: "state_key", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "value", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
        ],
        uuidv7_columns: &[],
        unique_indexes: &[UniqueIndexContract { columns: &["state_key"], origin: "u", partial: false }],
        required_sql_fragments: &[],
    },
    TableContract {
        name: "companion_threads",
        columns: &[
            ColumnContract { name: "id", declared_type: "INTEGER", not_null: false, primary_key_position: 1 },
            ColumnContract { name: "conversation_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "companion_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "title", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "created_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "updated_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
        ],
        uuidv7_columns: &["conversation_id", "companion_id"],
        unique_indexes: &[
            UniqueIndexContract { columns: &["conversation_id"], origin: "u", partial: false },
            UniqueIndexContract { columns: &["companion_id"], origin: "u", partial: false },
        ],
        required_sql_fragments: &[],
    },
    TableContract {
        name: "companion_runtime_state",
        columns: &[
            ColumnContract { name: "id", declared_type: "INTEGER", not_null: false, primary_key_position: 1 },
            ColumnContract { name: "companion_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "state_key", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "value", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
        ],
        uuidv7_columns: &["companion_id"],
        unique_indexes: &[UniqueIndexContract { columns: &["companion_id", "state_key"], origin: "u", partial: false }],
        required_sql_fragments: &[],
    },
    TableContract {
        name: "companion_skills",
        columns: &[
            ColumnContract { name: "id", declared_type: "INTEGER", not_null: false, primary_key_position: 1 },
            ColumnContract { name: "companion_skill_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "skill_name", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "companion_id", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "status", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "source", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "confidence", declared_type: "REAL", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "provenance_event_ids", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "strength", declared_type: "REAL", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "version", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "skill_pattern_id", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "usage_count", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "last_used_at", declared_type: "INTEGER", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "created_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "updated_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "signature", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
        ],
        uuidv7_columns: &["companion_skill_id", "companion_id", "skill_pattern_id"],
        unique_indexes: &[
            UniqueIndexContract { columns: &["companion_skill_id"], origin: "u", partial: false },
            UniqueIndexContract { columns: &["skill_name"], origin: "c", partial: true },
            UniqueIndexContract { columns: &["companion_id", "skill_name"], origin: "c", partial: true },
        ],
        required_sql_fragments: &[],
    },
    TableContract {
        name: "skill_pattern_stats",
        columns: &[
            ColumnContract { name: "id", declared_type: "INTEGER", not_null: false, primary_key_position: 1 },
            ColumnContract { name: "skill_pattern_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "signature", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "occurrence_count", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "distinct_sessions", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "examples", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "status", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "last_seen", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
        ],
        uuidv7_columns: &["skill_pattern_id"],
        unique_indexes: &[UniqueIndexContract { columns: &["skill_pattern_id"], origin: "u", partial: false }],
        required_sql_fragments: &[],
    },
    TableContract {
        name: "evolution_feedback",
        columns: &[
            ColumnContract { name: "id", declared_type: "INTEGER", not_null: false, primary_key_position: 1 },
            ColumnContract { name: "feedback_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "companion_skill_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "skill_name_snapshot", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "skill_pattern_id", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "signature_snapshot", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "decision", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "reason", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "created_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
        ],
        uuidv7_columns: &["feedback_id", "companion_skill_id", "skill_pattern_id"],
        unique_indexes: &[UniqueIndexContract { columns: &["feedback_id"], origin: "u", partial: false }],
        required_sql_fragments: &[],
    },
    TableContract {
        name: "companion_session_windows",
        columns: &[
            ColumnContract { name: "id", declared_type: "INTEGER", not_null: false, primary_key_position: 1 },
            ColumnContract { name: "session_window_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "companion_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "conversation_id", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "session_day", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "started_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "last_activity_at", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "closed_at", declared_type: "INTEGER", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "status", declared_type: "TEXT", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "message_count", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "boundary_ts", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
            ColumnContract { name: "digest", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "highlights", declared_type: "TEXT", not_null: false, primary_key_position: 0 },
            ColumnContract { name: "token_estimate", declared_type: "INTEGER", not_null: true, primary_key_position: 0 },
        ],
        uuidv7_columns: &["session_window_id", "companion_id", "conversation_id"],
        unique_indexes: &[UniqueIndexContract { columns: &["session_window_id"], origin: "u", partial: false }],
        required_sql_fragments: &[],
    },
];

const BASELINE_INDEXES: &[NamedIndexContract] = &[
    NamedIndexContract {
        name: "idx_companion_memories_kind",
        table: "companion_memories",
        unique: false,
        partial: false,
        columns: &[
            IndexColumnContract { name: "kind", descending: false },
            IndexColumnContract { name: "status", descending: false },
            IndexColumnContract { name: "strength", descending: true },
        ],
        where_fragment: None,
    },
    NamedIndexContract {
        name: "idx_companion_skills_owner",
        table: "companion_skills",
        unique: false,
        partial: false,
        columns: &[
            IndexColumnContract { name: "companion_id", descending: false },
            IndexColumnContract { name: "status", descending: false },
            IndexColumnContract { name: "strength", descending: true },
        ],
        where_fragment: None,
    },
    NamedIndexContract {
        name: "idx_companion_skills_shared_name",
        table: "companion_skills",
        unique: true,
        partial: true,
        columns: &[IndexColumnContract { name: "skill_name", descending: false }],
        where_fragment: Some("wherecompanion_idisnull"),
    },
    NamedIndexContract {
        name: "idx_companion_skills_private_owner_name",
        table: "companion_skills",
        unique: true,
        partial: true,
        columns: &[
            IndexColumnContract { name: "companion_id", descending: false },
            IndexColumnContract { name: "skill_name", descending: false },
        ],
        where_fragment: Some("wherecompanion_idisnotnull"),
    },
    NamedIndexContract {
        name: "idx_skill_pattern_signature",
        table: "skill_pattern_stats",
        unique: false,
        partial: false,
        columns: &[IndexColumnContract { name: "signature", descending: false }],
        where_fragment: None,
    },
    NamedIndexContract {
        name: "idx_evolution_feedback_skill",
        table: "evolution_feedback",
        unique: false,
        partial: false,
        columns: &[IndexColumnContract { name: "companion_skill_id", descending: false }],
        where_fragment: None,
    },
    NamedIndexContract {
        name: "idx_evolution_feedback_pattern",
        table: "evolution_feedback",
        unique: false,
        partial: false,
        columns: &[IndexColumnContract { name: "skill_pattern_id", descending: false }],
        where_fragment: None,
    },
    NamedIndexContract {
        name: "idx_csw_companion_day",
        table: "companion_session_windows",
        unique: false,
        partial: false,
        columns: &[
            IndexColumnContract { name: "companion_id", descending: false },
            IndexColumnContract { name: "session_day", descending: false },
        ],
        where_fragment: None,
    },
    NamedIndexContract {
        name: "idx_csw_status",
        table: "companion_session_windows",
        unique: false,
        partial: false,
        columns: &[
            IndexColumnContract { name: "companion_id", descending: false },
            IndexColumnContract { name: "status", descending: false },
            IndexColumnContract { name: "last_activity_at", descending: false },
        ],
        where_fragment: None,
    },
];

fn normalized_schema_sql(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

async fn index_key_columns(
    pool: &SqlitePool,
    index_name: &str,
) -> Result<Vec<(String, bool)>, AppError> {
    let rows = sqlx::query(
        "SELECT name, \"desc\" AS descending \
         FROM pragma_index_xinfo(?) WHERE \"key\" = 1 ORDER BY seqno",
    )
    .bind(index_name)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    rows.into_iter()
        .map(|row| {
            let name: Option<String> = row.try_get("name").map_err(db_err)?;
            let name = name.ok_or_else(|| {
                AppError::Internal(format!(
                    "companion store index {index_name} contains an expression instead of a column"
                ))
            })?;
            let descending = row.get::<i64, _>("descending") != 0;
            Ok((name, descending))
        })
        .collect()
}

async fn validate_table_contract(
    pool: &SqlitePool,
    contract: &TableContract,
    table_sql: &str,
) -> Result<(), AppError> {
    let columns = sqlx::query(
        "SELECT cid, name, type, \"notnull\" AS not_null, pk, hidden \
         FROM pragma_table_xinfo(?) ORDER BY cid",
    )
    .bind(contract.name)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    if columns.len() != contract.columns.len() {
        let actual: Vec<String> = columns.iter().map(|row| row.get("name")).collect();
        let expected: Vec<&str> = contract.columns.iter().map(|column| column.name).collect();
        return Err(AppError::Internal(format!(
            "companion store table {} column set is not the exact v3 baseline: expected {expected:?}, found {actual:?}",
            contract.name
        )));
    }
    for (actual, expected) in columns.iter().zip(contract.columns) {
        let name: String = actual.get("name");
        let declared_type: String = actual.get("type");
        let not_null = actual.get::<i64, _>("not_null") != 0;
        let primary_key_position = actual.get::<i64, _>("pk");
        let hidden = actual.get::<i64, _>("hidden");
        if name != expected.name
            || !declared_type.eq_ignore_ascii_case(expected.declared_type)
            || not_null != expected.not_null
            || primary_key_position != expected.primary_key_position
            || hidden != 0
        {
            return Err(AppError::Internal(format!(
                "companion store table {} column contract mismatch at {}: expected \
                 (name={}, type={}, not_null={}, pk={}), found \
                 (name={name}, type={declared_type}, not_null={not_null}, pk={primary_key_position}, hidden={hidden})",
                contract.name,
                expected.name,
                expected.name,
                expected.declared_type,
                expected.not_null,
                expected.primary_key_position,
            )));
        }
    }

    let normalized = normalized_schema_sql(table_sql);
    if !normalized.contains("idintegerprimarykeyautoincrement") {
        return Err(AppError::Internal(format!(
            "companion store table {} is not the v3 AUTOINCREMENT baseline",
            contract.name
        )));
    }
    for column in contract.uuidv7_columns {
        let required = [
            format!("length({column})=36"),
            format!("lower({column})={column}"),
            format!("{column}glob'????????-????-7???-[89ab]???-????????????'"),
            format!("replace({column},'-','')notglob'*[^0-9a-f]*'"),
        ];
        if let Some(missing) = required
            .iter()
            .find(|fragment| !normalized.contains(fragment.as_str()))
        {
            return Err(AppError::Internal(format!(
                "companion store table {} column {column} is missing UUIDv7 CHECK fragment {missing}",
                contract.name
            )));
        }
    }
    for fragment in contract.required_sql_fragments {
        if !normalized.contains(fragment) {
            return Err(AppError::Internal(format!(
                "companion store table {} is missing required CHECK fragment {fragment}",
                contract.name
            )));
        }
    }

    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct UniqueIndexShape {
        columns: Vec<String>,
        origin: String,
        partial: bool,
    }

    let index_rows = sqlx::query(
        "SELECT name, \"unique\" AS is_unique, origin, partial \
         FROM pragma_index_list(?)",
    )
    .bind(contract.name)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    let mut actual_unique = Vec::new();
    for row in index_rows {
        if row.get::<i64, _>("is_unique") == 0 {
            continue;
        }
        let index_name: String = row.get("name");
        let columns = index_key_columns(pool, &index_name)
            .await?
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        actual_unique.push(UniqueIndexShape {
            columns,
            origin: row.get("origin"),
            partial: row.get::<i64, _>("partial") != 0,
        });
    }
    actual_unique.sort();
    let mut expected_unique: Vec<UniqueIndexShape> = contract
        .unique_indexes
        .iter()
        .map(|index| UniqueIndexShape {
            columns: index.columns.iter().map(|column| (*column).to_owned()).collect(),
            origin: index.origin.to_owned(),
            partial: index.partial,
        })
        .collect();
    expected_unique.sort();
    if actual_unique != expected_unique {
        return Err(AppError::Internal(format!(
            "companion store table {} unique index contract mismatch: expected {expected_unique:?}, found {actual_unique:?}",
            contract.name
        )));
    }

    let foreign_keys = sqlx::query("SELECT * FROM pragma_foreign_key_list(?)")
        .bind(contract.name)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;
    if !foreign_keys.is_empty() {
        return Err(AppError::Internal(format!(
            "companion store table {} contains physical foreign keys",
            contract.name
        )));
    }
    Ok(())
}

async fn validate_named_indexes(pool: &SqlitePool) -> Result<(), AppError> {
    let actual_names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master \
         WHERE type = 'index' AND sql IS NOT NULL ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    let expected_names: std::collections::BTreeSet<&str> =
        BASELINE_INDEXES.iter().map(|index| index.name).collect();
    let actual_names_set: std::collections::BTreeSet<&str> =
        actual_names.iter().map(String::as_str).collect();
    if actual_names_set != expected_names {
        return Err(AppError::Internal(format!(
            "companion store index set is not the exact v3 baseline: expected {expected_names:?}, found {actual_names_set:?}"
        )));
    }

    for contract in BASELINE_INDEXES {
        let row = sqlx::query(
            "SELECT tbl_name, sql FROM sqlite_master WHERE type = 'index' AND name = ?",
        )
        .bind(contract.name)
        .fetch_one(pool)
        .await
        .map_err(db_err)?;
        let table: String = row.get("tbl_name");
        let sql: String = row.try_get("sql").map_err(db_err)?;
        let list_row = sqlx::query(
            "SELECT \"unique\" AS is_unique, origin, partial \
             FROM pragma_index_list(?) WHERE name = ?",
        )
        .bind(contract.table)
        .bind(contract.name)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?
        .ok_or_else(|| {
            AppError::Internal(format!(
                "companion store table {} is missing index {}",
                contract.table, contract.name
            ))
        })?;
        let unique = list_row.get::<i64, _>("is_unique") != 0;
        let origin: String = list_row.get("origin");
        let partial = list_row.get::<i64, _>("partial") != 0;
        let actual_columns = index_key_columns(pool, contract.name).await?;
        let expected_columns: Vec<(String, bool)> = contract
            .columns
            .iter()
            .map(|column| (column.name.to_owned(), column.descending))
            .collect();
        if table != contract.table
            || unique != contract.unique
            || partial != contract.partial
            || origin != "c"
            || actual_columns != expected_columns
        {
            return Err(AppError::Internal(format!(
                "companion store index {} does not match the v3 baseline: \
                 table={table}, unique={unique}, partial={partial}, origin={origin}, columns={actual_columns:?}",
                contract.name
            )));
        }
        if let Some(fragment) = contract.where_fragment
            && !normalized_schema_sql(&sql).contains(fragment)
        {
            return Err(AppError::Internal(format!(
                "companion store partial index {} is missing predicate {fragment}",
                contract.name
            )));
        }
    }
    Ok(())
}

async fn validate_fts_contract(pool: &SqlitePool) -> Result<(), AppError> {
    let sql: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?",
    )
    .bind(FTS_TABLE)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?;
    let sql = sql.ok_or_else(|| {
        AppError::Internal(format!("companion store missing FTS index table {FTS_TABLE}"))
    })?;
    let normalized = normalized_schema_sql(&sql);
    for fragment in [
        "usingfts5",
        "content='companion_memories'",
        "content_rowid='id'",
        "tokenize='trigram'",
    ] {
        if !normalized.contains(fragment) {
            return Err(AppError::Internal(format!(
                "companion store FTS table {FTS_TABLE} is missing required definition fragment {fragment}"
            )));
        }
    }
    Ok(())
}

async fn validate_baseline_schema(pool: &SqlitePool) -> Result<(), AppError> {
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .map_err(db_err)?;
    if version != STORE_VERSION {
        return Err(AppError::Internal(format!(
            "companion store contract version mismatch: expected {STORE_VERSION}, found {version}"
        )));
    }
    for table in BASELINE_TABLES {
        let sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind(table.name)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?
        .ok_or_else(|| AppError::Internal(format!("companion store missing table {}", table.name)))?;
        validate_table_contract(pool, table, &sql).await?;
    }
    validate_fts_contract(pool).await?;
    validate_named_indexes(pool).await?;
    let trigger_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'")
            .fetch_one(pool)
            .await
            .map_err(db_err)?;
    if trigger_count != 0 {
        return Err(AppError::Internal(
            "companion store v3 must not contain physical triggers".into(),
        ));
    }
    let user_tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    let expected: std::collections::BTreeSet<&str> = BASELINE_TABLES
        .iter()
        .map(|table| table.name)
        .chain(std::iter::once(FTS_TABLE))
        .chain(FTS_SHADOW_TABLES.iter().copied())
        .collect();
    let actual: std::collections::BTreeSet<&str> =
        user_tables.iter().map(String::as_str).collect();
    if actual != expected {
        return Err(AppError::Internal(format!(
            "companion store table set is not the exact v3 baseline: expected {expected:?}, found {actual:?}"
        )));
    }
    Ok(())
}

async fn create_baseline_schema(pool: &SqlitePool) -> Result<(), AppError> {
    sqlx::raw_sql(SCHEMA).execute(pool).await.map_err(db_err)?;
    sqlx::raw_sql(FTS_SCHEMA).execute(pool).await.map_err(db_err)?;
    sqlx::raw_sql(&format!("PRAGMA user_version = {STORE_VERSION}"))
        .execute(pool)
        .await
        .map_err(db_err)?;
    validate_baseline_schema(pool).await
}

/// Idempotent in-place upgrade of an existing v3 store to the current v3
/// baseline: add the nullable embedding columns, collapse the retired
/// `(scope_kind, scope_companion_id)` owner pair into one nullable
/// `companion_id`, add the external-content FTS5 index when missing, rebuild the
/// index when its row count desyncs from the main table, remove the retired
/// learn-run/suggestion tables, and re-home the vestigial unowned memories and
/// skills onto `row_owner`. User memories and skills are preserved verbatim —
/// nothing is ever deleted or duplicated here.
/// Non-v3 stores are left untouched for `validate_baseline_schema` to reject.
async fn upgrade_schema_in_place(
    pool: &SqlitePool,
    row_owner: Option<&str>,
) -> Result<(), AppError> {
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .map_err(db_err)?;
    if version != STORE_VERSION {
        return Ok(());
    }
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_xinfo('companion_memories')")
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
    if columns.is_empty() {
        // No companion_memories table at all — not a v3 layout; leave it to validation.
        return Ok(());
    }
    for (column, definition) in [
        ("embedding", "ALTER TABLE companion_memories ADD COLUMN embedding BLOB"),
        ("embedding_model", "ALTER TABLE companion_memories ADD COLUMN embedding_model TEXT"),
    ] {
        if !columns.iter().any(|name| name == column) {
            sqlx::raw_sql(definition).execute(pool).await.map_err(db_err)?;
        }
    }
    // Must run before the FTS bookkeeping below: it rebuilds companion_memories,
    // and the forced index rebuild is how the index is re-anchored afterwards.
    let owner_columns_collapsed = collapse_owner_columns(pool).await?;

    let fts_sql: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?",
    )
    .bind(FTS_TABLE)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?;
    let mut rebuild = match fts_sql {
        None => {
            sqlx::raw_sql(FTS_SCHEMA).execute(pool).await.map_err(db_err)?;
            true
        }
        Some(_) => false,
    };
    // The owner-column collapse re-creates the external-content table the index
    // points at. The copy preserves every `id` verbatim, so the index is still
    // correct — this rebuild is the belt to that braces, and cheap: it only ever
    // runs on the single boot that performs the collapse.
    rebuild = rebuild || owner_columns_collapsed;
    if !rebuild {
        let main_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM companion_memories")
            .fetch_one(pool)
            .await
            .map_err(db_err)?;
        // count(*) on an external-content fts5 table mirrors the CONTENT table,
        // not the index — the docsize shadow table (one row per indexed doc) is
        // the authoritative index row count.
        let fts_rows: i64 =
            sqlx::query_scalar(&format!("SELECT count(*) FROM {FTS_TABLE}_docsize"))
                .fetch_one(pool)
                .await
                .map_err(db_err)?;
        rebuild = main_rows != fts_rows;
    }
    if rebuild {
        // 'rebuild' repopulates the external-content index from the main table
        // in one statement — idempotent by construction.
        sqlx::query(&format!("INSERT INTO {FTS_TABLE}({FTS_TABLE}) VALUES('rebuild')"))
            .execute(pool)
            .await
            .map_err(db_err)?;
    }
    // Learning cadence and event progress live in companion_state; historical
    // run rows were presentation-only and are intentionally retired.
    sqlx::raw_sql("DROP TABLE IF EXISTS companion_learn_runs")
        .execute(pool)
        .await
        .map_err(db_err)?;
    // The 建议 (suggestion) feature was removed end to end; its card table is
    // presentation-only state with no remaining reader or writer.
    sqlx::raw_sql("DROP TABLE IF EXISTS companion_suggestions")
        .execute(pool)
        .await
        .map_err(db_err)?;
    backfill_memory_owner(pool, row_owner).await?;
    backfill_skill_owner(pool, row_owner).await?;
    Ok(())
}

/// One-time, idempotent collapse of the retired two-column owner encoding
/// (`scope_kind TEXT NOT NULL` + `scope_companion_id TEXT`, paired by a table
/// CHECK into `('user', NULL)` = unowned and `('companion', id)` = owned) into
/// ONE nullable `companion_id`. Returns true when it actually rebuilt anything.
///
/// `scope_kind` was fully determined by whether an owner was present, so the pair
/// could only ever disagree, never inform: `companion_id IS NULL` says exactly
/// what `('user', NULL)` said, once instead of twice. Collapsing it also deletes
/// the two paired CHECK constraints, the `scope_kind`-keyed partial indexes and
/// the "is this pair one of the two legal states?" validation on every read.
///
/// This is a full table rebuild, not an `ALTER TABLE ... DROP COLUMN`: SQLite
/// refuses to drop a column named in a table CHECK. It is unconditional (no "is
/// every row owned yet?" precondition) precisely because the target column stays
/// NULLABLE — a zero-companion install, where nothing can be owned, is still
/// representable.
///
/// Non-negotiables:
/// - Every row survives, with its owner, lifecycle, pin, strength and timestamps
///   verbatim. `SELECT`ing `scope_companion_id` straight into `companion_id` is
///   exactly the old semantics (the CHECK made the id NULL iff the kind was
///   `'user'`), and it never *loses* an owner even if some historical build wrote
///   a pair the CHECK would have rejected.
/// - `id` is preserved verbatim, because `companion_memories_fts` is
///   external-content FTS5 with `content_rowid='id'` — renumbering the rows would
///   silently point every indexed document at the wrong memory. The caller also
///   forces an index rebuild after this returns true.
/// - The new tables are created by replaying [`SCHEMA`] (every statement is
///   `IF NOT EXISTS`), so the rebuilt shape cannot drift from the baseline the
///   contract validation demands, and the named indexes — dropped along with
///   their table — come back with it.
/// - It all happens in ONE transaction, so an interrupted upgrade leaves the
///   pre-collapse store fully intact.
async fn collapse_owner_columns(pool: &SqlitePool) -> Result<bool, AppError> {
    /// Column mapping of one table's rebuild: the fresh column list, and the
    /// matching `SELECT` list against the legacy table.
    struct Rebuild {
        table: &'static str,
        columns: String,
        legacy_select: String,
    }

    async fn legacy_columns(pool: &SqlitePool, table: &str) -> Result<Vec<String>, AppError> {
        sqlx::query_scalar(&format!("SELECT name FROM pragma_table_xinfo('{table}')"))
            .fetch_all(pool)
            .await
            .map_err(db_err)
    }

    let mut rebuilds = Vec::new();
    // `{owner}` is the one column that differs between the two shapes; every
    // other column is copied position-for-position, by name.
    for (table, template) in [
        (
            "companion_memories",
            "id, memory_id, kind, content, tags, importance, strength, pinned, source, \
             status, created_at, updated_at, last_reinforced_at, {owner}, embedding, \
             embedding_model",
        ),
        (
            "companion_skills",
            "id, companion_skill_id, skill_name, {owner}, status, source, confidence, \
             provenance_event_ids, strength, version, skill_pattern_id, usage_count, \
             last_used_at, created_at, updated_at, signature",
        ),
    ] {
        let present = legacy_columns(pool, table).await?;
        if !present.iter().any(|name| name == "scope_kind" || name == "scope_companion_id") {
            continue; // already collapsed
        }
        // A store carrying `scope_kind` without `scope_companion_id` is not a
        // shape this codebase ever wrote, but reading a column that is not there
        // would fail the boot of that install for good; treat it as unowned.
        let owner = if present.iter().any(|name| name == "scope_companion_id") {
            "scope_companion_id"
        } else {
            "NULL"
        };
        rebuilds.push(Rebuild {
            table,
            columns: template.replace("{owner}", "companion_id"),
            // Aliased, so the staging table already carries the NEW column names
            // and the copy back is one symmetric column list.
            legacy_select: template.replace("{owner}", &format!("{owner} AS companion_id")),
        });
    }
    if rebuilds.is_empty() {
        return Ok(false);
    }

    let mut tx = pool.begin().await.map_err(db_err)?;
    for rebuild in &rebuilds {
        sqlx::query(&format!(
            "CREATE TEMP TABLE {}_owner_collapse AS SELECT {} FROM {}",
            rebuild.table, rebuild.legacy_select, rebuild.table
        ))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        // Drops the table's indexes with it; SCHEMA below re-creates both.
        sqlx::query(&format!("DROP TABLE {}", rebuild.table))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    sqlx::raw_sql(SCHEMA).execute(&mut *tx).await.map_err(db_err)?;
    for rebuild in &rebuilds {
        sqlx::query(&format!(
            "INSERT INTO {table}({columns}) SELECT {columns} FROM {table}_owner_collapse",
            table = rebuild.table,
            columns = rebuild.columns
        ))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        sqlx::query(&format!("DROP TABLE {}_owner_collapse", rebuild.table))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    tracing::info!(
        tables = ?rebuilds.iter().map(|rebuild| rebuild.table).collect::<Vec<_>>(),
        "collapsed the retired (scope_kind, scope_companion_id) pair into one nullable companion_id"
    );
    Ok(true)
}

/// One-time, idempotent re-homing of the vestigial unowned memories
/// (`companion_id IS NULL`) onto `row_owner`. 共享记忆作为产品概念已删除，而
/// **历史上每一条 learner 写出来的记忆都是共享的**，所以这些行是主人积累的
/// 大多数记忆：它们必须原地改主人，一条都不能删。
///
/// RE-HOME, never duplicate: copying each row to every companion would multiply
/// the row count by the roster size, break `memory_id` stability (rotting export
/// bundles and external references), and let every copy decay and archive
/// independently until the same fact silently diverges per companion.
///
/// An empty roster has no legal owner: the rows are left exactly as they are and
/// this simply takes effect the next time the store opens with a companion
/// present. That is also why `companion_id` stays NULLABLE — a zero-companion
/// install is a supported state.
async fn backfill_memory_owner(
    pool: &SqlitePool,
    row_owner: Option<&str>,
) -> Result<(), AppError> {
    let Some(owner) = row_owner else {
        return Ok(());
    };
    // A malformed owner would write an unreadable row and hard-fail the very
    // next boot on `validate_companion_references`; fail here instead.
    validate_companion_id(owner, "memory backfill companion_id")?;
    let affected = sqlx::query(
        "UPDATE companion_memories
            SET companion_id = ?
          WHERE companion_id IS NULL",
    )
    .bind(owner)
    .execute(pool)
    .await
    .map_err(db_err)?
    .rows_affected();
    if affected > 0 {
        tracing::info!(
            memories = affected,
            companion_id = %owner,
            "re-homed shared companion memories onto their owner"
        );
    }
    Ok(())
}

/// One-time, idempotent re-homing of the vestigial unowned skills
/// (`companion_id IS NULL`) onto `row_owner` — the exact analogue of
/// [`backfill_memory_owner`], for the same reason: 共享技能 is gone as a product
/// concept, a skill belongs to one companion, and the owner-scoped list would
/// otherwise hide these rows from every companion forever.
///
/// RE-HOME, never duplicate or delete: a skill row is the metadata half of one
/// on-disk `SKILL.md`, so copying it per companion would point several rows at
/// one file and dropping it would orphan that file (and hard-fail the boot
/// inventory audit). The file itself is moved into the owner's tree by
/// [`crate::skill_io::rehome_unowned_skill_dirs`], which reconciles from the
/// filesystem and therefore also repairs a crash in between.
///
/// A name COLLISION is skipped rather than forced: `idx_companion_skills_private_owner_name`
/// makes `(owner, skill_name)` unique, so re-homing an unowned `foo` onto a
/// companion that already has its own `foo` would raise a UNIQUE violation
/// inside `upgrade_schema_in_place` — i.e. fail the boot of every install that
/// has such a pair, permanently. The colliding row keeps its legacy shape (still
/// on disk, still exportable, nothing lost) and is logged.
async fn backfill_skill_owner(
    pool: &SqlitePool,
    row_owner: Option<&str>,
) -> Result<(), AppError> {
    let Some(owner) = row_owner else {
        return Ok(());
    };
    // A malformed owner would write an unreadable row and hard-fail the very
    // next boot on `validate_companion_references`; fail here instead.
    validate_companion_id(owner, "skill backfill companion_id")?;
    let affected = sqlx::query(
        "UPDATE companion_skills
            SET companion_id = ?
          WHERE companion_id IS NULL
            AND NOT EXISTS (
              SELECT 1 FROM companion_skills owned
               WHERE owned.companion_id = ?
                 AND owned.skill_name = companion_skills.skill_name
            )",
    )
    .bind(owner)
    .bind(owner)
    .execute(pool)
    .await
    .map_err(db_err)?
    .rows_affected();
    if affected > 0 {
        tracing::info!(
            skills = affected,
            companion_id = %owner,
            "re-homed shared companion skills onto their owner"
        );
    }
    let stranded: Vec<String> =
        sqlx::query_scalar("SELECT skill_name FROM companion_skills WHERE companion_id IS NULL")
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
    if !stranded.is_empty() {
        tracing::warn!(
            skills = ?stranded,
            companion_id = %owner,
            "kept these legacy shared skills unowned: the owner already has a skill of the same name"
        );
    }
    Ok(())
}

pub(crate) fn row_to_memory(row: &sqlx::sqlite::SqliteRow) -> Result<CompanionMemory, AppError> {
    let tags: String = row.get("tags");
    let memory_id: String = row.get("memory_id");
    CompanionMemoryId::try_from(memory_id.as_str())
        .map_err(|error| invalid_disk_id("memory id", &memory_id, error))?;
    let parsed_tags = serde_json::from_str(&tags).map_err(|error| {
        AppError::Internal(format!(
            "companion store memory '{}' contains invalid tags JSON: {error}",
            memory_id
        ))
    })?;
    let companion_id: Option<String> = row.try_get("companion_id").map_err(db_err)?;
    if let Some(owner) = companion_id.as_deref() {
        CompanionId::try_from(owner)
            .map_err(|error| invalid_disk_id("memory companion id", owner, error))?;
    }
    Ok(CompanionMemory {
        memory_id,
        kind: row.get("kind"),
        content: row.get("content"),
        tags: parsed_tags,
        importance: row.get("importance"),
        strength: row.get("strength"),
        pinned: row.get::<i64, _>("pinned") != 0,
        source: row.get("source"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        last_reinforced_at: row.get("last_reinforced_at"),
        companion_id,
    })
}

/// Local-time day key (`YYYYMMDD`) for a ms-epoch timestamp — the partition key
/// for session-window digests. Uses the local timezone to stay consistent with
/// the event collector's `events/YYYYMMDD.jsonl` day boundaries.
pub fn local_day(ts_ms: TimestampMs) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts_ms)
        .single()
        .map(|d| d.format("%Y%m%d").to_string())
        .unwrap_or_else(|| "00000000".into())
}

fn row_to_window(row: &sqlx::sqlite::SqliteRow) -> Result<SessionWindow, AppError> {
    let session_window_id: String = row.get("session_window_id");
    CompanionSessionWindowId::try_from(session_window_id.as_str())
        .map_err(|error| invalid_disk_id("session-window id", &session_window_id, error))?;
    let companion_id: String = row.get("companion_id");
    CompanionId::try_from(companion_id.as_str())
        .map_err(|error| invalid_disk_id("session-window companion id", &companion_id, error))?;
    let conversation_id: String = row.get("conversation_id");
    ConversationId::try_from(conversation_id.as_str())
        .map_err(|error| invalid_disk_id("session-window conversation id", &conversation_id, error))?;
    let highlights: Option<String> = row.try_get("highlights").map_err(db_err)?;
    if let Some(raw) = highlights.as_deref() {
        serde_json::from_str::<serde_json::Value>(raw).map_err(|error| {
            AppError::Internal(format!(
                "companion store session window '{}' contains invalid highlights JSON: {error}",
                session_window_id
            ))
        })?;
    }
    Ok(SessionWindow {
        session_window_id,
        companion_id,
        conversation_id,
        session_day: row.get("session_day"),
        started_at: row.get("started_at"),
        last_activity_at: row.get("last_activity_at"),
        closed_at: row.try_get("closed_at").map_err(db_err)?,
        status: row.get("status"),
        message_count: row.get("message_count"),
        boundary_ts: row.get("boundary_ts"),
        digest: row.try_get("digest").map_err(db_err)?,
        highlights,
        token_estimate: row.get("token_estimate"),
    })
}

fn row_to_companion_thread(row: &sqlx::sqlite::SqliteRow) -> Result<CompanionThread, AppError> {
    let conversation_id: String = row.get("conversation_id");
    ConversationId::try_from(conversation_id.as_str())
        .map_err(|error| invalid_disk_id("thread conversation id", &conversation_id, error))?;
    let companion_id: String = row.get("companion_id");
    CompanionId::try_from(companion_id.as_str())
        .map_err(|error| invalid_disk_id("thread companion id", &companion_id, error))?;
    Ok(CompanionThread {
        conversation_id,
        companion_id,
        title: row.get("title"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

impl CompanionStore {
    /// Crate-internal pool handle for the store's submodule layers
    /// (`memory_search` runs its SQL on the same connection pool).
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Open (or create) the v3 baseline `{companion_dir}/memory.db`.
    ///
    /// `row_owner` is the companion that the one-time re-homing migrations assign
    /// to every vestigial unowned (`companion_id IS NULL`) memory and skill — resolve it
    /// from the live roster BEFORE opening the store (see
    /// [`CompanionRegistry::resolve_row_owner`](crate::registry::CompanionRegistry::resolve_row_owner)).
    /// `None` (empty roster) leaves those rows untouched for a later open.
    pub async fn open(companion_dir: &Path, row_owner: Option<&str>) -> Result<Self, AppError> {
        std::fs::create_dir_all(companion_dir)
            .map_err(|e| AppError::Internal(format!("create companion dir: {e}")))?;
        let database_path = companion_dir.join("memory.db");
        let database_exists = match std::fs::symlink_metadata(&database_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(AppError::Internal(format!(
                    "companion store path is not a regular file: {}",
                    database_path.display()
                )));
            }
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "inspect companion store {}: {error}",
                    database_path.display()
                )));
            }
        };
        let opts = SqliteConnectOptions::new()
            .filename(&database_path)
            .create_if_missing(!database_exists)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));
        {
            let bootstrap = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts.clone())
                .await
                .map_err(db_err)?;
            let init = if database_exists {
                // In-place upgrade first (idempotent ALTER/CREATE IF; preserves
                // rows), then the strict contract check.
                match upgrade_schema_in_place(&bootstrap, row_owner).await {
                    Ok(()) => validate_baseline_schema(&bootstrap).await,
                    Err(error) => Err(error),
                }
            } else {
                create_baseline_schema(&bootstrap).await
            };
            bootstrap.close().await;
            init?;
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(opts)
            .await
            .map_err(db_err)?;
        validate_baseline_schema(&pool).await?;
        Ok(Self { pool })
    }

    /// In-memory store for tests. The db lives inside the pool's single
    /// connection, so (unlike `open`) schema bootstrap must run on that same
    /// pool — a separate bootstrap connection would see a different db.
    pub async fn open_memory() -> Result<Self, AppError> {
        let opts = SqliteConnectOptions::new().in_memory(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(db_err)?;
        create_baseline_schema(&pool).await?;
        Ok(Self { pool })
    }

    // ----- state kv -----

    pub async fn get_state(&self, key: &str) -> Result<Option<String>, AppError> {
        let row = sqlx::query("SELECT value FROM companion_state WHERE state_key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| r.get("value")))
    }

    pub async fn set_state(&self, key: &str, value: &str) -> Result<(), AppError> {
        sqlx::query("INSERT INTO companion_state(state_key, value) VALUES(?, ?) ON CONFLICT(state_key) DO UPDATE SET value = excluded.value")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn get_state_i64(&self, key: &str) -> Result<i64, AppError> {
        match self.get_state(key).await? {
            None => Ok(0),
            Some(value) => value.parse().map_err(|error| {
                AppError::Internal(format!(
                    "companion state {key:?} contains invalid integer {value:?}: {error}"
                ))
            }),
        }
    }

    // ----- per-companion state kv (companion_runtime_state) -----

    pub async fn get_companion_state(&self, companion_id: &str, key: &str) -> Result<Option<String>, AppError> {
        validate_companion_id(companion_id, "companion state companion_id")?;
        let row = sqlx::query("SELECT value FROM companion_runtime_state WHERE companion_id = ? AND state_key = ?")
            .bind(companion_id)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| r.get("value")))
    }

    pub async fn set_companion_state(&self, companion_id: &str, key: &str, value: &str) -> Result<(), AppError> {
        validate_companion_id(companion_id, "companion state companion_id")?;
        sqlx::query(
            "INSERT INTO companion_runtime_state(companion_id, state_key, value) VALUES(?, ?, ?)
             ON CONFLICT(companion_id, state_key) DO UPDATE SET value = excluded.value",
        )
        .bind(companion_id)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub async fn delete_companion_state(&self, companion_id: &str, key: &str) -> Result<(), AppError> {
        validate_companion_id(companion_id, "companion state companion_id")?;
        sqlx::query("DELETE FROM companion_runtime_state WHERE companion_id = ? AND state_key = ?")
            .bind(companion_id)
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn get_companion_state_i64(&self, companion_id: &str, key: &str) -> Result<i64, AppError> {
        match self.get_companion_state(companion_id, key).await? {
            None => Ok(0),
            Some(value) => value.parse().map_err(|error| {
                AppError::Internal(format!(
                    "companion state ({companion_id}, {key:?}) contains invalid integer {value:?}: {error}"
                ))
            }),
        }
    }

    /// Atomic per-companion XP increment (single upsert, key fixed to 'xp').
    /// Returns the companion's new total.
    pub async fn add_companion_xp(&self, companion_id: &str, delta: i64) -> Result<i64, AppError> {
        validate_companion_id(companion_id, "companion xp companion_id")?;
        let row = sqlx::query(
            "INSERT INTO companion_runtime_state(companion_id, state_key, value) VALUES(?, 'xp', ?)
             ON CONFLICT(companion_id, state_key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + ? AS TEXT)
             RETURNING CAST(value AS INTEGER) AS xp",
        )
        .bind(companion_id)
        .bind(delta.to_string())
        .bind(delta)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.get("xp"))
    }

    /// Write `(companion_id, key)` only if that row does not exist yet.
    ///
    /// `INSERT OR IGNORE` against `UNIQUE(companion_id, state_key)` is what makes
    /// the per-companion state migrations idempotent without any extra marker:
    /// re-running them can never clobber a value the companion has since moved on
    /// from (a learn cursor that has advanced, a mood the last run wrote).
    /// Returns true when a row was actually created.
    pub async fn seed_companion_state(
        &self,
        companion_id: &str,
        key: &str,
        value: &str,
    ) -> Result<bool, AppError> {
        validate_companion_id(companion_id, "companion state companion_id")?;
        let affected = sqlx::query(
            "INSERT OR IGNORE INTO companion_runtime_state(companion_id, state_key, value)
             VALUES(?, ?, ?)",
        )
        .bind(companion_id)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(db_err)?
        .rows_affected();
        Ok(affected > 0)
    }

    /// Boot migration for the runtime state that became per-companion in 2026-08
    /// together with the 学习 / 进化 settings themselves.
    ///
    /// Each key listed in [`MIGRATED_GLOBAL_STATE_KEYS`] is copied from its single
    /// `companion_state` row onto every companion that has no row of its own, and
    /// the global row is then DELETED. Re-running this is a no-op either way, and
    /// the per-companion write is `INSERT OR IGNORE`, so a value a companion has
    /// since moved on from (an advanced cursor, a mood the last run wrote) can
    /// never be clobbered.
    ///
    /// The cursors are the load-bearing part. Leaving a companion at the absent-row
    /// default of 0 would make its first run re-read the entire retained event
    /// history — N companions re-distilling weeks of events at once means duplicate
    /// memories and a large, unexpected LLM bill. `mood` is here so nobody's
    /// companion visibly resets to "content" on upgrade, and the two `last_*_ts`
    /// stamps so the whole roster does not fire a run simultaneously on the first
    /// tick after the update.
    ///
    /// An EMPTY roster deletes nothing: with nobody to copy the values onto, the
    /// global rows are still the only place the owner's cursors exist, and the
    /// first companion created later must inherit them rather than restart from
    /// the oldest retained event.
    pub async fn seed_companion_state_from_global(
        &self,
        companion_ids: &[String],
    ) -> Result<usize, AppError> {
        let mut seeded = 0usize;
        for key in MIGRATED_GLOBAL_STATE_KEYS {
            let Some(value) = self.get_state(key).await? else {
                continue;
            };
            for companion_id in companion_ids {
                if self.seed_companion_state(companion_id, key, &value).await? {
                    seeded += 1;
                }
            }
        }
        if seeded > 0 {
            tracing::info!(
                rows = seeded,
                "seeded per-companion learn state from the retired install-wide keys"
            );
        }
        if !companion_ids.is_empty() {
            let dropped = self.delete_retired_global_state().await?;
            if dropped > 0 {
                tracing::info!(
                    rows = dropped,
                    "deleted the retired install-wide learn/evolve state rows"
                );
            }
        }
        Ok(seeded)
    }

    /// Delete the install-wide `companion_state` rows that became per-companion,
    /// once every companion has its own copy. Idempotent: the second call deletes
    /// nothing because the first one already did.
    ///
    /// These rows had exactly one remaining justification — "a rollback to an
    /// older build still finds them" — and it is false: the same in-place upgrade
    /// drops the retired `companion_suggestions` table, and an older build
    /// validates an exact table set, so it cannot open this store at all. Unread
    /// rows that no longer answer any question are just data waiting to be
    /// mistaken for the truth.
    async fn delete_retired_global_state(&self) -> Result<u64, AppError> {
        let placeholders = MIGRATED_GLOBAL_STATE_KEYS
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("DELETE FROM companion_state WHERE state_key IN ({placeholders})");
        let mut query = sqlx::query(&sql);
        for key in MIGRATED_GLOBAL_STATE_KEYS {
            query = query.bind(*key);
        }
        Ok(query.execute(&self.pool).await.map_err(db_err)?.rows_affected())
    }

    /// Remove every per-companion row owned by `companion_id` (runtime kv + companion
    /// thread registrations + private memories/skills/session windows) in one
    /// transaction. Used by companion deletion.
    pub async fn delete_companion_rows(&self, companion_id: &str) -> Result<(), AppError> {
        validate_companion_id(companion_id, "deleted companion_id")?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        // De-index the private memories about to be deleted (FTS 'delete' needs
        // the old rowid+content, so this must run before the DELETE below).
        let indexed = sqlx::query(
            "SELECT id, content FROM companion_memories WHERE companion_id = ?",
        )
        .bind(companion_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_err)?;
        for row in &indexed {
            fts_index_delete(&mut *tx, row.get("id"), row.get("content")).await?;
        }
        for sql in [
            "DELETE FROM companion_memories WHERE companion_id = ?",
            "DELETE FROM companion_skills WHERE companion_id = ?",
            "DELETE FROM companion_session_windows WHERE companion_id = ?",
            "DELETE FROM companion_runtime_state WHERE companion_id = ?",
            "DELETE FROM companion_threads WHERE companion_id = ?",
        ] {
            sqlx::query(sql)
                .bind(companion_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Audit all logical companion references after the roster is loaded.
    /// Physical foreign keys are intentionally absent in v3, so startup must
    /// reject rows whose parent companion no longer exists instead of exposing
    /// partially orphaned side-store state.
    pub async fn validate_companion_references(
        &self,
        live_companion_ids: &std::collections::HashSet<String>,
    ) -> Result<(), AppError> {
        let references = [
            // The two owner columns are NULLABLE (an unowned row the boot
            // migration has not re-homed yet), which the IS NOT NULL below skips;
            // the other three are NOT NULL.
            ("companion_memories", "companion_id"),
            ("companion_skills", "companion_id"),
            ("companion_runtime_state", "companion_id"),
            ("companion_threads", "companion_id"),
            ("companion_session_windows", "companion_id"),
        ];
        for (table, column) in references {
            let sql = format!(
                "SELECT DISTINCT {column} FROM {table} WHERE {column} IS NOT NULL"
            );
            let values: Vec<String> = sqlx::query_scalar(&sql)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;
            for value in values {
                CompanionId::try_from(value.as_str())
                    .map_err(|error| invalid_disk_id("logical companion reference", &value, error))?;
                if !live_companion_ids.contains(&value) {
                    return Err(AppError::Internal(format!(
                        "companion store table {table} contains orphaned companion reference {value:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    // ----- memories -----

    /// Test-only insert of an UNOWNED (`companion_id = NULL`) memory — the shape
    /// legacy installs are full of. Deliberately `cfg(test)`: production has no
    /// ownerless write path any more, every writer resolves an owner first.
    #[cfg(test)]
    pub(crate) async fn insert_memory(
        &self,
        kind: &str,
        content: &str,
        tags: &[String],
        importance: f64,
        source: &str,
    ) -> Result<CompanionMemory, AppError> {
        self.insert_memory_scoped(kind, content, tags, importance, source, None).await
    }

    /// Insert a memory owned by `owner`. Every production caller resolves the
    /// owner first; `None` (an unowned row) exists only for legacy fixtures.
    pub async fn insert_memory_scoped(
        &self,
        kind: &str,
        content: &str,
        tags: &[String],
        importance: f64,
        source: &str,
        owner: Option<&str>,
    ) -> Result<CompanionMemory, AppError> {
        // Best-effort redaction before any secret reaches durable storage.
        // Covers both write paths (manual save_memory and the distill learner),
        // which both funnel through here.
        let content = nomi_redact::redact_secrets(content);
        let now = now_ms();
        validate_row_owner(owner)?;
        let mem = CompanionMemory {
            memory_id: CompanionMemoryId::new().into_string(),
            kind: kind.to_owned(),
            content: content.into_owned(),
            tags: tags.to_vec(),
            importance: importance.clamp(0.0, 1.0),
            strength: importance.clamp(0.0, 1.0),
            pinned: false,
            source: source.to_owned(),
            status: "active".into(),
            created_at: now,
            updated_at: now,
            last_reinforced_at: now,
            companion_id: owner.map(str::to_owned),
        };
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let row = sqlx::query(
            "INSERT INTO companion_memories(memory_id, kind, content, tags, importance, strength, pinned, source, status, created_at, updated_at, last_reinforced_at, companion_id)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)
             RETURNING id",
        )
        .bind(&mem.memory_id)
        .bind(&mem.kind)
        .bind(&mem.content)
        .bind(
            serde_json::to_string(&mem.tags)
                .map_err(|error| AppError::Internal(format!("serialize memory tags: {error}")))?,
        )
        .bind(mem.importance)
        .bind(mem.strength)
        .bind(mem.pinned as i64)
        .bind(&mem.source)
        .bind(&mem.status)
        .bind(mem.created_at)
        .bind(mem.updated_at)
        .bind(mem.last_reinforced_at)
        .bind(&mem.companion_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        fts_index_insert(&mut *tx, row.get("id"), &mem.content).await?;
        tx.commit().await.map_err(db_err)?;
        Ok(mem)
    }

    /// Crude dedup guard for ONE owner: an active memory this companion can
    /// read (its own, or a not-yet-re-homed unowned row) of the same kind whose
    /// normalized content equals the candidate, or contains it (either
    /// direction) when the two are close in length. The length-ratio guard
    /// stops a short memory ("主人用 Rust") from swallowing a longer, genuinely
    /// distinct one that merely embeds the same phrase.
    ///
    /// The owner predicate is load-bearing now that every write is owned:
    /// without it companion B's genuinely new memory would be silently
    /// swallowed because companion A happens to hold a similar one.
    pub async fn find_similar_active(
        &self,
        kind: &str,
        content: &str,
        owner: &str,
    ) -> Result<Option<String>, AppError> {
        validate_companion_id(owner, "memory dedup companion_id")?;
        let rows = sqlx::query(&format!(
            "SELECT memory_id, content FROM companion_memories WHERE kind = ? AND status = 'active'{MEMORY_VISIBILITY_PREDICATE}"
        ))
        .bind(kind)
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        for row in rows {
            let existing: String = row.get("content");
            if memory_contents_similar(content, &existing) {
                let id: String = row.get("memory_id");
                CompanionMemoryId::try_from(id.as_str())
                    .map_err(|error| invalid_disk_id("memory id", &id, error))?;
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    pub async fn list_memories(&self, filter: &MemoryFilter) -> Result<Vec<CompanionMemory>, AppError> {
        if let Some(companion_id) = filter.companion_id.as_deref() {
            validate_companion_id(companion_id, "memory filter companion_id")?;
        }
        let mut sql = format!("SELECT * FROM companion_memories{}", memory_filter_clause(filter));
        sql.push_str(" ORDER BY pinned DESC, strength DESC, updated_at DESC LIMIT ? OFFSET ?");
        let mut query = sqlx::query(&sql);
        if let Some(kind) = &filter.kind {
            query = query.bind(kind);
        }
        if let Some(q) = &filter.q {
            query = query.bind(format!("%{q}%"));
        }
        if let Some(status) = &filter.status {
            query = query.bind(status);
        }
        if let Some(cid) = &filter.companion_id {
            query = query.bind(cid);
        }
        let limit = if filter.limit <= 0 { 100 } else { filter.limit.min(500) };
        query = query.bind(limit).bind(filter.offset.max(0));
        let rows = query.fetch_all(&self.pool).await.map_err(db_err)?;
        rows.iter().map(row_to_memory).collect()
    }

    /// One page of memories with an explicit sort order (the REST list endpoint's
    /// `sort` param on the non-FTS path; the FTS path ranks in `search_memories`).
    pub async fn list_memory_page_sorted(&self, filter: &MemoryFilter, sort: MemoryListSort) -> Result<MemoryPage, AppError> {
        if let Some(companion_id) = filter.companion_id.as_deref() {
            validate_companion_id(companion_id, "memory filter companion_id")?;
        }
        let order_by = match sort {
            MemoryListSort::Default => " ORDER BY pinned DESC, strength DESC, updated_at DESC",
            MemoryListSort::Time => " ORDER BY updated_at DESC",
            MemoryListSort::Importance => " ORDER BY pinned DESC, importance DESC, strength DESC, updated_at DESC",
        };
        let mut items_sql = format!("SELECT * FROM companion_memories{}", memory_filter_clause(filter));
        items_sql.push_str(order_by);
        items_sql.push_str(" LIMIT ? OFFSET ?");
        let mut items_query = sqlx::query(&items_sql);
        if let Some(kind) = &filter.kind {
            items_query = items_query.bind(kind);
        }
        if let Some(q) = &filter.q {
            items_query = items_query.bind(format!("%{q}%"));
        }
        if let Some(status) = &filter.status {
            items_query = items_query.bind(status);
        }
        if let Some(cid) = &filter.companion_id {
            items_query = items_query.bind(cid);
        }
        let limit = if filter.limit <= 0 { 100 } else { filter.limit.min(500) };
        let rows = items_query
            .bind(limit)
            .bind(filter.offset.max(0))
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;

        let count_sql = format!("SELECT COUNT(*) AS n FROM companion_memories{}", memory_filter_clause(filter));
        let mut count_query = sqlx::query(&count_sql);
        if let Some(kind) = &filter.kind {
            count_query = count_query.bind(kind);
        }
        if let Some(q) = &filter.q {
            count_query = count_query.bind(format!("%{q}%"));
        }
        if let Some(status) = &filter.status {
            count_query = count_query.bind(status);
        }
        if let Some(cid) = &filter.companion_id {
            count_query = count_query.bind(cid);
        }
        let total = count_query.fetch_one(&self.pool).await.map_err(db_err)?.get("n");

        Ok(MemoryPage {
            items: rows.iter().map(row_to_memory).collect::<Result<Vec<_>, _>>()?,
            total,
        })
    }

    /// Count memories in one lifecycle state. `companion_id` scopes the count to
    /// what that companion can read (its own + not-yet-re-homed unowned rows),
    /// which is exactly what its memory list shows; `None` counts every row and
    /// is only for the zero-companion aggregate snapshot.
    pub async fn count_memories(&self, status: &str, companion_id: Option<&str>) -> Result<i64, AppError> {
        let mut sql = String::from("SELECT COUNT(*) AS n FROM companion_memories WHERE status = ?");
        if let Some(companion_id) = companion_id {
            validate_companion_id(companion_id, "memory count companion_id")?;
            sql.push_str(MEMORY_VISIBILITY_PREDICATE);
        }
        let mut query = sqlx::query(&sql).bind(status);
        if let Some(companion_id) = companion_id {
            query = query.bind(companion_id);
        }
        let row = query.fetch_one(&self.pool).await.map_err(db_err)?;
        Ok(row.get("n"))
    }

    /// Edit content / pin / lifecycle of a row `actor` owns. Ownership is
    /// deliberately NOT editable: 共享记忆概念删除后，一条记忆的主人在写入时就定了，
    /// 改文字不能改归属 —— and per [`MemoryActor`] another companion's row is not
    /// addressable here at all.
    pub async fn update_memory(
        &self,
        memory_id: &str,
        content: Option<&str>,
        pinned: Option<bool>,
        status: Option<&str>,
        actor: &MemoryActor,
    ) -> Result<(), AppError> {
        CompanionMemoryId::try_from(memory_id)
            .map_err(|error| AppError::BadRequest(format!("invalid memory id: {error}")))?;
        actor.validate()?;
        // Validate + redact edited content symmetrically with insert_memory_scoped:
        // a user/agent edit must not bypass the empty-content guard or secret
        // redaction that the insert path enforces.
        let redacted: Option<String> = match content {
            Some(c) => {
                let trimmed = c.trim();
                if trimmed.is_empty() {
                    return Err(AppError::BadRequest("memory content is empty".into()));
                }
                Some(nomi_redact::redact_secrets(trimmed).into_owned())
            }
            None => None,
        };
        let now = now_ms();
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        // The ownership gate and the FTS bookkeeping are the same lookup: it
        // yields the rowid and the OLD indexed content (the fts5 'delete' command
        // needs it verbatim), and rejects a row this actor does not own.
        let located = locate_memory_for_mutation(&mut *tx, memory_id, actor).await?;
        let Some((rowid, old_content)) = located else {
            return Err(memory_not_found(memory_id));
        };
        let result = sqlx::query(
            "UPDATE companion_memories SET
               content = COALESCE(?, content),
               pinned = COALESCE(?, pinned),
               status = COALESCE(?, status),
               updated_at = ?
             WHERE memory_id = ?",
        )
        .bind(redacted.as_deref())
        .bind(pinned.map(|p| p as i64))
        .bind(status)
        .bind(now)
        .bind(memory_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(memory_not_found(memory_id));
        }
        // Content edits must re-index against the pre-edit text.
        if let Some(new_content) = redacted.as_deref() {
            fts_index_delete(&mut *tx, rowid, &old_content).await?;
            fts_index_insert(&mut *tx, rowid, new_content).await?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Permanently delete a row `actor` owns. Deleting a memory that does not
    /// exist stays an idempotent no-op (historical semantics); deleting one that
    /// belongs to ANOTHER companion is a `NotFound` error, never a quiet success.
    pub async fn delete_memory(&self, memory_id: &str, actor: &MemoryActor) -> Result<(), AppError> {
        CompanionMemoryId::try_from(memory_id)
            .map_err(|error| AppError::BadRequest(format!("invalid memory id: {error}")))?;
        actor.validate()?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let Some((rowid, content)) = locate_memory_for_mutation(&mut *tx, memory_id, actor).await? else {
            return Ok(());
        };
        sqlx::query("DELETE FROM companion_memories WHERE memory_id = ?")
            .bind(memory_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        fts_index_delete(&mut *tx, rowid, &content).await?;
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Apply one [`MemoryBatchAction`] to every id in a SINGLE transaction —
    /// atomic: any invalid/missing id, or any id `actor` does not own, rolls the
    /// whole batch back.
    pub async fn batch_update_memories(
        &self,
        ids: &[String],
        action: &MemoryBatchAction,
        actor: &MemoryActor,
    ) -> Result<(), AppError> {
        if ids.is_empty() {
            return Err(AppError::BadRequest("batch ids must not be empty".into()));
        }
        for id in ids {
            CompanionMemoryId::try_from(id.as_str())
                .map_err(|error| AppError::BadRequest(format!("invalid memory id: {error}")))?;
        }
        actor.validate()?;
        if let MemoryBatchAction::Reclassify { kind } = action
            && !MEMORY_KINDS.contains(&kind.as_str())
        {
            return Err(AppError::BadRequest(format!("invalid memory kind '{kind}'")));
        }
        let now = now_ms();
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        for id in ids {
            // Ownership first: a foreign id errors out here and (by dropping the
            // tx) rolls back whatever the batch already applied.
            let Some((rowid, content)) = locate_memory_for_mutation(&mut *tx, id, actor).await? else {
                return Err(memory_not_found(id));
            };
            match action {
                MemoryBatchAction::Archive => {
                    sqlx::query("UPDATE companion_memories SET status = 'archived', updated_at = ? WHERE memory_id = ?")
                        .bind(now)
                        .bind(id)
                        .execute(&mut *tx)
                        .await
                        .map_err(db_err)?;
                }
                MemoryBatchAction::Restore => {
                    sqlx::query("UPDATE companion_memories SET status = 'active', updated_at = ? WHERE memory_id = ?")
                        .bind(now)
                        .bind(id)
                        .execute(&mut *tx)
                        .await
                        .map_err(db_err)?;
                }
                MemoryBatchAction::Reclassify { kind } => {
                    sqlx::query("UPDATE companion_memories SET kind = ?, updated_at = ? WHERE memory_id = ?")
                        .bind(kind)
                        .bind(now)
                        .bind(id)
                        .execute(&mut *tx)
                        .await
                        .map_err(db_err)?;
                }
                MemoryBatchAction::Delete => {
                    sqlx::query("DELETE FROM companion_memories WHERE memory_id = ?")
                        .bind(id)
                        .execute(&mut *tx)
                        .await
                        .map_err(db_err)?;
                    fts_index_delete(&mut *tx, rowid, &content).await?;
                }
            }
        }
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Merge-assistant执行：insert the user-confirmed merged memory and archive
    /// the source group in one transaction. Sources keep their content (提留痕)
    /// and gain a `superseded_by:{merged_id}` audit tag. The group must be ≥2
    /// active memories that `actor` owns and that share one scope; the merged
    /// memory inherits that scope, the max importance/strength and any pin.
    pub async fn merge_memories(
        &self,
        group: &[String],
        merged_content: &str,
        kind: &str,
        actor: &MemoryActor,
    ) -> Result<CompanionMemory, AppError> {
        if group.len() < 2 {
            return Err(AppError::BadRequest("merge group must contain at least two memories".into()));
        }
        for id in group {
            CompanionMemoryId::try_from(id.as_str())
                .map_err(|error| AppError::BadRequest(format!("invalid memory id: {error}")))?;
        }
        actor.validate()?;
        if !MEMORY_KINDS.contains(&kind) {
            return Err(AppError::BadRequest(format!("invalid memory kind '{kind}'")));
        }
        let merged_content = merged_content.trim();
        if merged_content.is_empty() {
            return Err(AppError::BadRequest("merged content is empty".into()));
        }
        let merged_content = nomi_redact::redact_secrets(merged_content).into_owned();

        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let mut sources = Vec::with_capacity(group.len());
        for id in group {
            let row = sqlx::query("SELECT * FROM companion_memories WHERE memory_id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_err)?
                .ok_or_else(|| memory_not_found(id))?;
            let memory = row_to_memory(&row)?;
            // Ownership gate, same rule as every other mutator: a companion can
            // only merge rows it can see. The mixed-owner check below then keeps
            // an AnyOwner caller from welding two companions' memories together.
            if !actor.can_reach(memory.companion_id.as_deref()) {
                return Err(memory_not_found(id));
            }
            if memory.status != "active" {
                return Err(AppError::BadRequest(format!(
                    "memory '{id}' is not active; only active memories can be merged"
                )));
            }
            sources.push(memory);
        }
        let owner = sources[0].companion_id.clone();
        if sources.iter().any(|m| m.companion_id != owner) {
            return Err(AppError::BadRequest(
                "merge group must share one owner: memories of different companions never merge".into(),
            ));
        }

        let now = now_ms();
        let merged = CompanionMemory {
            memory_id: CompanionMemoryId::new().into_string(),
            kind: kind.to_owned(),
            content: merged_content,
            tags: vec![],
            importance: sources.iter().map(|m| m.importance).fold(0.0, f64::max),
            strength: sources.iter().map(|m| m.strength).fold(0.0, f64::max),
            pinned: sources.iter().any(|m| m.pinned),
            source: "merge".into(),
            status: "active".into(),
            created_at: now,
            updated_at: now,
            last_reinforced_at: now,
            companion_id: owner,
        };
        let row = sqlx::query(
            "INSERT INTO companion_memories(memory_id, kind, content, tags, importance, strength, pinned, source, status, created_at, updated_at, last_reinforced_at, companion_id)
             VALUES(?,?,?,'[]',?,?,?,?,?,?,?,?,?)
             RETURNING id",
        )
        .bind(&merged.memory_id)
        .bind(&merged.kind)
        .bind(&merged.content)
        .bind(merged.importance)
        .bind(merged.strength)
        .bind(merged.pinned as i64)
        .bind(&merged.source)
        .bind(&merged.status)
        .bind(merged.created_at)
        .bind(merged.updated_at)
        .bind(merged.last_reinforced_at)
        .bind(&merged.companion_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        fts_index_insert(&mut *tx, row.get("id"), &merged.content).await?;

        for source in &sources {
            let mut tags = source.tags.clone();
            tags.push(format!("superseded_by:{}", merged.memory_id));
            sqlx::query("UPDATE companion_memories SET status = 'archived', tags = ?, updated_at = ? WHERE memory_id = ?")
                .bind(serde_json::to_string(&tags).map_err(|error| {
                    AppError::Internal(format!("serialize merged memory tags: {error}"))
                })?)
                .bind(now)
                .bind(&source.memory_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(merged)
    }

    /// Reinforce: bump strength toward 1.0 and refresh the reinforcement clock.
    pub async fn reinforce_memories(&self, ids: &[String]) -> Result<(), AppError> {
        let now = now_ms();
        for id in ids {
            CompanionMemoryId::try_from(id.as_str())
                .map_err(|error| AppError::BadRequest(format!("invalid memory id: {error}")))?;
            sqlx::query(
                "UPDATE companion_memories SET strength = MIN(1.0, strength + 0.2), last_reinforced_at = ?, updated_at = ?, status = 'active' WHERE memory_id = ?",
            )
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    /// Supersede: archive replaced memories (kept for provenance).
    pub async fn archive_memories(&self, ids: &[String]) -> Result<(), AppError> {
        let now = now_ms();
        for id in ids {
            CompanionMemoryId::try_from(id.as_str())
                .map_err(|error| AppError::BadRequest(format!("invalid memory id: {error}")))?;
            sqlx::query("UPDATE companion_memories SET status = 'archived', updated_at = ? WHERE memory_id = ?")
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        Ok(())
    }

    /// Apply exponential decay to every non-pinned active memory, archiving
    /// the ones that fall below the threshold. Returns archived count.
    pub async fn decay_memories(&self) -> Result<i64, AppError> {
        let now = now_ms();
        let rows = sqlx::query(
            "SELECT memory_id, kind, strength, last_reinforced_at FROM companion_memories WHERE status = 'active' AND pinned = 0",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let mut archived = 0i64;
        for row in rows {
            let kind: String = row.get("kind");
            let Some(half_life) = half_life_days(&kind) else { continue };
            let strength: f64 = row.get("strength");
            let last: i64 = row.get("last_reinforced_at");
            let age_days = ((now - last).max(0)) as f64 / 86_400_000.0;
            let decayed = strength * 0.5f64.powf(age_days / half_life);
            let id: String = row.get("memory_id");
            CompanionMemoryId::try_from(id.as_str())
                .map_err(|error| invalid_disk_id("memory id", &id, error))?;
            if decayed < ARCHIVE_THRESHOLD {
                sqlx::query("UPDATE companion_memories SET strength = ?, status = 'archived', updated_at = ? WHERE memory_id = ?")
                    .bind(decayed)
                    .bind(now)
                    .bind(&id)
                    .execute(&self.pool)
                    .await
                    .map_err(db_err)?;
                archived += 1;
            } else {
                sqlx::query("UPDATE companion_memories SET strength = ? WHERE memory_id = ?")
                    .bind(decayed)
                    .bind(&id)
                    .execute(&self.pool)
                    .await
                    .map_err(db_err)?;
            }
        }
        Ok(archived)
    }

    /// Top memories for prompt injection: all pinned + per-kind top-N by
    /// strength, within a rough char budget. Scoped to `companion_id` via
    /// [`MEMORY_VISIBILITY_PREDICATE`]: that companion's own memories plus any
    /// unowned row the boot migration has not re-homed yet. Another companion's
    /// memories are never injected here.
    ///
    /// This is the ONLY path that puts memories into a prompt.
    pub async fn memories_for_injection(&self, companion_id: &str, per_kind: i64, char_budget: usize) -> Result<Vec<CompanionMemory>, AppError> {
        validate_companion_id(companion_id, "memory injection companion_id")?;
        let mut picked: Vec<CompanionMemory> = Vec::new();
        let pinned = sqlx::query(&format!(
            "SELECT * FROM companion_memories WHERE status = 'active' AND pinned = 1{MEMORY_VISIBILITY_PREDICATE} ORDER BY strength DESC"
        ))
        .bind(companion_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        picked.extend(pinned.iter().map(row_to_memory).collect::<Result<Vec<_>, _>>()?);
        for kind in MEMORY_KINDS {
            let rows = sqlx::query(&format!(
                "SELECT * FROM companion_memories WHERE status = 'active' AND pinned = 0 AND kind = ?{MEMORY_VISIBILITY_PREDICATE} ORDER BY strength DESC LIMIT ?"
            ))
            .bind(kind)
            .bind(companion_id)
            .bind(per_kind)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
            picked.extend(rows.iter().map(row_to_memory).collect::<Result<Vec<_>, _>>()?);
        }
        let mut used = 0usize;
        picked.retain(|m| {
            used += m.content.len();
            used <= char_budget
        });
        Ok(picked)
    }

    // ----- session windows (伙伴会话窗口归档) -----

    /// The companion's currently-open window, if any.
    pub async fn open_window(&self, companion_id: &str) -> Result<Option<SessionWindow>, AppError> {
        validate_companion_id(companion_id, "session-window companion_id")?;
        let row = sqlx::query(
            "SELECT * FROM companion_session_windows WHERE companion_id = ? AND status = 'open' \
             ORDER BY started_at DESC LIMIT 1",
        )
        .bind(companion_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        row.as_ref().map(row_to_window).transpose()
    }

    /// Get-or-create the companion's open window. A fresh window's `boundary_ts`
    /// is `now` unless `boundary_ts` overrides it (used when rolling over from a
    /// just-closed window so the new window excludes already-archived messages).
    pub async fn ensure_open_window(
        &self,
        companion_id: &str,
        conversation_id: &str,
        boundary_ts: TimestampMs,
    ) -> Result<SessionWindow, AppError> {
        validate_companion_id(companion_id, "session-window companion_id")?;
        validate_conversation_id(conversation_id, "session-window conversation_id")?;
        if let Some(w) = self.open_window(companion_id).await? {
            return Ok(w);
        }
        let now = now_ms();
        let w = SessionWindow {
            session_window_id: CompanionSessionWindowId::new().into_string(),
            companion_id: companion_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            session_day: local_day(now),
            started_at: now,
            last_activity_at: now,
            closed_at: None,
            status: "open".into(),
            message_count: 0,
            boundary_ts,
            digest: None,
            highlights: None,
            token_estimate: 0,
        };
        sqlx::query(
            "INSERT INTO companion_session_windows \
             (session_window_id, companion_id, conversation_id, session_day, started_at, last_activity_at, \
              closed_at, status, message_count, boundary_ts, digest, highlights, token_estimate) \
             VALUES(?,?,?,?,?,?,NULL,'open',0,?,NULL,NULL,0)",
        )
        .bind(&w.session_window_id)
        .bind(&w.companion_id)
        .bind(&w.conversation_id)
        .bind(&w.session_day)
        .bind(w.started_at)
        .bind(w.last_activity_at)
        .bind(w.boundary_ts)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(w)
    }

    /// Record activity on an open window (bumps `last_activity_at` and, when
    /// larger, `message_count`). Never regresses the count so a partial re-scan
    /// can't shrink it.
    pub async fn touch_window(&self, window_id: &str, last_activity_at: TimestampMs, message_count: i64) -> Result<(), AppError> {
        CompanionSessionWindowId::try_from(window_id)
            .map_err(|error| AppError::BadRequest(format!("invalid session-window id: {error}")))?;
        sqlx::query(
            "UPDATE companion_session_windows SET last_activity_at = ?, message_count = MAX(message_count, ?) \
             WHERE session_window_id = ? AND status = 'open'",
        )
        .bind(last_activity_at)
        .bind(message_count)
        .bind(window_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Close a window with its compressed digest. `status` is `archived` (has a
    /// digest) or `skipped` (too little content — digest stays NULL).
    pub async fn close_window(
        &self,
        window_id: &str,
        status: &str,
        digest: Option<&str>,
        highlights: Option<&str>,
        token_estimate: i64,
    ) -> Result<(), AppError> {
        CompanionSessionWindowId::try_from(window_id)
            .map_err(|error| AppError::BadRequest(format!("invalid session-window id: {error}")))?;
        if let Some(highlights) = highlights {
            serde_json::from_str::<serde_json::Value>(highlights).map_err(|error| {
                AppError::BadRequest(format!("invalid session-window highlights JSON: {error}"))
            })?;
        }
        sqlx::query(
            "UPDATE companion_session_windows \
             SET status = ?, digest = ?, highlights = ?, token_estimate = ?, closed_at = ? \
             WHERE session_window_id = ?",
        )
        .bind(status)
        .bind(digest)
        .bind(highlights)
        .bind(token_estimate)
        .bind(now_ms())
        .bind(window_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Archived digests for one companion, most-recent day first. `limit` caps rows.
    pub async fn list_digests(&self, companion_id: &str, limit: i64) -> Result<Vec<SessionWindow>, AppError> {
        validate_companion_id(companion_id, "session-window companion_id")?;
        let rows = sqlx::query(
            "SELECT * FROM companion_session_windows WHERE companion_id = ? AND status = 'archived' \
             ORDER BY started_at DESC LIMIT ?",
        )
        .bind(companion_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.iter().map(row_to_window).collect()
    }

    /// Every LOCAL day (`YYYYMMDD`) this companion has at least one archived
    /// digest for, newest first. The complete set — unlike [`Self::list_digests`]
    /// this is not row-capped, because a day index must not claim a day has no
    /// diary merely because the digest fell outside a page.
    pub async fn archived_digest_days(&self, companion_id: &str) -> Result<Vec<String>, AppError> {
        validate_companion_id(companion_id, "session-window companion_id")?;
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT session_day FROM companion_session_windows \
             WHERE companion_id = ? AND status = 'archived' \
             ORDER BY session_day DESC",
        )
        .bind(companion_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(|(day,)| day).collect())
    }

    /// Digests whose LOCAL start day falls in `[since_day, until_day]` (inclusive,
    /// `YYYYMMDD` string compare). Either bound may be empty to leave it open.
    pub async fn digests_in_range(&self, companion_id: &str, since_day: &str, until_day: &str) -> Result<Vec<SessionWindow>, AppError> {
        validate_companion_id(companion_id, "session-window companion_id")?;
        let rows = sqlx::query(
            "SELECT * FROM companion_session_windows \
             WHERE companion_id = ? AND status = 'archived' \
               AND (? = '' OR session_day >= ?) AND (? = '' OR session_day <= ?) \
             ORDER BY session_day ASC, started_at ASC",
        )
        .bind(companion_id)
        .bind(since_day)
        .bind(since_day)
        .bind(until_day)
        .bind(until_day)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.iter().map(row_to_window).collect()
    }

    /// "去年今日" — archived digests whose day-of-year (`MMDD`) matches `mmdd`,
    /// excluding the current `session_day`, most-recent year first. `mmdd` is the
    /// 4-char suffix of a `YYYYMMDD` day.
    pub async fn digests_on_day_of_year(&self, companion_id: &str, mmdd: &str, exclude_day: &str, limit: i64) -> Result<Vec<SessionWindow>, AppError> {
        validate_companion_id(companion_id, "session-window companion_id")?;
        let rows = sqlx::query(
            "SELECT * FROM companion_session_windows \
             WHERE companion_id = ? AND status = 'archived' \
               AND substr(session_day, 5) = ? AND session_day != ? \
             ORDER BY session_day DESC LIMIT ?",
        )
        .bind(companion_id)
        .bind(mmdd)
        .bind(exclude_day)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.iter().map(row_to_window).collect()
    }

    // ----- export/import support (spec §4.8) -----

    /// Page size for the full-table dump cursors below.
    const DUMP_PAGE: i64 = 500;

    /// Every `companion_memories` row (all statuses, archived included), streamed
    /// out via an id cursor so an arbitrarily large table never needs one
    /// giant query. Ordered by id (stable across calls).
    ///
    /// The memory BUNDLE export is the only production caller left: it packages
    /// the whole hub by definition. Nothing that answers a request for one
    /// companion may use this — see [`Self::dump_active_memories_visible_to`].
    pub async fn dump_memories_all(&self) -> Result<Vec<CompanionMemory>, AppError> {
        let mut out = Vec::new();
        let mut cursor = String::new();
        loop {
            let rows = sqlx::query("SELECT * FROM companion_memories WHERE memory_id > ? ORDER BY memory_id LIMIT ?")
                .bind(&cursor)
                .bind(Self::DUMP_PAGE)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;
            let Some(last) = rows.last() else { break };
            let next_cursor: String = last.get("memory_id");
            CompanionMemoryId::try_from(next_cursor.as_str())
                .map_err(|error| invalid_disk_id("memory id", &next_cursor, error))?;
            cursor = next_cursor;
            out.extend(rows.iter().map(row_to_memory).collect::<Result<Vec<_>, _>>()?);
        }
        Ok(out)
    }

    /// Exactly the memories ONE companion owns (all statuses, archived included),
    /// same id-cursor streaming as [`Self::dump_memories_all`]. Owner-exact on
    /// purpose: unlike the read path's visibility predicate this never picks up a
    /// vestigial unowned row, because a companion bundle must carry that
    /// companion's memories and nobody else's.
    pub async fn dump_memories_for_companion(
        &self,
        companion_id: &str,
    ) -> Result<Vec<CompanionMemory>, AppError> {
        validate_companion_id(companion_id, "memory companion_id")?;
        let mut out = Vec::new();
        let mut cursor = String::new();
        loop {
            let rows = sqlx::query(
                "SELECT * FROM companion_memories \
                 WHERE companion_id = ? AND memory_id > ? \
                 ORDER BY memory_id LIMIT ?",
            )
            .bind(companion_id)
            .bind(&cursor)
            .bind(Self::DUMP_PAGE)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
            let Some(last) = rows.last() else { break };
            let next_cursor: String = last.get("memory_id");
            CompanionMemoryId::try_from(next_cursor.as_str())
                .map_err(|error| invalid_disk_id("memory id", &next_cursor, error))?;
            cursor = next_cursor;
            out.extend(rows.iter().map(row_to_memory).collect::<Result<Vec<_>, _>>()?);
        }
        Ok(out)
    }

    /// Every ACTIVE memory one companion can SEE — its own plus the vestigial
    /// unowned rows, i.e. [`MEMORY_VISIBILITY_PREDICATE`] — for the merge-assistant
    /// dry run. Same id-cursor streaming as [`Self::dump_memories_all`].
    ///
    /// Scoped in SQL rather than filtered by the caller on purpose: the merge
    /// surface belongs to ONE companion, so another companion's memory text must
    /// never leave the process for it, and only the active layer is mergeable.
    pub async fn dump_active_memories_visible_to(
        &self,
        companion_id: &str,
    ) -> Result<Vec<CompanionMemory>, AppError> {
        validate_companion_id(companion_id, "memory companion_id")?;
        let sql = format!(
            "SELECT * FROM companion_memories \
             WHERE status = 'active' AND memory_id > ?{MEMORY_VISIBILITY_PREDICATE} \
             ORDER BY memory_id LIMIT ?"
        );
        let mut out = Vec::new();
        let mut cursor = String::new();
        loop {
            let rows = sqlx::query(&sql)
                .bind(&cursor)
                .bind(companion_id)
                .bind(Self::DUMP_PAGE)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;
            let Some(last) = rows.last() else { break };
            let next_cursor: String = last.get("memory_id");
            CompanionMemoryId::try_from(next_cursor.as_str())
                .map_err(|error| invalid_disk_id("memory id", &next_cursor, error))?;
            cursor = next_cursor;
            out.extend(rows.iter().map(row_to_memory).collect::<Result<Vec<_>, _>>()?);
        }
        Ok(out)
    }

    pub async fn get_memory(&self, memory_id: &str) -> Result<Option<CompanionMemory>, AppError> {
        CompanionMemoryId::try_from(memory_id)
            .map_err(|error| AppError::BadRequest(format!("invalid memory id: {error}")))?;
        let row = sqlx::query("SELECT * FROM companion_memories WHERE memory_id = ?")
            .bind(memory_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.as_ref().map(row_to_memory).transpose()
    }

    /// Fidelity insert for import: every field (memory_id, timestamps, strength,
    /// pinned, source, status, …) is written exactly as given — unlike
    /// [`insert_memory`], nothing is regenerated or clamped. The caller is
    /// responsible for id-collision handling (see `export::import_bundle`).
    pub async fn insert_memory_raw(&self, mem: &CompanionMemory) -> Result<(), AppError> {
        CompanionMemoryId::try_from(mem.memory_id.as_str())
            .map_err(|error| AppError::BadRequest(format!("invalid imported memory id: {error}")))?;
        validate_row_owner(mem.companion_id.as_deref())?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let row = sqlx::query(
            "INSERT INTO companion_memories(memory_id, kind, content, tags, importance, strength, pinned, source, status, created_at, updated_at, last_reinforced_at, companion_id)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)
             RETURNING id",
        )
        .bind(&mem.memory_id)
        .bind(&mem.kind)
        .bind(&mem.content)
        .bind(
            serde_json::to_string(&mem.tags).map_err(|error| {
                AppError::BadRequest(format!("invalid imported memory tags: {error}"))
            })?,
        )
        .bind(mem.importance)
        .bind(mem.strength)
        .bind(mem.pinned as i64)
        .bind(&mem.source)
        .bind(&mem.status)
        .bind(mem.created_at)
        .bind(mem.updated_at)
        .bind(mem.last_reinforced_at)
        .bind(&mem.companion_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        fts_index_insert(&mut *tx, row.get("id"), &mem.content).await?;
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    // ----- companion threads -----

    /// Register a conversation as a companion thread (idempotent upsert).
    /// Both IDs must be canonical. Re-registering an existing thread refreshes
    /// title/clock and preserves the one-thread-per-companion invariant.
    pub async fn insert_companion_thread(
        &self,
        conversation_id: &str,
        companion_id: &str,
        title: &str,
    ) -> Result<CompanionThread, AppError> {
        validate_conversation_id(conversation_id, "companion thread conversation_id")?;
        validate_companion_id(companion_id, "companion thread companion_id")?;
        let now = now_ms();
        // The canonical conversation ID is the stable thread identity. An
        // upsert refreshes mutable thread metadata for that same entity.
        let row = sqlx::query(
            "INSERT INTO companion_threads(conversation_id, companion_id, title, created_at, updated_at) VALUES(?,?,?,?,?)
             ON CONFLICT(conversation_id) DO UPDATE SET companion_id = excluded.companion_id, title = excluded.title, updated_at = excluded.updated_at
             RETURNING conversation_id, companion_id, title, created_at, updated_at",
        )
        .bind(conversation_id)
        .bind(companion_id)
        .bind(title)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        row_to_companion_thread(&row)
    }

    /// Threads, most recently touched first — all of them, or only one companion's.
    pub async fn list_companion_threads(&self, companion_id: Option<&str>) -> Result<Vec<CompanionThread>, AppError> {
        if let Some(companion_id) = companion_id {
            validate_companion_id(companion_id, "companion thread companion_id")?;
        }
        let rows = if let Some(companion_id) = companion_id {
            sqlx::query("SELECT * FROM companion_threads WHERE companion_id = ? ORDER BY updated_at DESC")
                .bind(companion_id)
                .fetch_all(&self.pool)
                .await
        } else {
            sqlx::query("SELECT * FROM companion_threads ORDER BY updated_at DESC")
                .fetch_all(&self.pool)
                .await
        }
        .map_err(db_err)?;
        rows.iter().map(row_to_companion_thread).collect()
    }

    /// The owning companion of a registered thread. Only an unregistered
    /// conversation returns `None`; ownerless disk rows cannot be created by the v3 schema.
    pub async fn thread_companion_id(&self, conversation_id: &str) -> Result<Option<String>, AppError> {
        validate_conversation_id(conversation_id, "companion thread conversation_id")?;
        let row = sqlx::query("SELECT companion_id FROM companion_threads WHERE conversation_id = ?")
            .bind(conversation_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(row) = row else { return Ok(None) };
        let companion_id: String = row.get("companion_id");
        CompanionId::try_from(companion_id.as_str())
            .map_err(|error| invalid_disk_id("thread companion id", &companion_id, error))?;
        Ok(Some(companion_id))
    }

    pub async fn is_companion_thread(&self, conversation_id: &str) -> Result<bool, AppError> {
        validate_conversation_id(conversation_id, "companion thread conversation_id")?;
        let row = sqlx::query("SELECT 1 AS x FROM companion_threads WHERE conversation_id = ?")
            .bind(conversation_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.is_some())
    }

    pub async fn delete_companion_thread(&self, conversation_id: &str) -> Result<(), AppError> {
        validate_conversation_id(conversation_id, "companion thread conversation_id")?;
        sqlx::query("DELETE FROM companion_threads WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}


// ---------------------------------------------------------------------------
// 自进化：技能注册表 / 挖矿统计 / 反馈回流
// 正文以磁盘 SKILL.md 为事实源（见 nomifun-extension::skill_service）；这里只存
// 元数据 + 溯源 + 生命周期。共享技能已作为产品概念删除：每个技能行都属于恰好
// 一个伙伴（companion_id）；companion_id = NULL 只是启动迁移还没认领的遗留行。
// ---------------------------------------------------------------------------

/// 一个伙伴自进化技能的注册表行。
///
/// The retired `scope_kind` discriminator is neither stored nor carried: it was
/// fully determined by whether there is an owner, so having both invited the two
/// to disagree (and shipped a dead discriminator over the wire). `companion_id`
/// is the single answer to "whose skill is this".
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionSkill {
    #[serde(deserialize_with = "deserialize_uuidv7_string")]
    pub companion_skill_id: String,
    pub skill_name: String,
    /// The owning companion (`companion_skills.companion_id`). `None` is only the
    /// vestigial legacy state — a row written when skills could be shared, which
    /// [`backfill_skill_owner`] re-homes at the first launch that has a companion
    /// to re-home it onto.
    ///
    /// Serialized under the column's own name, exactly like
    /// [`CompanionMemory::companion_id`]: the field used to travel as
    /// `scope_companion_id` next to a retired `scope_kind` discriminator, and both
    /// retired names are now gone from the wire. [`crate::export`] accepts and
    /// translates them when importing a bundle written by an older build — the only
    /// place the retired spelling can still arrive from, because a bundle is a file
    /// on the owner's disk rather than a request we can ask them to re-issue.
    pub companion_id: Option<String>,
    pub status: String,
    pub source: String,
    pub confidence: f64,
    #[serde(deserialize_with = "deserialize_uuidv7_strings")]
    pub provenance_event_ids: Vec<String>,
    pub strength: f64,
    pub version: i64,
    /// Logical reference to the mined pattern that produced this skill.
    #[serde(deserialize_with = "deserialize_optional_uuidv7_string")]
    pub skill_pattern_id: Option<String>,
    pub usage_count: i64,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Originating mined-pattern signature ("" for manual/demonstrated skills);
    /// used to suppress a rejected pattern from re-proposal (纠偏回流).
    #[serde(default)]
    pub signature: String,
}

/// One page of a companion's own skills and the number of matching rows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompanionSkillPage {
    pub items: Vec<CompanionSkill>,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPattern {
    pub skill_pattern_id: String,
    pub signature: String,
    pub status: String,
}

fn row_to_skill(row: &sqlx::sqlite::SqliteRow) -> Result<CompanionSkill, AppError> {
    let companion_skill_id: String = row.get("companion_skill_id");
    validate_uuidv7(&companion_skill_id)
        .map_err(|error| invalid_disk_id("companion skill id", &companion_skill_id, error))?;
    let provenance: String = row.get("provenance_event_ids");
    let provenance_event_ids: Vec<String> = serde_json::from_str(&provenance).map_err(|error| {
        AppError::Internal(format!(
            "companion store skill '{}' contains invalid provenance_event_ids JSON: {error}",
            companion_skill_id
        ))
    })?;
    for event_id in &provenance_event_ids {
        validate_uuidv7(event_id)
            .map_err(|error| invalid_disk_id("skill provenance event id", event_id, error))?;
    }
    let companion_id: Option<String> = row.get("companion_id");
    if let Some(owner) = companion_id.as_deref() {
        CompanionId::try_from(owner)
            .map_err(|error| invalid_disk_id("skill companion id", owner, error))?;
    }
    let skill_pattern_id: Option<String> = row.get("skill_pattern_id");
    if let Some(skill_pattern_id) = skill_pattern_id.as_deref() {
        validate_uuidv7(skill_pattern_id)
            .map_err(|error| invalid_disk_id("skill pattern id", skill_pattern_id, error))?;
    }
    Ok(CompanionSkill {
        companion_skill_id,
        skill_name: row.get("skill_name"),
        companion_id,
        status: row.get("status"),
        source: row.get("source"),
        confidence: row.get("confidence"),
        provenance_event_ids,
        strength: row.get("strength"),
        version: row.get("version"),
        skill_pattern_id,
        usage_count: row.get("usage_count"),
        last_used_at: row.get("last_used_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        signature: row.get("signature"),
    })
}

impl CompanionStore {
    /// Read every durable skill row for the startup filesystem inventory audit.
    pub(crate) async fn list_all_skills(&self) -> Result<Vec<CompanionSkill>, AppError> {
        let rows = sqlx::query("SELECT * FROM companion_skills ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.iter().map(row_to_skill).collect()
    }

    /// Insert or update a skill registry row by its durable business ID.
    ///
    /// An owner is mandatory: 共享技能 is gone, so there is no legitimate way to
    /// create an ownerless row any more. Failing here is what keeps the vestigial
    /// unowned shape a read-only legacy artefact instead of something a new write
    /// path can resurrect.
    pub async fn insert_skill(&self, s: &CompanionSkill) -> Result<(), AppError> {
        validate_uuidv7(&s.companion_skill_id)
            .map_err(|error| AppError::BadRequest(format!("invalid companion_skill_id: {error}")))?;
        let Some(owner) = s.companion_id.as_deref() else {
            return Err(AppError::BadRequest(
                "a companion skill must belong to one companion (companion_id)".into(),
            ));
        };
        validate_companion_id(owner, "skill companion_id")?;
        for event_id in &s.provenance_event_ids {
            validate_uuidv7(event_id).map_err(|error| {
                AppError::BadRequest(format!(
                    "invalid skill provenance_event_ids entry {event_id:?}: {error}"
                ))
            })?;
        }
        if let Some(skill_pattern_id) = s.skill_pattern_id.as_deref() {
            validate_uuidv7(skill_pattern_id)
                .map_err(|error| AppError::BadRequest(format!("invalid skill_pattern_id: {error}")))?;
            let parent_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM skill_pattern_stats WHERE skill_pattern_id = ?
                 )",
            )
            .bind(skill_pattern_id)
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
            if !parent_exists {
                return Err(AppError::BadRequest(format!(
                    "skill_pattern_id '{skill_pattern_id}' does not reference an existing pattern"
                )));
            }
        }
        let provenance_event_ids = serde_json::to_string(&s.provenance_event_ids)
            .map_err(|error| AppError::BadRequest(format!("invalid skill provenance_event_ids: {error}")))?;
        sqlx::query(
            "INSERT INTO companion_skills(companion_skill_id, skill_name, companion_id, status, source, confidence,
                provenance_event_ids, strength, version, skill_pattern_id, usage_count, last_used_at, created_at, updated_at, signature)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(companion_skill_id) DO UPDATE SET
                skill_name=excluded.skill_name, companion_id=excluded.companion_id,
                status=excluded.status, source=excluded.source, confidence=excluded.confidence,
                provenance_event_ids=excluded.provenance_event_ids, strength=excluded.strength,
                version=excluded.version, skill_pattern_id=excluded.skill_pattern_id,
                usage_count=excluded.usage_count, last_used_at=excluded.last_used_at,
                updated_at=excluded.updated_at, signature=excluded.signature",
        )
        .bind(&s.companion_skill_id)
        .bind(&s.skill_name)
        .bind(owner)
        .bind(&s.status)
        .bind(&s.source)
        .bind(s.confidence)
        .bind(&provenance_event_ids)
        .bind(s.strength)
        .bind(s.version)
        .bind(&s.skill_pattern_id)
        .bind(s.usage_count)
        .bind(s.last_used_at)
        .bind(s.created_at)
        .bind(s.updated_at)
        .bind(&s.signature)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// A companion's skills. There is no cross-companion read: a skill belongs
    /// to exactly one companion, so this is the whole list.
    pub async fn list_skills(&self, companion_id: &str) -> Result<Vec<CompanionSkill>, AppError> {
        validate_companion_id(companion_id, "skill companion_id")?;
        let rows = sqlx::query(
            "SELECT * FROM companion_skills WHERE companion_id = ? \
             ORDER BY strength DESC, updated_at DESC, skill_name ASC",
        )
        .bind(companion_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.iter().map(row_to_skill).collect()
    }

    /// One page of a companion's own skills, optionally limited to one lifecycle status.
    pub async fn list_skill_page(
        &self,
        companion_id: &str,
        status: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<CompanionSkillPage, AppError> {
        validate_companion_id(companion_id, "skill companion_id")?;
        let status_clause = if status.is_some() { " AND status = ?" } else { "" };
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);

        let items_sql = format!(
            "SELECT * FROM companion_skills WHERE companion_id = ?{status_clause} \
             ORDER BY strength DESC, updated_at DESC, skill_name ASC LIMIT ? OFFSET ?"
        );
        let mut items_query = sqlx::query(&items_sql).bind(companion_id);
        if let Some(status) = status {
            items_query = items_query.bind(status);
        }
        let rows = items_query
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;

        let count_sql = format!(
            "SELECT COUNT(*) AS n FROM companion_skills WHERE companion_id = ?{status_clause}"
        );
        let mut count_query = sqlx::query(&count_sql).bind(companion_id);
        if let Some(status) = status {
            count_query = count_query.bind(status);
        }
        let total = count_query.fetch_one(&self.pool).await.map_err(db_err)?.get("n");

        Ok(CompanionSkillPage {
            items: rows.iter().map(row_to_skill).collect::<Result<Vec<_>, _>>()?,
            total,
        })
    }

    pub async fn get_skill(&self, companion_skill_id: &str) -> Result<Option<CompanionSkill>, AppError> {
        validate_uuidv7(companion_skill_id)
            .map_err(|error| AppError::BadRequest(format!("invalid companion_skill_id: {error}")))?;
        let row = sqlx::query("SELECT * FROM companion_skills WHERE companion_skill_id = ?")
            .bind(companion_skill_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.as_ref().map(row_to_skill).transpose()
    }

    pub async fn get_owned_skill(
        &self,
        companion_id: &str,
        companion_skill_id: &str,
    ) -> Result<Option<CompanionSkill>, AppError> {
        validate_companion_id(companion_id, "skill companion_id")?;
        validate_uuidv7(companion_skill_id)
            .map_err(|error| AppError::BadRequest(format!("invalid companion_skill_id: {error}")))?;
        let row = sqlx::query(
            "SELECT * FROM companion_skills
             WHERE companion_id = ? AND companion_skill_id = ?",
        )
            .bind(companion_id)
            .bind(companion_skill_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.as_ref().map(row_to_skill).transpose()
    }

    pub async fn find_owned_skill_by_name(
        &self,
        companion_id: &str,
        skill_name: &str,
    ) -> Result<Option<CompanionSkill>, AppError> {
        validate_companion_id(companion_id, "skill companion_id")?;
        let row = sqlx::query(
            "SELECT * FROM companion_skills
             WHERE companion_id = ? AND skill_name = ?",
        )
        .bind(companion_id)
        .bind(skill_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        row.as_ref().map(row_to_skill).transpose()
    }

    pub async fn set_skill_status(
        &self,
        companion_skill_id: &str,
        status: &str,
    ) -> Result<(), AppError> {
        validate_uuidv7(companion_skill_id)
            .map_err(|error| AppError::BadRequest(format!("invalid companion_skill_id: {error}")))?;
        let result = sqlx::query(
            "UPDATE companion_skills SET status = ?, updated_at = ?
             WHERE companion_skill_id = ?",
        )
            .bind(status)
            .bind(now_ms())
            .bind(companion_skill_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound(format!(
                "companion skill '{companion_skill_id}' not found"
            )));
        }
        Ok(())
    }

    async fn record_skill_usage(
        &self,
        companion_skill_id: &str,
        now: i64,
    ) -> Result<(), AppError> {
        validate_uuidv7(companion_skill_id)
            .map_err(|error| AppError::BadRequest(format!("invalid companion_skill_id: {error}")))?;
        // Bump usage AND reinforce strength toward 1.0 (mirrors reinforce_memories) so that
        // a frequently-used skill survives the decay pass — "used skills stay sharp".
        sqlx::query(
            "UPDATE companion_skills SET usage_count = usage_count + 1, last_used_at = ?, \
             strength = MIN(1.0, strength + 0.1), updated_at = ? \
             WHERE companion_skill_id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(companion_skill_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Resolve the runtime tool's human-readable name to one durable row of
    /// `owner`'s, then perform the durable update by `companion_skill_id`.
    /// Name lookups are owner-scoped: two companions may hold same-named skills
    /// and the usage must land on the one that was actually loaded.
    pub async fn record_skill_usage_by_name(
        &self,
        owner: &str,
        skill_name: &str,
        now: i64,
    ) -> Result<(), AppError> {
        validate_companion_id(owner, "skill companion_id")?;
        let companion_skill_id: Option<String> = sqlx::query_scalar(
            "SELECT companion_skill_id FROM companion_skills
             WHERE companion_id = ? AND skill_name = ?",
        )
        .bind(owner)
        .bind(skill_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        if let Some(companion_skill_id) = companion_skill_id {
            self.record_skill_usage(&companion_skill_id, now).await?;
        }
        Ok(())
    }

    /// Decay `owner`'s active-skill strength by age since last use; auto-archive those below
    /// threshold.
    /// Manual/demonstrated skills (`source != 'mined'`) never decay (analog of profile memories).
    /// This is NOT a user rejection: it writes no feedback and never suppresses the originating
    /// pattern, so resumed behavior can be re-mined. Only flips the DB row (SKILL.md stays). Returns archived count.
    ///
    /// Owner-scoped because forgetting is per companion: each companion's own
    /// `evolve.skill_half_life_days` sets its clock, and its run must not archive a
    /// sibling's skill. The vestigial unowned (`companion_id IS NULL`) rows are therefore
    /// never decayed — they only exist in a zero-companion install, where no run
    /// happens at all, and the boot migration re-homes them the moment one exists.
    pub async fn decay_skills(
        &self,
        owner: &str,
        half_life_days: f64,
        archive_threshold: f64,
    ) -> Result<Vec<CompanionSkill>, AppError> {
        validate_companion_id(owner, "skill decay companion_id")?;
        let now = now_ms();
        let rows = sqlx::query(
            "SELECT companion_skill_id, companion_id, skill_name, source, strength,
                    COALESCE(last_used_at, created_at) AS clock \
             FROM companion_skills WHERE status = 'active' AND companion_id = ?",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let half = half_life_days.max(0.1);
        let mut archived = Vec::new();
        for row in rows {
            let source: String = row.get("source");
            if source != "mined" {
                continue; // manual / demonstrated skills never decay
            }
            let strength: f64 = row.get("strength");
            let clock: i64 = row.get("clock");
            let age_days = ((now - clock).max(0)) as f64 / 86_400_000.0;
            let decayed = strength * 0.5f64.powf(age_days / half);
            let cid: Option<String> = row.get("companion_id");
            if let Some(companion_id) = cid.as_deref() {
                CompanionId::try_from(companion_id)
                    .map_err(|error| invalid_disk_id("skill scope companion id", companion_id, error))?;
            }
            let companion_skill_id: String = row.get("companion_skill_id");
            validate_uuidv7(&companion_skill_id).map_err(|error| {
                invalid_disk_id("companion skill id", &companion_skill_id, error)
            })?;
            if decayed < archive_threshold {
                sqlx::query(
                    "UPDATE companion_skills
                     SET strength = ?, status = 'archived', updated_at = ?
                     WHERE companion_skill_id = ?",
                )
                    .bind(decayed)
                    .bind(now)
                    .bind(&companion_skill_id)
                    .execute(&self.pool)
                    .await
                    .map_err(db_err)?;
                let archived_row = sqlx::query(
                    "SELECT * FROM companion_skills WHERE companion_skill_id = ?",
                )
                .bind(&companion_skill_id)
                .fetch_one(&self.pool)
                .await
                .map_err(db_err)?;
                archived.push(row_to_skill(&archived_row)?);
            } else {
                sqlx::query(
                    "UPDATE companion_skills SET strength = ? WHERE companion_skill_id = ?",
                )
                    .bind(decayed)
                    .bind(&companion_skill_id)
                    .execute(&self.pool)
                    .await
                    .map_err(db_err)?;
            }
        }
        Ok(archived)
    }

    /// Count the skills a companion generated since `since_ms` — the weekly
    /// digest's "what I learned". Deliberately not split by lifecycle status:
    /// how many of them are currently *active* was the 专精 badge's number, and
    /// that framing is gone.
    pub async fn count_skills_since(&self, companion_id: &str, since_ms: i64) -> Result<i64, AppError> {
        validate_companion_id(companion_id, "skill companion_id")?;
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM companion_skills WHERE companion_id = ? AND created_at >= ?",
        )
        .bind(companion_id)
        .bind(since_ms)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(n)
    }

    /// Skill names created since `since_ms`, newest first (for the weekly digest list).
    pub async fn list_skill_names_since(&self, companion_id: &str, since_ms: i64, limit: i64) -> Result<Vec<String>, AppError> {
        validate_companion_id(companion_id, "skill companion_id")?;
        let rows = sqlx::query(
            "SELECT skill_name FROM companion_skills WHERE companion_id = ? AND created_at >= ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(companion_id)
        .bind(since_ms)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(|r| r.get::<String, _>("skill_name")).collect())
    }

    /// Count active memories created since `since_ms` that `companion_id` can
    /// read — the weekly digest is rendered per companion, so it must not count
    /// the rest of the roster's memories.
    pub async fn count_memories_since(&self, since_ms: i64, companion_id: &str) -> Result<i64, AppError> {
        validate_companion_id(companion_id, "memory count companion_id")?;
        let n: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM companion_memories WHERE status = 'active' AND created_at >= ?{MEMORY_VISIBILITY_PREDICATE}"
        ))
        .bind(since_ms)
        .bind(companion_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(n)
    }

    /// Find an existing active/draft skill of this companion whose NAME is near-identical to
    /// `name` (exact lowercased, or ≥0.6 containment) — for evolve-in-place instead of duplicating.
    /// Returns the durable skill row. Same-name is excluded because name
    /// collisions remain a filesystem constraint, not an entity identity.
    pub async fn find_similar_skill(
        &self,
        companion_id: &str,
        name: &str,
    ) -> Result<Option<CompanionSkill>, AppError> {
        validate_companion_id(companion_id, "skill companion_id")?;
        let target = name.to_lowercase();
        let rows = sqlx::query(
            "SELECT * FROM companion_skills
             WHERE companion_id = ? AND status IN ('active','draft')",
        )
        .bind(companion_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        for row in &rows {
            let existing: String = row.get("skill_name");
            if existing == name {
                continue;
            }
            let e = existing.to_lowercase();
            if e == target {
                return row_to_skill(row).map(Some);
            }
            let (short, long) = if e.len() <= target.len() { (&e, &target) } else { (&target, &e) };
            if !short.is_empty() && long.contains(short.as_str()) && (short.len() as f64 / long.len() as f64) >= 0.6 {
                return row_to_skill(row).map(Some);
            }
        }
        Ok(None)
    }

    /// Bump a skill's version (on evolve-in-place).
    pub async fn bump_skill_version(&self, companion_skill_id: &str) -> Result<(), AppError> {
        validate_uuidv7(companion_skill_id)
            .map_err(|error| AppError::BadRequest(format!("invalid companion_skill_id: {error}")))?;
        sqlx::query(
            "UPDATE companion_skills SET version = version + 1, updated_at = ?
             WHERE companion_skill_id = ?",
        )
            .bind(now_ms())
            .bind(companion_skill_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    /// Record one pattern occurrence and retain at most 50 fixed-structure
    /// samples. `distinct_sessions` is the count of distinct conversation IDs.
    pub async fn bump_pattern(
        &self,
        signature: &str,
        conversation_id: &str,
        event_id: &str,
        now: i64,
    ) -> Result<SkillPattern, AppError> {
        if signature.trim().is_empty() {
            return Err(AppError::BadRequest(
                "pattern signature must not be empty".into(),
            ));
        }
        let conversation_id = ConversationId::try_from(conversation_id)
            .map_err(|error| AppError::BadRequest(format!("invalid pattern conversation_id: {error}")))?;
        validate_uuidv7(event_id)
            .map_err(|error| AppError::BadRequest(format!("invalid pattern event_id: {error}")))?;
        let existing = sqlx::query(
            "SELECT skill_pattern_id, examples, status
             FROM skill_pattern_stats WHERE signature = ? ORDER BY id ASC LIMIT 1",
        )
            .bind(signature)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let skill_pattern_id = existing
            .as_ref()
            .map(|row| row.get::<String, _>("skill_pattern_id"))
            .unwrap_or_else(|| CompanionSkillPatternId::new().into_string());
        validate_uuidv7(&skill_pattern_id).map_err(|error| {
            invalid_disk_id("skill pattern id", &skill_pattern_id, error)
        })?;
        let mut examples: Vec<PatternExample> = existing
            .as_ref()
            .map(|row| row.get::<String, _>("examples"))
            .as_deref()
            .map(|raw| {
                serde_json::from_str(raw).map_err(|error| {
                    AppError::Internal(format!(
                        "companion store pattern {signature:?} contains invalid examples JSON: {error}"
                    ))
                })
            })
            .transpose()?
            .unwrap_or_default();
        examples.push(PatternExample {
            conversation_id,
            event_id: event_id.to_owned(),
        });
        if examples.len() > 50 {
            let cut = examples.len() - 50;
            examples.drain(0..cut);
        }
        let distinct: std::collections::HashSet<&str> =
            examples.iter().map(|sample| sample.conversation_id.as_str()).collect();
        let distinct_n = distinct.len() as i64;
        let examples_json = serde_json::to_string(&examples)
            .map_err(|error| AppError::Internal(format!("serialize pattern examples: {error}")))?;
        if existing.is_some() {
            sqlx::query(
                "UPDATE skill_pattern_stats
                 SET occurrence_count = occurrence_count + 1,
                     distinct_sessions = ?, examples = ?, last_seen = ?
                 WHERE skill_pattern_id = ?",
            )
            .bind(distinct_n)
            .bind(&examples_json)
            .bind(now)
            .bind(&skill_pattern_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        } else {
            sqlx::query(
                "INSERT INTO skill_pattern_stats(
                    skill_pattern_id, signature, occurrence_count,
                    distinct_sessions, examples, status, last_seen
                 ) VALUES(?,?,1,?,?,'open',?)",
            )
            .bind(&skill_pattern_id)
            .bind(signature)
            .bind(distinct_n)
            .bind(&examples_json)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        }
        Ok(SkillPattern {
            skill_pattern_id,
            signature: signature.to_owned(),
            status: existing
                .as_ref()
                .map(|row| row.get("status"))
                .unwrap_or_else(|| "open".to_owned()),
        })
    }

    pub async fn mark_pattern_status(
        &self,
        skill_pattern_id: &str,
        status: &str,
    ) -> Result<(), AppError> {
        validate_uuidv7(skill_pattern_id)
            .map_err(|error| AppError::BadRequest(format!("invalid skill_pattern_id: {error}")))?;
        let result = sqlx::query(
            "UPDATE skill_pattern_stats SET status = ? WHERE skill_pattern_id = ?",
        )
            .bind(status)
            .bind(skill_pattern_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound(format!(
                "skill pattern '{skill_pattern_id}' not found"
            )));
        }
        Ok(())
    }

    /// Resolve a derived signature to the durable pattern identity.
    pub async fn find_pattern_by_signature(
        &self,
        signature: &str,
    ) -> Result<Option<SkillPattern>, AppError> {
        let row = sqlx::query(
            "SELECT skill_pattern_id, signature, status
             FROM skill_pattern_stats WHERE signature = ? ORDER BY id ASC LIMIT 1",
        )
            .bind(signature)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.map(|row| {
            let skill_pattern_id: String = row.get("skill_pattern_id");
            validate_uuidv7(&skill_pattern_id).map_err(|error| {
                invalid_disk_id("skill pattern id", &skill_pattern_id, error)
            })?;
            Ok(SkillPattern {
                skill_pattern_id,
                signature: row.get("signature"),
                status: row.get("status"),
            })
        })
        .transpose()
    }

    pub async fn record_feedback(
        &self,
        feedback_id: &str,
        companion_skill_id: &str,
        skill_name_snapshot: &str,
        skill_pattern_id: Option<&str>,
        signature_snapshot: Option<&str>,
        decision: &str,
        reason: Option<&str>,
        now: i64,
    ) -> Result<(), AppError> {
        nomifun_common::CompanionEvolutionFeedbackId::try_from(feedback_id).map_err(|error| {
            AppError::BadRequest(format!("invalid evolution feedback id: {error}"))
        })?;
        validate_uuidv7(companion_skill_id)
            .map_err(|error| AppError::BadRequest(format!("invalid companion_skill_id: {error}")))?;
        if let Some(skill_pattern_id) = skill_pattern_id {
            validate_uuidv7(skill_pattern_id)
                .map_err(|error| AppError::BadRequest(format!("invalid skill_pattern_id: {error}")))?;
        }
        if skill_name_snapshot.trim().is_empty() {
            return Err(AppError::BadRequest(
                "evolution feedback skill_name_snapshot must not be empty".into(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let skill_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM companion_skills WHERE companion_skill_id = ?
             )",
        )
        .bind(companion_skill_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        if !skill_exists {
            return Err(AppError::BadRequest(format!(
                "companion_skill_id '{companion_skill_id}' does not reference an existing skill"
            )));
        }
        if let Some(skill_pattern_id) = skill_pattern_id {
            let pattern_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM skill_pattern_stats WHERE skill_pattern_id = ?
                 )",
            )
            .bind(skill_pattern_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
            if !pattern_exists {
                return Err(AppError::BadRequest(format!(
                    "skill_pattern_id '{skill_pattern_id}' does not reference an existing pattern"
                )));
            }
        }
        sqlx::query(
            "INSERT INTO evolution_feedback(
                feedback_id, companion_skill_id, skill_name_snapshot,
                skill_pattern_id, signature_snapshot, decision, reason, created_at
             ) VALUES(?,?,?,?,?,?,?,?)",
        )
            .bind(feedback_id)
            .bind(companion_skill_id)
            .bind(skill_name_snapshot)
            .bind(skill_pattern_id)
            .bind(signature_snapshot)
            .bind(decision)
            .bind(reason)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// 是否曾被拒绝（负样本）：存在 decision='reject' 的反馈即视为该签名被否决。
    pub async fn is_signature_rejected(&self, signature: &str) -> Result<bool, AppError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM evolution_feedback f
             LEFT JOIN skill_pattern_stats p
               ON p.skill_pattern_id = f.skill_pattern_id
             WHERE (p.signature = ? OR f.signature_snapshot = ?)
               AND f.decision = 'reject'",
        )
            .bind(signature)
            .bind(signature)
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn companion_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        CompanionId::try_from(raw.as_str()).unwrap().into_string()
    }

    fn conversation_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        ConversationId::try_from(raw.as_str()).unwrap().into_string()
    }

    #[tokio::test]
    async fn v3_baseline_all_tables_use_autoincrement_integer_primary_keys() {
        let store = CompanionStore::open_memory().await.unwrap();
        validate_baseline_schema(&store.pool).await.unwrap();
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(version, STORE_VERSION);

        for table in BASELINE_TABLES {
            let columns = sqlx::query(&format!("PRAGMA table_info({})", table.name))
                .fetch_all(&store.pool)
                .await
                .unwrap();
            let id = columns
                .iter()
                .find(|row| row.get::<String, _>("name") == "id")
                .unwrap();
            assert_eq!(id.get::<String, _>("type").to_ascii_uppercase(), "INTEGER");
            assert_eq!(id.get::<i64, _>("pk"), 1);
        }
    }

    /// Replace one fragment of [`SCHEMA`], asserting that it occurs EXACTLY once.
    ///
    /// Every schema-drift fixture below is a string mutation of the real schema,
    /// which makes them silently self-disabling: the day a needle stops matching,
    /// `replacen` returns the schema unchanged and the "malformed" case becomes a
    /// copy of the valid one — a test that passes while testing nothing. This
    /// helper turns that into a loud failure at the mutation site.
    fn mutate_schema(needle: &str, replacement: &str) -> String {
        let occurrences = SCHEMA.matches(needle).count();
        assert_eq!(
            occurrences, 1,
            "schema-drift fixture needle must occur exactly once in SCHEMA (found {occurrences}): {needle}"
        );
        SCHEMA.replacen(needle, replacement, 1)
    }

    async fn assert_malformed_v3_rejected(schema: &str, description: &str) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true))
            .await
            .unwrap();
        sqlx::raw_sql(schema).execute(&pool).await.unwrap();
        // The FTS stanza is not under test here; create it so the failure
        // points at the mutated fragment instead of a missing FTS table.
        sqlx::raw_sql(FTS_SCHEMA).execute(&pool).await.unwrap();
        sqlx::raw_sql(&format!("PRAGMA user_version = {STORE_VERSION}"))
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            validate_baseline_schema(&pool).await.is_err(),
            "malformed v3 schema must be rejected: {description}"
        );
    }

    #[tokio::test]
    async fn v3_baseline_rejects_missing_columns_uuid_checks_uniques_and_indexes() {
        let malformed = [
            (
                mutate_schema("  embedding BLOB,\n  embedding_model TEXT\n", "  embedding BLOB\n"),
                "all tables exist but companion_memories.embedding_model is missing",
            ),
            (
                mutate_schema("  embedding_model TEXT\n", "  embedding_model INTEGER\n"),
                "companion_memories.embedding_model has the wrong declared type",
            ),
            (
                mutate_schema("  embedding_model TEXT\n", "  embedding_model TEXT NOT NULL\n"),
                "companion_memories.embedding_model has the wrong nullability",
            ),
            (
                mutate_schema(
                    "  embedding_model TEXT\n",
                    "  embedding_model TEXT,\n  unexpected TEXT\n",
                ),
                "companion_memories has an extra column",
            ),
            (
                mutate_schema(
                    "memory_id TEXT NOT NULL UNIQUE CHECK (\n    length(memory_id) = 36\n    AND lower(memory_id) = memory_id\n    AND memory_id GLOB '????????-????-7???-[89ab]???-????????????'\n    AND replace(memory_id, '-', '') NOT GLOB '*[^0-9a-f]*'\n  )",
                    "memory_id TEXT NOT NULL UNIQUE",
                ),
                "memory_id has no UUIDv7 CHECK",
            ),
            (
                mutate_schema(
                    "memory_id TEXT NOT NULL UNIQUE CHECK",
                    "memory_id TEXT NOT NULL CHECK",
                ),
                "memory_id has no UNIQUE constraint",
            ),
            (
                // The owner column is nullable and carries no discriminator, so
                // its UUIDv7 CHECK is the only thing standing between the store
                // and an unaddressable owner id.
                mutate_schema(
                    "  companion_id TEXT CHECK (\n    companion_id IS NULL\n    OR (\n      length(companion_id) = 36\n      AND lower(companion_id) = companion_id\n      AND companion_id GLOB '????????-????-7???-[89ab]???-????????????'\n      AND replace(companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'\n    )\n  ),\n  embedding BLOB,",
                    "  companion_id TEXT,\n  embedding BLOB,",
                ),
                "companion_memories.companion_id has no UUIDv7 CHECK",
            ),
            (
                mutate_schema(
                    "CREATE INDEX IF NOT EXISTS idx_companion_memories_kind ON companion_memories(kind, status, strength DESC);\n",
                    "",
                ),
                "required memory kind index is missing",
            ),
            (
                mutate_schema(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_companion_skills_shared_name ON companion_skills(skill_name) WHERE companion_id IS NULL;\n",
                    "",
                ),
                "required partial unique skill index is missing",
            ),
            (
                // Same columns, wrong predicate: the two partial uniques together
                // are what keep one skill name per owner AND one unowned row per
                // name, so a widened predicate must not pass as the baseline.
                mutate_schema(
                    "ON companion_skills(companion_id, skill_name) WHERE companion_id IS NOT NULL;",
                    "ON companion_skills(companion_id, skill_name) WHERE status != 'archived';",
                ),
                "the private-owner unique index has the wrong partial predicate",
            ),
            (
                format!(
                    "{SCHEMA}\nCREATE INDEX unexpected_v3_index ON companion_memories(kind);"
                ),
                "an extra user-defined index is present",
            ),
        ];

        for (schema, description) in malformed {
            assert_malformed_v3_rejected(&schema, description).await;
        }
    }

    #[tokio::test]
    async fn v3_baseline_rejects_non_v3_table_shape() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true))
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE companion_memories (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        assert!(create_baseline_schema(&pool).await.is_err());
    }

    #[tokio::test]
    async fn file_store_rejects_unversioned_or_future_schema_without_repair() {
        for version in [0_i64, STORE_VERSION + 1] {
            let root = tempfile::tempdir().unwrap();
            let database_path = root.path().join("memory.db");
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&database_path)
                        .create_if_missing(true),
                )
                .await
                .unwrap();
            sqlx::raw_sql(SCHEMA).execute(&pool).await.unwrap();
            sqlx::raw_sql(&format!("PRAGMA user_version = {version}"))
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;

            assert!(CompanionStore::open(root.path(), None).await.is_err());
            let verify = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(SqliteConnectOptions::new().filename(&database_path))
                .await
                .unwrap();
            let persisted: i64 = sqlx::query_scalar("PRAGMA user_version")
                .fetch_one(&verify)
                .await
                .unwrap();
            assert_eq!(
                persisted, version,
                "open must not stamp a non-v3 store as v3"
            );
            verify.close().await;
        }
    }

    #[tokio::test]
    async fn state_and_runtime_state_upsert_by_unique_business_keys() {
        let store = CompanionStore::open_memory().await.unwrap();
        store.set_state("cursor", "1").await.unwrap();
        store.set_state("cursor", "2").await.unwrap();
        assert_eq!(store.get_state("cursor").await.unwrap().as_deref(), Some("2"));

        let companion = companion_fixture(1);
        store.set_companion_state(&companion, "mood", "ok").await.unwrap();
        store.set_companion_state(&companion, "mood", "happy").await.unwrap();
        assert_eq!(
            store.get_companion_state(&companion, "mood").await.unwrap().as_deref(),
            Some("happy")
        );
    }

    #[tokio::test]
    async fn memory_uses_a_named_unique_id() {
        let store = CompanionStore::open_memory().await.unwrap();
        let memory = store
            .insert_memory("knowledge", "Rust", &[], 0.8, "manual")
            .await
            .unwrap();
        assert_eq!(
            store.get_memory(&memory.memory_id).await.unwrap().unwrap().memory_id,
            memory.memory_id
        );
    }

    #[tokio::test]
    async fn pattern_examples_persist_event_id_and_reject_generic_id() {
        let store = CompanionStore::open_memory().await.unwrap();
        let conversation_id = conversation_fixture(20);
        let event_id = nomifun_common::generate_id();
        store
            .bump_pattern("grep-read", &conversation_id, &event_id, 1)
            .await
            .unwrap();

        let examples: String =
            sqlx::query_scalar("SELECT examples FROM skill_pattern_stats WHERE signature = ?")
                .bind("grep-read")
                .fetch_one(&store.pool)
                .await
                .unwrap();
        let wire: serde_json::Value = serde_json::from_str(&examples).unwrap();
        assert_eq!(wire[0]["event_id"], event_id);
        assert!(wire[0].get("id").is_none());

        sqlx::query("UPDATE skill_pattern_stats SET examples = ? WHERE signature = ?")
            .bind(format!(
                "[{{\"conversation_id\":\"{conversation_id}\",\"id\":\"{}\"}}]",
                nomifun_common::generate_id()
            ))
            .bind("grep-read")
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(
            store
                .bump_pattern(
                    "grep-read",
                    &conversation_id,
                    &nomifun_common::generate_id(),
                    2
                )
                .await
                .is_err()
        );
    }

    /// The PRE-COLLAPSE v3 layout, i.e. what every install on disk actually has:
    /// both owner tables still carry the retired `(scope_kind, scope_companion_id)`
    /// pair with its paired CHECK, the two partial skill uniques are keyed on
    /// `scope_kind`, `companion_memories` has no embedding columns, and there is no
    /// FTS5 stanza (that lives in `FTS_SCHEMA`).
    ///
    /// Derived from [`SCHEMA`] rather than frozen as a literal so that a baseline
    /// table added later is present here too — `upgrade_schema_in_place` never
    /// CREATES a missing table, so a frozen fixture would rot into a "missing
    /// table" failure that says nothing about this migration. Every needle is
    /// asserted to match exactly once, so the fixture can never quietly stop being
    /// legacy.
    fn legacy_scope_v3_schema() -> String {
        // The three DDL fragments below are verbatim: two of them are the shape
        // 0.3.8 shipped, the third is the shape SCHEMA has now.
        let legacy_owner_columns = |default_kind: &str| {
            format!(
                r#"  scope_kind TEXT NOT NULL DEFAULT '{default_kind}' CHECK(scope_kind IN ('user', 'companion')),
  scope_companion_id TEXT CHECK (
    scope_companion_id IS NULL
    OR (
      length(scope_companion_id) = 36
      AND lower(scope_companion_id) = scope_companion_id
      AND scope_companion_id GLOB '????????-????-7???-[89ab]???-????????????'
      AND replace(scope_companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
  ),
"#
            )
        };
        const PAIRED_CHECK: &str = r#"  CHECK((scope_kind = 'user' AND scope_companion_id IS NULL) OR
        (scope_kind = 'companion' AND scope_companion_id IS NOT NULL))
"#;
        const NEW_OWNER_COLUMN: &str = r#"  companion_id TEXT CHECK (
    companion_id IS NULL
    OR (
      length(companion_id) = 36
      AND lower(companion_id) = companion_id
      AND companion_id GLOB '????????-????-7???-[89ab]???-????????????'
      AND replace(companion_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
  ),
"#;

        let steps: Vec<(String, String)> = vec![
            // companion_memories: the owner column plus the two embedding columns
            // are its whole tail, so one needle restores the legacy tail in full.
            (
                format!("{NEW_OWNER_COLUMN}  embedding BLOB,\n  embedding_model TEXT\n);"),
                format!("{}{PAIRED_CHECK});", legacy_owner_columns("user")),
            ),
            // companion_skills: the owner column sits mid-table, and the paired
            // CHECK goes after the last column.
            (
                format!("{NEW_OWNER_COLUMN}  status TEXT NOT NULL DEFAULT 'draft',"),
                format!(
                    "{}  status TEXT NOT NULL DEFAULT 'draft',",
                    legacy_owner_columns("companion")
                ),
            ),
            (
                "  signature TEXT NOT NULL DEFAULT ''\n);".to_owned(),
                format!("  signature TEXT NOT NULL DEFAULT '',\n{PAIRED_CHECK});"),
            ),
            (
                "ON companion_skills(companion_id, status, strength DESC)".to_owned(),
                "ON companion_skills(scope_companion_id, status, strength DESC)".to_owned(),
            ),
            (
                "ON companion_skills(skill_name) WHERE companion_id IS NULL;".to_owned(),
                "ON companion_skills(skill_name) WHERE scope_kind = 'user';".to_owned(),
            ),
            (
                "ON companion_skills(companion_id, skill_name) WHERE companion_id IS NOT NULL;".to_owned(),
                "ON companion_skills(scope_companion_id, skill_name) WHERE scope_kind = 'companion';".to_owned(),
            ),
        ];
        let mut sql = SCHEMA.to_owned();
        for (needle, replacement) in steps {
            let occurrences = sql.matches(needle.as_str()).count();
            assert_eq!(
                occurrences, 1,
                "legacy fixture needle must occur exactly once (found {occurrences}): {needle}"
            );
            sql = sql.replacen(needle.as_str(), &replacement, 1);
        }
        assert!(
            // Anchored on the line start so `scope_companion_id` does not match it.
            !sql.contains("\n  companion_id TEXT CHECK"),
            "the legacy fixture must carry no collapsed owner column"
        );
        sql
    }

    /// Actual indexed-document count. `count(*)` on an external-content fts5
    /// table mirrors the content table, so the docsize shadow table is the
    /// only honest way to observe the index itself.
    async fn fts_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM companion_memories_fts_docsize")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn fts_match_count(pool: &SqlitePool, term: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM companion_memories_fts WHERE companion_memories_fts MATCH ?")
            .bind(format!("\"{}\"", term.replace('"', "\"\"")))
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// The whole owner migration, on the layout every existing install has: the
    /// retired `(scope_kind, scope_companion_id)` pair is rebuilt into one nullable
    /// `companion_id`, and only then are the unowned rows re-homed. Both halves
    /// must preserve every row EXACTLY — this is the owner's accumulated memory,
    /// and a rebuild that drops, duplicates, renumbers or silently re-owns a row
    /// is unrecoverable for them.
    #[tokio::test]
    async fn legacy_v3_file_store_upgrades_in_place_preserving_memories() {
        let root = tempfile::tempdir().unwrap();
        let database_path = root.path().join("memory.db");
        let memory_id_active = CompanionMemoryId::new().into_string();
        let memory_id_archived = CompanionMemoryId::new().into_string();
        let memory_id_owned_pinned = CompanionMemoryId::new().into_string();
        // Unowned 共享技能 rows, the pre-re-homing shape: ('user', NULL). `collides`
        // shares its name with a skill the future owner already owns.
        let skill_id_shared = nomifun_common::generate_id();
        let skill_id_collides = nomifun_common::generate_id();
        let skill_id_owned = nomifun_common::generate_id();
        let owner = companion_fixture(77);
        // Row ids of the pre-collapse rows: the FTS index is external-content and
        // keyed on `companion_memories.id`, so the rebuild must preserve them.
        let ids_before: Vec<(String, i64)>;
        {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&database_path)
                        .create_if_missing(true),
                )
                .await
                .unwrap();
            sqlx::raw_sql(&legacy_scope_v3_schema()).execute(&pool).await.unwrap();
            sqlx::raw_sql(&format!("PRAGMA user_version = {STORE_VERSION}"))
                .execute(&pool)
                .await
                .unwrap();
            // Owned and unowned, active and archived, pinned and not.
            for (memory_id, status, pinned, scope_kind, scope_owner, content) in [
                (&memory_id_active, "active", 0, "user", None, "主人喜欢深烘焙咖啡豆"),
                (&memory_id_archived, "archived", 0, "user", None, "主人去年在东京出差"),
                (
                    &memory_id_owned_pinned,
                    "active",
                    1,
                    "companion",
                    Some(owner.as_str()),
                    "主人的生日是十一月三号",
                ),
            ] {
                sqlx::query(
                    "INSERT INTO companion_memories(memory_id, kind, content, tags, importance, strength, pinned, source, status, created_at, updated_at, last_reinforced_at, scope_kind, scope_companion_id)
                     VALUES(?, 'preference', ?, '[\"a\"]', 0.8, 0.6, ?, 'manual', ?, 11, 22, 33, ?, ?)",
                )
                .bind(memory_id)
                .bind(content)
                .bind(pinned)
                .bind(status)
                .bind(scope_kind)
                .bind(scope_owner)
                .execute(&pool)
                .await
                .unwrap();
            }
            for (skill_id, name, owner_id, status) in [
                (&skill_id_shared, "legacy-shared", None, "active"),
                (&skill_id_collides, "collides", None, "archived"),
                (&skill_id_owned, "collides", Some(owner.as_str()), "active"),
            ] {
                sqlx::query(
                    "INSERT INTO companion_skills(companion_skill_id, skill_name, scope_kind, scope_companion_id, status, source, confidence, provenance_event_ids, strength, version, usage_count, created_at, updated_at, signature)
                     VALUES(?, ?, ?, ?, ?, 'mined', 0.9, '[]', 1.0, 1, 0, 1, 1, '')",
                )
                .bind(skill_id)
                .bind(name)
                .bind(if owner_id.is_some() { "companion" } else { "user" })
                .bind(owner_id)
                .bind(status)
                .execute(&pool)
                .await
                .unwrap();
            }
            ids_before = sqlx::query("SELECT memory_id, id FROM companion_memories ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get("memory_id"), row.get("id")))
                .collect();
            pool.close().await;
        }

        let store = CompanionStore::open(root.path(), None).await.unwrap();

        // The nullable embedding columns were added in place, and the retired
        // owner pair is physically gone from BOTH tables.
        for (table, expected) in [
            ("companion_memories", vec!["companion_id", "embedding", "embedding_model"]),
            ("companion_skills", vec!["companion_id"]),
        ] {
            let columns: Vec<String> =
                sqlx::query_scalar(&format!("SELECT name FROM pragma_table_xinfo('{table}')"))
                    .fetch_all(&store.pool)
                    .await
                    .unwrap();
            for retired in ["scope_kind", "scope_companion_id"] {
                assert!(
                    !columns.contains(&retired.to_owned()),
                    "{table}.{retired} must be dropped in place: {columns:?}"
                );
            }
            for column in expected {
                assert!(columns.contains(&column.to_owned()), "{table}: {columns:?}");
            }
        }

        // The FTS index exists and was backfilled with every row (active + archived),
        // and it is anchored on the PRESERVED rowids.
        assert_eq!(fts_count(&store.pool).await, 3);
        assert_eq!(fts_match_count(&store.pool, "深烘焙").await, 1);
        assert_eq!(fts_match_count(&store.pool, "东京出差").await, 1);
        assert_eq!(fts_match_count(&store.pool, "十一月三号").await, 1);
        let ids_after: Vec<(String, i64)> =
            sqlx::query("SELECT memory_id, id FROM companion_memories ORDER BY id")
                .fetch_all(&store.pool)
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get("memory_id"), row.get("id")))
                .collect();
        assert_eq!(
            ids_after, ids_before,
            "the rebuild must preserve every rowid: the FTS index is content_rowid='id'"
        );

        // Every row survived the rebuild verbatim — content, lifecycle, pin,
        // strength, tags, timestamps — including the vestigial unowned ownership:
        // an empty roster has no legal owner, so the re-homing migration
        // deliberately leaves those rows for a later open.
        let active = store.get_memory(&memory_id_active).await.unwrap().unwrap();
        assert_eq!(active.content, "主人喜欢深烘焙咖啡豆");
        assert_eq!(active.status, "active");
        assert_eq!(active.companion_id, None);
        assert_eq!(active.tags, vec!["a".to_owned()]);
        assert_eq!((active.importance, active.strength), (0.8, 0.6));
        assert_eq!(
            (active.created_at, active.updated_at, active.last_reinforced_at),
            (11, 22, 33)
        );
        let archived = store.get_memory(&memory_id_archived).await.unwrap().unwrap();
        assert_eq!(archived.status, "archived");
        assert_eq!(archived.companion_id, None);
        // An already-owned row keeps its owner and its pin through the rebuild.
        let owned = store.get_memory(&memory_id_owned_pinned).await.unwrap().unwrap();
        assert_eq!(owned.companion_id.as_deref(), Some(owner.as_str()));
        assert!(owned.pinned, "a pinned row must stay pinned");
        assert!(!active.pinned);
        // Same for the skill rows: no roster, no owner, no re-homing.
        assert_eq!(store.get_skill(&skill_id_shared).await.unwrap().unwrap().companion_id, None);
        assert_eq!(
            store.get_skill(&skill_id_owned).await.unwrap().unwrap().companion_id.as_deref(),
            Some(owner.as_str()),
            "an already-owned skill keeps its owner through the rebuild"
        );

        // Idempotent: a second open of the already-collapsed store is a no-op that
        // still opens, and does not rebuild anything a third time.
        drop(store);
        let reopened = CompanionStore::open(root.path(), None).await.unwrap();
        assert_eq!(fts_count(&reopened.pool).await, 3);
        assert!(!collapse_owner_columns(&reopened.pool).await.unwrap());
        let ids_again: Vec<(String, i64)> =
            sqlx::query("SELECT memory_id, id FROM companion_memories ORDER BY id")
                .fetch_all(&reopened.pool)
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get("memory_id"), row.get("id")))
                .collect();
        assert_eq!(ids_again, ids_before, "the second boot must change nothing");
        drop(reopened);

        // Opening WITH an owner re-homes every unowned row onto it — active and
        // archived alike — preserving each memory verbatim otherwise. This is the
        // migration that deletes 共享记忆 from the data: **每条记忆都不能丢**。
        let migrated = CompanionStore::open(root.path(), Some(&owner)).await.unwrap();
        let unowned_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM companion_memories WHERE companion_id IS NULL")
                .fetch_one(&migrated.pool)
                .await
                .unwrap();
        assert_eq!(unowned_left, 0, "no unowned memory may survive the backfill");
        for (memory_id, content, status) in [
            (&memory_id_active, "主人喜欢深烘焙咖啡豆", "active"),
            (&memory_id_archived, "主人去年在东京出差", "archived"),
            (&memory_id_owned_pinned, "主人的生日是十一月三号", "active"),
        ] {
            let memory = migrated.get_memory(memory_id).await.unwrap().unwrap();
            assert_eq!(memory.content, content, "content must survive verbatim");
            assert_eq!(memory.status, status, "lifecycle must survive verbatim");
            assert_eq!(memory.companion_id.as_deref(), Some(owner.as_str()));
        }
        // RE-HOMED, not duplicated: still exactly three rows and three indexed docs.
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM companion_memories")
            .fetch_one(&migrated.pool)
            .await
            .unwrap();
        assert_eq!(rows, 3, "re-homing must never multiply rows");
        assert_eq!(fts_count(&migrated.pool).await, 3);
        // The re-homed rows now inject for their owner, and the index still
        // answers a query for a row that was written before the rebuild.
        let injected = migrated.memories_for_injection(&owner, 10, 10_000).await.unwrap();
        assert_eq!(injected.len(), 2, "only the two active rows inject: {injected:?}");
        let found = migrated
            .list_memory_page_sorted(
                &MemoryFilter {
                    q: Some("深烘焙".into()),
                    companion_id: Some(owner.clone()),
                    ..Default::default()
                },
                MemoryListSort::Default,
            )
            .await
            .unwrap();
        assert_eq!(found.total, 1, "the re-homed row is still findable by content");

        // The SKILL rows re-home the same way — 共享技能 is gone as a concept, and
        // an unowned row would be invisible in every companion's list. Each row
        // keeps its name and lifecycle; the only thing that changes is the owner.
        let rehomed = migrated.get_skill(&skill_id_shared).await.unwrap().unwrap();
        assert_eq!(rehomed.skill_name, "legacy-shared");
        assert_eq!(rehomed.status, "active");
        assert_eq!(rehomed.companion_id.as_deref(), Some(owner.as_str()));
        assert_eq!(
            migrated.list_skills(&owner).await.unwrap().len(),
            2,
            "the re-homed skill and the pre-owned one both belong to the owner now"
        );
        // A name collision is SKIPPED, never forced: (owner, skill_name) is unique,
        // so forcing it would raise a UNIQUE violation inside the boot migration and
        // fail the launch of every install that has such a pair. The row keeps its
        // legacy shape — nothing is deleted, nothing is overwritten.
        let stranded = migrated.get_skill(&skill_id_collides).await.unwrap().unwrap();
        assert_eq!(stranded.companion_id, None, "a colliding row stays unowned");
        assert_eq!(stranded.status, "archived", "and keeps its lifecycle verbatim");
        let pre_owned = migrated.get_skill(&skill_id_owned).await.unwrap().unwrap();
        assert_eq!(pre_owned.status, "active", "the owner's own same-named skill is untouched");
        let skill_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM companion_skills")
            .fetch_one(&migrated.pool)
            .await
            .unwrap();
        assert_eq!(skill_rows, 3, "re-homing must never add or drop a skill row");

        // Idempotent: re-running with a DIFFERENT owner must not steal already
        // owned rows (the UPDATE only touches companion_id IS NULL).
        drop(migrated);
        let other_owner = companion_fixture(78);
        let again = CompanionStore::open(root.path(), Some(&other_owner)).await.unwrap();
        let still_owned = again.get_memory(&memory_id_active).await.unwrap().unwrap();
        assert_eq!(still_owned.companion_id.as_deref(), Some(owner.as_str()));
        assert_eq!(
            again.get_skill(&skill_id_shared).await.unwrap().unwrap().companion_id.as_deref(),
            Some(owner.as_str()),
            "a second boot must not re-home an already owned skill onto someone else"
        );
        // The collision was skipped, not resolved forever: the roster changed, so
        // the row gets a fresh chance and finally lands on a real owner.
        assert_eq!(
            again.get_skill(&skill_id_collides).await.unwrap().unwrap().companion_id.as_deref(),
            Some(other_owner.as_str())
        );
    }

    #[tokio::test]
    async fn v3_upgrade_drops_retired_tables_and_preserves_learning_and_evolution_state() {
        let root = tempfile::tempdir().unwrap();
        let database_path = root.path().join("memory.db");
        let memory_id = CompanionMemoryId::new().into_string();
        let companion_id = companion_fixture(91);
        let skill_id = nomifun_common::generate_id();
        {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&database_path)
                        .create_if_missing(true),
                )
                .await
                .unwrap();
            sqlx::raw_sql(SCHEMA).execute(&pool).await.unwrap();
            sqlx::raw_sql(FTS_SCHEMA).execute(&pool).await.unwrap();
            // Both retired tables are re-created by hand: they are gone from
            // SCHEMA, so only an explicit legacy stanza can prove the drop.
            sqlx::raw_sql(
                r#"
CREATE TABLE companion_learn_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  learn_run_id TEXT NOT NULL UNIQUE,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  status TEXT NOT NULL,
  events_processed INTEGER NOT NULL DEFAULT 0,
  memories_added INTEGER NOT NULL DEFAULT 0,
  suggestions_added INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  summary TEXT
);
CREATE TABLE companion_suggestions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  suggestion_id TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL,
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  action TEXT,
  status TEXT NOT NULL DEFAULT 'new',
  created_at INTEGER NOT NULL,
  decided_at INTEGER
);
CREATE INDEX idx_companion_suggestions_status ON companion_suggestions(status, created_at DESC);
"#,
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::raw_sql(&format!("PRAGMA user_version = {STORE_VERSION}"))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO companion_memories(memory_id, kind, content, tags, importance, strength, pinned, source, status, created_at, updated_at, last_reinforced_at, companion_id)
                 VALUES(?, 'preference', '保留的学习记忆', '[]', 0.8, 0.8, 0, 'learn', 'active', 1, 1, 1, NULL)",
            )
            .bind(&memory_id)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO companion_skills(companion_skill_id, skill_name, companion_id, status, source, confidence, provenance_event_ids, strength, version, created_at, updated_at, signature)
                 VALUES(?, 'preserved-skill', ?, 'active', 'mined', 0.9, '[]', 1.0, 1, 1, 1, 'grep-read')",
            )
            .bind(&skill_id)
            .bind(&companion_id)
            .execute(&pool)
            .await
            .unwrap();
            for (key, value) in [
                ("last_learn_ts", "101"),
                ("learn_cursor_ts", "102"),
                ("last_evolve_ts", "201"),
                ("evolve_cursor_ts", "202"),
                ("mood", "happy"),
            ] {
                sqlx::query("INSERT INTO companion_state(state_key, value) VALUES(?, ?)")
                    .bind(key)
                    .bind(value)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query(
                "INSERT INTO companion_runtime_state(companion_id, state_key, value) VALUES(?, 'xp', '17')",
            )
            .bind(&companion_id)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO companion_learn_runs(learn_run_id, started_at, finished_at, status, events_processed, memories_added, suggestions_added, summary)
                 VALUES(?, 1, 2, 'ok', 3, 1, 1, 'legacy diary')",
            )
            .bind(nomifun_common::generate_id())
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO companion_suggestions(suggestion_id, kind, title, body, status, created_at)
                 VALUES(?, 'insight', '旧建议', '正文', 'new', 1)",
            )
            .bind(nomifun_common::generate_id())
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }

        let store = CompanionStore::open(root.path(), None).await.unwrap();
        for retired in ["companion_learn_runs", "companion_suggestions"] {
            let retired_table_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
            )
            .bind(retired)
            .fetch_one(&store.pool)
            .await
            .unwrap();
            assert_eq!(retired_table_count, 0, "{retired} must be dropped in place");
        }
        assert_eq!(
            store.get_memory(&memory_id).await.unwrap().unwrap().content,
            "保留的学习记忆"
        );
        let preserved_skill = store.get_skill(&skill_id).await.unwrap().unwrap();
        assert_eq!(preserved_skill.skill_name, "preserved-skill");
        assert_eq!(preserved_skill.status, "active");
        assert_eq!(preserved_skill.companion_id.as_deref(), Some(companion_id.as_str()));
        // Opening the store alone must not touch the install-wide rows: the
        // per-companion seeding below is what consumes them, and it needs the
        // roster, which the store layer does not have.
        for (key, expected) in [
            ("last_learn_ts", 101),
            ("learn_cursor_ts", 102),
            ("last_evolve_ts", 201),
            ("evolve_cursor_ts", 202),
        ] {
            assert_eq!(store.get_state_i64(key).await.unwrap(), expected, "state key {key}");
        }
        assert_eq!(store.get_state("mood").await.unwrap().as_deref(), Some("happy"));
        assert_eq!(store.get_companion_state_i64(&companion_id, "xp").await.unwrap(), 17);

        // The seeding copies each key onto the roster and then DELETES the
        // install-wide row: it has no reader left, and the "keep it for a
        // rollback" story is false — the same upgrade just dropped
        // companion_suggestions, which an older build refuses to open without.
        let roster = vec![companion_id.clone()];
        assert!(store.seed_companion_state_from_global(&roster).await.unwrap() > 0);
        for key in MIGRATED_GLOBAL_STATE_KEYS {
            assert_eq!(store.get_state(key).await.unwrap(), None, "global {key} must be deleted");
        }
        assert_eq!(
            store.get_companion_state_i64(&companion_id, "learn_cursor_ts").await.unwrap(),
            102,
            "the cursor must survive on the companion — a lost cursor re-distills the whole spool"
        );
        assert_eq!(
            store.get_companion_state(&companion_id, MOOD_KEY).await.unwrap().as_deref(),
            Some("happy")
        );
        // Idempotent, and it never clobbers a value the companion has moved on from.
        store.set_companion_state(&companion_id, "learn_cursor_ts", "999").await.unwrap();
        assert_eq!(store.seed_companion_state_from_global(&roster).await.unwrap(), 0);
        assert_eq!(
            store.get_companion_state_i64(&companion_id, "learn_cursor_ts").await.unwrap(),
            999
        );

        // The removal migration is idempotent and the strict baseline remains openable.
        drop(store);
        CompanionStore::open(root.path(), None).await.unwrap();
    }

    /// An empty roster must NOT delete the install-wide rows: there is nobody to
    /// copy them onto, so they are still the only record of how far the owner's
    /// loops had consumed the event spool. Deleting them here would make the first
    /// companion created afterwards re-distill the entire retained history.
    #[tokio::test]
    async fn retired_global_state_survives_a_zero_companion_install() {
        let store = CompanionStore::open_memory().await.unwrap();
        store.set_state(crate::collector::LEARN_CURSOR_KEY, "4200").await.unwrap();
        assert_eq!(store.seed_companion_state_from_global(&[]).await.unwrap(), 0);
        assert_eq!(
            store.get_state_i64(crate::collector::LEARN_CURSOR_KEY).await.unwrap(),
            4200
        );
    }

    #[tokio::test]
    async fn fts_index_rebuilds_when_out_of_sync() {
        let root = tempfile::tempdir().unwrap();
        {
            let store = CompanionStore::open(root.path(), None).await.unwrap();
            store
                .insert_memory("knowledge", "Rust 的 borrow checker 很严格", &[], 0.8, "manual")
                .await
                .unwrap();
            // Sabotage the index out-of-band (simulates a crash between the
            // main-table write and the index write).
            sqlx::query("INSERT INTO companion_memories_fts(companion_memories_fts) VALUES('delete-all')")
                .execute(&store.pool)
                .await
                .unwrap();
            assert_eq!(fts_count(&store.pool).await, 0);
        }
        let reopened = CompanionStore::open(root.path(), None).await.unwrap();
        assert_eq!(fts_count(&reopened.pool).await, 1, "boot must rebuild a count-desynced index");
        assert_eq!(fts_match_count(&reopened.pool, "borrow").await, 1);
    }

    #[tokio::test]
    async fn fts_write_paths_keep_index_in_sync() {
        let store = CompanionStore::open_memory().await.unwrap();

        // insert
        let m = store
            .insert_memory("preference", "主人喜欢深烘焙咖啡豆", &[], 0.8, "manual")
            .await
            .unwrap();
        assert_eq!(fts_count(&store.pool).await, 1);
        assert_eq!(fts_match_count(&store.pool, "深烘焙").await, 1);

        // update(content) re-indexes: old term gone, new term found
        store
            .update_memory(&m.memory_id, Some("主人现在只喝浅烘焙手冲"), None, None, &MemoryActor::AnyOwner)
            .await
            .unwrap();
        assert_eq!(fts_count(&store.pool).await, 1);
        assert_eq!(fts_match_count(&store.pool, "深烘焙").await, 0);
        assert_eq!(fts_match_count(&store.pool, "浅烘焙").await, 1);

        // archive / restore only flip status — index untouched
        store.archive_memories(std::slice::from_ref(&m.memory_id)).await.unwrap();
        assert_eq!(fts_count(&store.pool).await, 1);
        store
            .update_memory(&m.memory_id, None, None, Some("active"), &MemoryActor::AnyOwner)
            .await
            .unwrap();
        assert_eq!(fts_count(&store.pool).await, 1);

        // delete removes the index entry
        store.delete_memory(&m.memory_id, &MemoryActor::AnyOwner).await.unwrap();
        assert_eq!(fts_count(&store.pool).await, 0);

        // raw import path also indexes
        let imported = CompanionMemory {
            memory_id: CompanionMemoryId::new().into_string(),
            kind: "episode".into(),
            content: "上周和主人一起调通了流水线".into(),
            tags: vec![],
            importance: 0.7,
            strength: 0.7,
            pinned: false,
            source: "import".into(),
            status: "active".into(),
            created_at: 1,
            updated_at: 1,
            last_reinforced_at: 1,
            companion_id: None,
        };
        store.insert_memory_raw(&imported).await.unwrap();
        assert_eq!(fts_match_count(&store.pool, "流水线").await, 1);

        // delete_companion_rows drops the owner's private memories from the index
        let owner = companion_fixture(7);
        store
            .insert_memory_scoped("task", "帮主人盯 CI 构建", &[], 0.8, "chat", Some(&owner))
            .await
            .unwrap();
        assert_eq!(fts_count(&store.pool).await, 2);
        store.delete_companion_rows(&owner).await.unwrap();
        assert_eq!(fts_count(&store.pool).await, 1);
        assert_eq!(fts_match_count(&store.pool, "流水线").await, 1);
    }

    /// The ownership invariant, at the layer that can actually enforce it:
    /// companion B can neither edit, delete, batch-archive nor merge companion
    /// A's memories, and each attempt is a clean `NotFound` — never a no-op
    /// reported as success. Drop the actor check from any of the four mutators and
    /// the matching pair of assertions here flips.
    #[tokio::test]
    async fn memory_mutations_reject_another_companions_rows() {
        let store = CompanionStore::open_memory().await.unwrap();
        let a = companion_fixture(41);
        let b = companion_fixture(42);
        let actor_a = MemoryActor::Companion(a.clone());
        let actor_b = MemoryActor::Companion(b.clone());
        let own = |owner: &str, content: &str| {
            let owner = owner.to_owned();
            let content = content.to_owned();
            let store = &store;
            async move {
                store
                    .insert_memory_scoped(
                        "preference",
                        &content,
                        &[],
                        0.8,
                        "manual",
                        Some(&owner),
                    )
                    .await
                    .unwrap()
            }
        };
        let edit = own(&a, "主人喜欢深烘焙咖啡").await;
        let doomed = own(&a, "主人周三下午开周会").await;
        let batched = own(&a, "主人在学萨克斯").await;
        // Normalized-similar pair (containment) so the merge group is legal.
        let dup_one = own(&a, "主人养了一只叫豆豆的猫").await;
        let dup_two = own(&a, "主人养了一只叫豆豆的猫，很黏人").await;

        let forbidden = |error: AppError, what: &str| match error {
            AppError::NotFound(_) => {}
            other => panic!("{what} by a non-owner must be NotFound, got {other:?}"),
        };

        // ── B is refused all four ──
        forbidden(
            store
                .update_memory(&edit.memory_id, Some("篡改"), None, None, &actor_b)
                .await
                .expect_err("B must not edit A's memory"),
            "update",
        );
        assert_eq!(
            store.get_memory(&edit.memory_id).await.unwrap().unwrap().content,
            "主人喜欢深烘焙咖啡",
            "a refused edit must not have landed"
        );
        forbidden(
            store
                .delete_memory(&doomed.memory_id, &actor_b)
                .await
                .expect_err("B must not delete A's memory"),
            "delete",
        );
        assert!(
            store.get_memory(&doomed.memory_id).await.unwrap().is_some(),
            "a refused delete must not have landed"
        );
        forbidden(
            store
                .batch_update_memories(
                    std::slice::from_ref(&batched.memory_id),
                    &MemoryBatchAction::Archive,
                    &actor_b,
                )
                .await
                .expect_err("B must not batch-archive A's memory"),
            "batch archive",
        );
        assert_eq!(
            store.get_memory(&batched.memory_id).await.unwrap().unwrap().status,
            "active",
            "a refused batch must not have landed"
        );
        let group = vec![dup_one.memory_id.clone(), dup_two.memory_id.clone()];
        forbidden(
            store
                .merge_memories(&group, "主人的猫叫豆豆，很黏人", "preference", &actor_b)
                .await
                .expect_err("B must not merge A's memories"),
            "merge",
        );
        assert_eq!(
            store.count_memories("active", Some(&a)).await.unwrap(),
            5,
            "a refused merge must not have inserted a merged row"
        );

        // ── A can still do all four to its own ──
        store
            .update_memory(&edit.memory_id, Some("主人现在只喝浅烘焙"), None, None, &actor_a)
            .await
            .unwrap();
        assert_eq!(
            store.get_memory(&edit.memory_id).await.unwrap().unwrap().content,
            "主人现在只喝浅烘焙"
        );
        store.delete_memory(&doomed.memory_id, &actor_a).await.unwrap();
        assert!(store.get_memory(&doomed.memory_id).await.unwrap().is_none());
        store
            .batch_update_memories(
                std::slice::from_ref(&batched.memory_id),
                &MemoryBatchAction::Archive,
                &actor_a,
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_memory(&batched.memory_id).await.unwrap().unwrap().status,
            "archived"
        );
        let merged = store
            .merge_memories(&group, "主人的猫叫豆豆，很黏人", "preference", &actor_a)
            .await
            .unwrap();
        assert_eq!(merged.companion_id.as_deref(), Some(a.as_str()));

        // ── The administrative escape is the only cross-owner path ──
        let stranger = own(&b, "别的伙伴的私事").await;
        store
            .update_memory(&stranger.memory_id, Some("机主改的"), None, None, &MemoryActor::AnyOwner)
            .await
            .unwrap();
        assert_eq!(
            store.get_memory(&stranger.memory_id).await.unwrap().unwrap().content,
            "机主改的"
        );
    }

    /// A vestigial unowned row (`companion_id IS NULL`) is mutable by any companion,
    /// exactly as it is READABLE by any companion: it means "not yet assigned",
    /// so a companion that sees it in its own list must be able to act on it.
    /// The new dump behind the merge assistant follows the same rule.
    #[tokio::test]
    async fn unowned_rows_stay_reachable_and_scoped_dumps_match_the_read_rule() {
        let store = CompanionStore::open_memory().await.unwrap();
        let a = companion_fixture(43);
        let b = companion_fixture(44);
        let actor_a = MemoryActor::Companion(a.clone());

        let unowned = store
            .insert_memory("profile", "主人是 Rust 工程师", &[], 0.9, "learn")
            .await
            .unwrap();
        let mine = store
            .insert_memory_scoped("task", "帮主人盯 CI 构建", &[], 0.8, "chat", Some(&a))
            .await
            .unwrap();
        let theirs = store
            .insert_memory_scoped("task", "别的伙伴的私事", &[], 0.8, "chat", Some(&b))
            .await
            .unwrap();

        // The merge-assistant feed carries only what A can read — never another
        // companion's memory CONTENT.
        let visible: Vec<String> = store
            .dump_active_memories_visible_to(&a)
            .await
            .unwrap()
            .into_iter()
            .map(|memory| memory.memory_id)
            .collect();
        assert!(visible.contains(&unowned.memory_id) && visible.contains(&mine.memory_id));
        assert!(
            !visible.contains(&theirs.memory_id),
            "another companion's memory must not be dumped: {visible:?}"
        );

        // Archived rows are not mergeable, so they are not in the feed either.
        store
            .batch_update_memories(
                std::slice::from_ref(&mine.memory_id),
                &MemoryBatchAction::Archive,
                &actor_a,
            )
            .await
            .unwrap();
        let visible: Vec<String> = store
            .dump_active_memories_visible_to(&a)
            .await
            .unwrap()
            .into_iter()
            .map(|memory| memory.memory_id)
            .collect();
        assert_eq!(visible, vec![unowned.memory_id.clone()]);

        // And an unowned row is still mutable by whoever can see it.
        store
            .update_memory(&unowned.memory_id, None, Some(true), None, &actor_a)
            .await
            .unwrap();
        assert!(store.get_memory(&unowned.memory_id).await.unwrap().unwrap().pinned);
        store.delete_memory(&unowned.memory_id, &actor_a).await.unwrap();
        assert!(store.get_memory(&unowned.memory_id).await.unwrap().is_none());
    }

    /// Regression net for the re-homing migration: an UNOWNED (`companion_id IS NULL`)
    /// row — the shape every learner-written memory had before this change —
    /// must still reach `memories_for_injection` for an arbitrary companion,
    /// while another companion's owned row must not. Collapsing the visibility
    /// predicate to `companion_id = ?` while such rows exist would stop
    /// injecting them silently, with no error and no other failing test.
    #[tokio::test]
    async fn injection_sees_unowned_rows_and_never_another_companions() {
        let store = CompanionStore::open_memory().await.unwrap();
        let reader = companion_fixture(31);
        let stranger = companion_fixture(32);

        store
            .insert_memory("profile", "主人是 Rust 工程师", &[], 0.9, "learn")
            .await
            .unwrap();
        store
            .insert_memory_scoped(
                "task",
                "帮主人盯 CI 构建",
                &[],
                0.8,
                "chat",
                Some(&reader),
            )
            .await
            .unwrap();
        store
            .insert_memory_scoped(
                "task",
                "别的伙伴的私事",
                &[],
                0.8,
                "chat",
                Some(&stranger),
            )
            .await
            .unwrap();

        let injected = store.memories_for_injection(&reader, 10, 10_000).await.unwrap();
        let contents: Vec<&str> = injected.iter().map(|m| m.content.as_str()).collect();
        assert!(
            contents.contains(&"主人是 Rust 工程师"),
            "an unowned legacy row must still be injected: {contents:?}"
        );
        assert!(contents.contains(&"帮主人盯 CI 构建"), "own memories inject: {contents:?}");
        assert!(
            !contents.contains(&"别的伙伴的私事"),
            "another companion's memory must never be injected: {contents:?}"
        );
    }

    #[tokio::test]
    async fn memory_import_transaction_indexes_fts() {
        let store = CompanionStore::open_memory().await.unwrap();
        let imported = CompanionMemory {
            memory_id: CompanionMemoryId::new().into_string(),
            kind: "knowledge".into(),
            content: "跨机导入的记忆也要能检索".into(),
            tags: vec![],
            importance: 0.6,
            strength: 0.6,
            pinned: false,
            source: "import".into(),
            status: "active".into(),
            created_at: 1,
            updated_at: 1,
            last_reinforced_at: 1,
            companion_id: None,
        };
        let tx = store.begin_memory_import(std::slice::from_ref(&imported)).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(fts_match_count(&store.pool, "跨机导入").await, 1);
    }

    #[tokio::test]
    async fn thread_and_session_window_roundtrip() {
        let store = CompanionStore::open_memory().await.unwrap();
        let companion = companion_fixture(1);
        let conversation = conversation_fixture(1);
        let thread = store
            .insert_companion_thread(&conversation, &companion, "伙伴会话")
            .await
            .unwrap();
        assert_eq!(thread.conversation_id, conversation);

        let window = store.ensure_open_window(&companion, &conversation, 0).await.unwrap();
        store.touch_window(&window.session_window_id, 10, 2).await.unwrap();
        store.close_window(&window.session_window_id, "archived", Some("摘要"), None, 8).await.unwrap();
        let digests = store.list_digests(&companion, 10).await.unwrap();
        assert_eq!(digests.len(), 1);
        assert_eq!(digests[0].session_window_id, window.session_window_id);
    }
}
