//! Chat analytics over the local macOS Messages database (`chat.db`),
//! mirroring the Codex Messages MCP tools `find_chats` /
//! `count_message_activity` (spec §6.2/§6.3).
//!
//! Posture:
//! - The database is opened **read-only** (`read_only(true)`,
//!   `create_if_missing(false)`); this module never writes and never creates
//!   files. The default location is `~/Library/Messages/chat.db` (macOS only).
//! - Full Disk Access is required by macOS to read that path. A permission
//!   failure is surfaced as [`ChatAnalyticsError::PermissionDenied`] with the
//!   guidance string — it is NEVER collapsed into "no data".
//! - The spec's custom SQLite functions (`extract_message`,
//!   `message_activity_bucket_index`) cannot be registered on a read-only
//!   sqlx handle, so bucketing is done Rust-side (chrono, local time;
//!   weeks begin Monday) and `attributedBody` is parsed by a minimal
//!   typedstream reader (see [`extract_message_from_attributed_body`]).
//!   The reader is heuristic: if the blob does not parse, message text
//!   degrades to `message.text` or is omitted — aggregation (counts) is
//!   unaffected and never crashes on unparseable blobs.
//!
//! Privacy: this module returns chat names / counts always, and message text
//! only when the caller passes `include_text = true`; any text leaving this
//! module goes through [`redact_message_text`] (nomi-redact). Message text is
//! never logged — only counts.
//!
//! Service hookup point: `ComputerHistoryService` should hold an
//! `Option<ChatAnalytics>` (None on non-macOS or when the db is missing),
//! surface availability in `status()`, and forward `find_chats` /
//! `count_message_activity` to the agent sink's `chat_activity` / `find_chats`
//! trait methods. (Owned by worker-observer's wiring; not edited here.)

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, TimeZone, Timelike, Utc};
use nomifun_common::AppError;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Pool, Row, Sqlite};

/// Core Data epoch offset (seconds) used by `message.date` nanoseconds.
const CORE_DATA_EPOCH_SECS: i64 = 978_307_200;
const NANOS_PER_SEC: i64 = 1_000_000_000;

/// Cursor pagination page size for ranked chat breakdowns (spec default).
const DEFAULT_CHAT_PAGE_LIMIT: usize = 20;
const MAX_CHAT_PAGE_LIMIT: usize = 100;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Distinguishable failure modes for chat.db access. Mapping to `AppError`
/// keeps the service facade stable while preserving the cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatAnalyticsErrorKind {
    /// macOS Full Disk Access has not been granted for this app.
    PermissionDenied,
    /// Not macOS, or the database does not exist at the expected path.
    Unavailable,
    /// The database exists but a query failed.
    Query,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub struct ChatAnalyticsError {
    pub kind: ChatAnalyticsErrorKind,
    pub message: String,
    /// User-facing guidance (shown for `permission_denied`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
}

impl std::fmt::Display for ChatAnalyticsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ChatAnalyticsError {}

const FULL_DISK_ACCESS_GUIDANCE: &str =
    "Full Disk Access \u{2014} Allows searching your messages when asked. \
     Enable it in System Settings > Privacy & Security > Full Disk Access.";

impl ChatAnalyticsError {
    pub fn permission_denied() -> Self {
        Self {
            kind: ChatAnalyticsErrorKind::PermissionDenied,
            message: "Required Messages permissions were not granted.".to_string(),
            guidance: Some(FULL_DISK_ACCESS_GUIDANCE.to_string()),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: ChatAnalyticsErrorKind::Unavailable,
            message: message.into(),
            guidance: None,
        }
    }

    pub fn query(message: impl Into<String>) -> Self {
        Self {
            kind: ChatAnalyticsErrorKind::Query,
            message: message.into(),
            guidance: None,
        }
    }

    pub fn kind(&self) -> &ChatAnalyticsErrorKind {
        &self.kind
    }
}

impl From<ChatAnalyticsError> for AppError {
    fn from(e: ChatAnalyticsError) -> Self {
        AppError::Internal(format!("chat analytics: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Request / response model (spec §6.1/§6.2)
// ---------------------------------------------------------------------------

/// Calendar bucketing interval. `Total` yields one bucket for a nonempty
/// range; `Week` buckets begin Monday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityInterval {
    #[default]
    Total,
    Day,
    Week,
    Hour,
}

/// Whether a request aggregates across all matching chats or ranks per chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityBreakdown {
    #[default]
    Overall,
    Chat,
}

/// Rank chats by total, sent, or received count (default total).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityRank {
    #[default]
    Total,
    Sent,
    Received,
}

/// Optional chat type filter; omit to include both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatType {
    Direct,
    Group,
}

/// `{ total, sent, received }`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityCount {
    pub total: i64,
    pub sent: i64,
    pub received: i64,
}

/// Half-open bucket `[start, end)` in epoch milliseconds, ordered and
/// index-aligned with `counts_by_bucket` (including empty intervals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityBucket {
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CountActivityRequest {
    /// ISO-8601 lower bound (inclusive). Omit to begin at oldest activity.
    pub from: Option<String>,
    /// ISO-8601 upper bound (exclusive). Omit to end at latest activity.
    pub to: Option<String>,
    pub interval: Option<ActivityInterval>,
    pub chat_type: Option<ChatType>,
    /// Restrict to these chats (guids as returned by `find_chats`).
    pub chat_guids: Option<Vec<String>>,
    pub breakdown: Option<ActivityBreakdown>,
    pub rank_by: Option<ActivityRank>,
    /// Page size for chat breakdowns (default 20).
    pub limit: Option<u32>,
    /// Opaque continuation from `next_cursor`.
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedChatActivity {
    pub chat_guid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub activity: ActivityCount,
}

#[derive(Debug, Clone, Serialize)]
pub struct CountActivityResult {
    /// Aggregate across every matching chat in the complete filtered range.
    pub overall_activity: ActivityCount,
    /// Ranked page of matching chats (breakdown = chat only).
    pub chats: Vec<RankedChatActivity>,
    /// Shared ordered half-open intervals, index-aligned with counts.
    pub buckets: Vec<ActivityBucket>,
    /// Index-aligned with `buckets`, including intervals with no activity.
    pub counts_by_bucket: ActivityBucketCounts,
    /// Resolved half-open range `[from_ms, to_ms)` (frozen across pages).
    pub range_start_ms: i64,
    pub range_end_ms: i64,
    pub interval: ActivityInterval,
    /// Exact number of chats with counted activity in the full range.
    pub matching_chat_count: i64,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivityBucketCounts {
    pub total: Vec<i64>,
    pub sent: Vec<i64>,
    pub received: Vec<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FindChatsRequest {
    /// Names, phone numbers, or email addresses that must all participate.
    pub participants: Option<Vec<String>>,
    /// When true, matches only chats containing exactly those participants.
    pub exact_participants: Option<bool>,
    /// Exact or partial chat name.
    pub chat_name: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// When true, return only chats with unread messages.
    pub unread_only: Option<bool>,
    /// Page size (default 20).
    pub limit: Option<u32>,
    /// Opaque continuation from `next_cursor`.
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatSummary {
    pub chat_guid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub service_name: Option<String>,
    pub participants: Vec<String>,
    /// Last message date in the filtered range, epoch ms (None if none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_ms: Option<i64>,
    pub unread_count: i64,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindChatsResult {
    pub chats: Vec<ChatSummary>,
    /// Exact number of matching chats in the complete filtered range.
    pub matching_chat_count: i64,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Access
// ---------------------------------------------------------------------------

/// Read-only handle over the local Messages database.
#[derive(Debug)]
pub struct ChatAnalytics {
    pool: Pool<Sqlite>,
    /// Resolved database path (exposed via [`ChatAnalytics::db_path`]).
    path: PathBuf,
}

impl ChatAnalytics {
    /// Open `~/Library/Messages/chat.db` read-only (macOS only). On other
    /// platforms this returns [`ChatAnalyticsError::Unavailable`]. A TCC /
    /// Full Disk Access failure maps to
    /// [`ChatAnalyticsError::PermissionDenied`] — never to empty data.
    pub async fn open_default() -> Result<Self, ChatAnalyticsError> {
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| ChatAnalyticsError::unavailable("HOME is not set"))?;
            let path = home.join("Library/Messages/chat.db");
            Self::open_at(&path).await
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(ChatAnalyticsError::unavailable(
                "chat.db analytics is only available on macOS",
            ))
        }
    }

    /// Open a chat.db at an explicit path, read-only. Used by tests and by
    /// callers that already resolved a snapshot path. Never creates a file.
    pub async fn open_at(path: &Path) -> Result<Self, ChatAnalyticsError> {
        if !path.exists() {
            return Err(ChatAnalyticsError::unavailable(format!(
                "Messages database not found at {}",
                path.display()
            )));
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect_with(options)
            .await
            .map_err(|e| classify_open_error(&e))?;
        // Probe with a real read: TCC denial can surface on first query even
        // when the open succeeds, and we must distinguish it from empty data.
        sqlx::query("SELECT COUNT(*) FROM sqlite_master")
            .fetch_one(&pool)
            .await
            .map_err(|e| classify_open_error(&e))?;
        Ok(Self {
            pool,
            path: path.to_path_buf(),
        })
    }

    /// Resolved database path (status/debug surface only; never logged).
    pub fn db_path(&self) -> &Path {
        &self.path
    }

    pub async fn status(&self) -> ChatAnalyticsStatus {
        ChatAnalyticsStatus {
            available: true,
            db_path: self.path.display().to_string(),
        }
    }

    fn err(&self, e: sqlx::Error) -> ChatAnalyticsError {
        ChatAnalyticsError::query(format!("Messages database query failed: {e}"))
    }

    // -- find_chats ----------------------------------------------------------

    /// Find chats by participant, chat name, date range, or unread status.
    /// When multiple participants are provided, every participant must belong
    /// to the chat. See [`FindChatsRequest`] for the argument semantics.
    pub async fn find_chats(
        &self,
        request: &FindChatsRequest,
    ) -> Result<FindChatsResult, ChatAnalyticsError> {
        let offset = decode_cursor(request.cursor.as_deref())?;
        let limit = clamp_limit(request.limit)?;
        let from_ns = parse_bound(request.from.as_deref())?;
        let to_ns = parse_bound(request.to.as_deref())?;

        let rows = sqlx::query(
            r#"
            SELECT chat.ROWID AS chat_row_id,
                   chat.guid AS chat_guid,
                   chat.display_name AS display_name,
                   chat.service_name AS service_name,
                   MAX(message.date) AS last_message_ns,
                   COALESCE(SUM(CASE WHEN message.is_read = 0
                                      AND message.is_from_me = 0
                                      AND message.is_finished = 1
                                      AND message.is_system_message = 0
                                THEN 1 ELSE 0 END), 0) AS unread_count,
                   COUNT(message.ROWID) AS message_count
            FROM chat chat
            JOIN chat_message_join cmj ON cmj.chat_id = chat.ROWID
            JOIN message ON message.ROWID = cmj.message_id
            WHERE (?1 IS NULL OR message.date >= ?1)
              AND (?2 IS NULL OR message.date < ?2)
              AND (?3 IS NULL
                   OR (chat.display_name IS NOT NULL
                       AND instr(lower(chat.display_name), lower(?3)) > 0))
            GROUP BY chat.ROWID
            HAVING (?4 = 0 OR unread_count > 0)
            ORDER BY last_message_ns DESC, chat.ROWID DESC
            "#,
        )
        .bind(from_ns)
        .bind(to_ns)
        .bind(request.chat_name.clone())
        .bind(i64::from(request.unread_only.unwrap_or(false)))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.err(e))?;

        let mut chats = Vec::with_capacity(rows.len());
        for row in rows {
            let row_id: i64 = row.try_get("chat_row_id").map_err(|e| self.err(e))?;
            let chat_guid: String = row.try_get("chat_guid").map_err(|e| self.err(e))?;
            let display_name: Option<String> =
                row.try_get("display_name").map_err(|e| self.err(e))?;
            let service_name: Option<String> =
                row.try_get("service_name").map_err(|e| self.err(e))?;
            let last_ns: Option<i64> =
                row.try_get("last_message_ns").map_err(|e| self.err(e))?;
            let unread_count: i64 = row.try_get("unread_count").map_err(|e| self.err(e))?;
            let message_count: i64 = row.try_get("message_count").map_err(|e| self.err(e))?;
            let entry = ChatSummary {
                chat_guid,
                display_name: display_name.filter(|n| !n.trim().is_empty()),
                service_name,
                participants: self.chat_participants(row_id).await?,
                last_message_ms: last_ns.map(core_data_ns_to_unix_ms),
                unread_count,
                message_count,
            };
            chats.push(entry);
        }

        // Participant matching (all requested participants must belong; only
        // chats with exactly those participants when exact_participants is
        // set) is applied Rust-side against the resolved participant list.
        if let Some(wanted) = requested_participants(request) {
            let exact = exact_enabled(request);
            chats.retain(|chat| {
                let lowered: Vec<String> =
                    chat.participants.iter().map(|p| p.to_lowercase()).collect();
                // Non-exact: every wanted participant belongs (extras allowed).
                // Exact: the chat's participant set is exactly the wanted set.
                let all_present = wanted.iter().all(|w| lowered.iter().any(|m| m == w));
                all_present && (!exact || lowered.len() == wanted.len())
            });
        }

        let matching = chats.len() as i64;
        let end = (offset + limit).min(chats.len());
        let page: Vec<ChatSummary> = chats[offset.min(chats.len())..end].to_vec();
        let has_more = end < chats.len();
        let next_cursor = if has_more {
            Some(encode_cursor(end))
        } else {
            None
        };
        Ok(FindChatsResult {
            chats: page,
            matching_chat_count: matching,
            has_more,
            next_cursor,
        })
    }

    async fn chat_participants(&self, chat_row_id: i64) -> Result<Vec<String>, ChatAnalyticsError> {
        let rows = sqlx::query(
            r#"
            SELECT COALESCE(handle.uncanonicalized_id, handle.id) AS participant
            FROM chat_handle_join chj
            JOIN handle ON handle.ROWID = chj.handle_id
            WHERE chj.chat_id = ?
            ORDER BY handle.ROWID
            "#,
        )
        .bind(chat_row_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.err(e))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.try_get::<String, _>("participant").ok())
            .collect())
    }

    // -- count_message_activity ----------------------------------------------

    /// Count Messages activity over time, overall or ranked per chat. Weeks
    /// begin Monday; buckets are half-open and index-aligned with
    /// `counts_by_bucket` (empty intervals included). See
    /// [`CountActivityRequest`] for argument semantics.
    pub async fn count_message_activity(
        &self,
        request: &CountActivityRequest,
    ) -> Result<CountActivityResult, ChatAnalyticsError> {
        let breakdown = request.breakdown.unwrap_or_default();
        let interval = request.interval.unwrap_or_default();
        if breakdown == ActivityBreakdown::Overall
            && (request.rank_by.is_some() || request.limit.is_some() || request.cursor.is_some())
        {
            return Err(ChatAnalyticsError::query(
                "cursor/limit/rank_by can only be used when breakdown is chat",
            ));
        }
        let offset = decode_cursor(request.cursor.as_deref())?;
        let limit = clamp_limit(request.limit)?;
        let rank_by = request.rank_by.unwrap_or_default();
        let from_ns = parse_bound(request.from.as_deref())?;
        let to_ns = parse_bound(request.to.as_deref())?;

        // Frozen half-open range: explicit bounds, otherwise min/max activity.
        let (range_start_ns, range_end_ns) = self
            .resolve_range(from_ns, to_ns)
            .await?
            .ok_or_else(|| {
                ChatAnalyticsError::query("no activity matches the given filters")
            })?;
        let range_start_ms = core_data_ns_to_unix_ms(range_start_ns);
        let range_end_ms = core_data_ns_to_unix_ms(range_end_ns);
        let buckets = build_buckets(range_start_ms, range_end_ms, interval);

        // Aggregation core: one row per (chat, message) inside the range,
        // sent/received and unread computed Rust-side (spec's SQLite UDFs are
        // not registrable on a read-only handle).
        let rows = sqlx::query(
            r#"
            SELECT chat.ROWID AS chat_row_id,
                   chat.guid AS chat_guid,
                   chat.display_name AS display_name,
                   message.date AS date_ns,
                   message.is_from_me AS is_from_me
            FROM chat chat
            JOIN chat_message_join cmj ON cmj.chat_id = chat.ROWID
            JOIN message ON message.ROWID = cmj.message_id
            WHERE message.date >= ?1
              AND message.date < ?2
              AND (?3 = 1 OR chat.guid IN (SELECT value FROM json_each(?4)))
              AND (?5 IS NULL
                   OR (?5 = 'direct' AND (chat.ROWID IN (
                        SELECT chj.chat_id FROM chat_handle_join chj GROUP BY chj.chat_id
                          HAVING COUNT(*) = 1)))
                   OR (?5 = 'group' AND chat.ROWID NOT IN (
                        SELECT chj.chat_id FROM chat_handle_join chj GROUP BY chj.chat_id
                          HAVING COUNT(*) = 1)))
            ORDER BY chat.ROWID, message.date
            "#,
        )
        .bind(range_start_ns)
        .bind(range_end_ns)
        .bind(i64::from(request.chat_guids.as_deref().unwrap_or(&[]).is_empty()))
        .bind(
            serde_json::to_string(request.chat_guids.as_deref().unwrap_or(&[])).unwrap_or_default(),
        )
        .bind(request.chat_type.map(|c| c.to_string()))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.err(e))?;

        // (chat_row_id, bucket_index) -> (total, sent, received)
        let mut per_chat: std::collections::BTreeMap<i64, RankedChatActivity> =
            std::collections::BTreeMap::new();
        let mut overall = vec![ActivityCount::default(); buckets.len()];
        for row in rows {
            let row_id: i64 = row.try_get("chat_row_id").map_err(|e| self.err(e))?;
            let chat_guid: String = row.try_get("chat_guid").map_err(|e| self.err(e))?;
            let display_name: Option<String> =
                row.try_get("display_name").map_err(|e| self.err(e))?;
            let date_ns: i64 = row.try_get("date_ns").map_err(|e| self.err(e))?;
            let is_from_me: i64 = row.try_get("is_from_me").map_err(|e| self.err(e))?;
            let date_ms = core_data_ns_to_unix_ms(date_ns);
            let index = bucket_index(&buckets, date_ms).ok_or_else(|| {
                ChatAnalyticsError::query("activity bucket is outside the supported range")
            })?;
            let sent = is_from_me != 0;
            let entry = overall[index];
            overall[index] = ActivityCount {
                total: entry.total + 1,
                sent: entry.sent + i64::from(sent),
                received: entry.received + i64::from(!sent),
            };
            let chat = per_chat.entry(row_id).or_insert_with(|| RankedChatActivity {
                chat_guid: chat_guid.clone(),
                display_name: display_name.filter(|n| !n.trim().is_empty()),
                activity: ActivityCount::default(),
            });
            chat.activity.total += 1;
            chat.activity.sent += i64::from(sent);
            chat.activity.received += i64::from(!sent);
        }

        let total_overall = overall
            .iter()
            .fold(ActivityCount::default(), |acc, c| ActivityCount {
                total: acc.total + c.total,
                sent: acc.sent + c.sent,
                received: acc.received + c.received,
            });

        let mut ranked: Vec<RankedChatActivity> = per_chat.into_values().collect();
        ranked.sort_by(|a, b| {
            let key = |c: &RankedChatActivity| match rank_by {
                ActivityRank::Total => c.activity.total,
                ActivityRank::Sent => c.activity.sent,
                ActivityRank::Received => c.activity.received,
            };
            key(b).cmp(&key(a)).then_with(|| b.chat_guid.cmp(&a.chat_guid))
        });
        let matching_chat_count = ranked.len() as i64;

        let mut page_chats = Vec::new();
        let mut has_more = false;
        let mut next_cursor = None;
        if breakdown == ActivityBreakdown::Chat {
            let end = (offset + limit).min(ranked.len());
            page_chats = ranked[offset.min(ranked.len())..end].to_vec();
            has_more = end < ranked.len();
            if has_more {
                next_cursor = Some(encode_cursor(end));
            }
        }

        let counts_by_bucket = ActivityBucketCounts {
            total: overall.iter().map(|c| c.total).collect(),
            sent: overall.iter().map(|c| c.sent).collect(),
            received: overall.iter().map(|c| c.received).collect(),
        };

        Ok(CountActivityResult {
            overall_activity: total_overall,
            chats: page_chats,
            buckets,
            counts_by_bucket,
            range_start_ms,
            range_end_ms,
            interval,
            matching_chat_count,
            has_more,
            next_cursor,
        })
    }

    /// Oldest and newest matching activity nanoseconds, honoring explicit
    /// bounds and chat filters. Returns `None` when nothing matches.
    async fn resolve_range(
        &self,
        from_ns: Option<i64>,
        to_ns: Option<i64>,
    ) -> Result<Option<(i64, i64)>, ChatAnalyticsError> {
        if let (Some(from), Some(to)) = (from_ns, to_ns) {
            if from >= to {
                return Err(ChatAnalyticsError::query(
                    "interval start must be earlier than interval end",
                ));
            }
        }
        let row = sqlx::query(
            r#"
            SELECT MIN(message.date) AS min_ns, MAX(message.date) AS max_ns
            FROM chat chat
            JOIN chat_message_join cmj ON cmj.chat_id = chat.ROWID
            JOIN message ON message.ROWID = cmj.message_id
            WHERE message.date >= ?1
              AND message.date < ?2
            "#,
        )
        .bind(from_ns.unwrap_or(0))
        .bind(to_ns.unwrap_or(i64::MAX))
        .fetch_one(&self.pool)
        .await
        .map_err(|e| self.err(e))?;
        let min_ns: Option<i64> = row.try_get("min_ns").map_err(|e| self.err(e))?;
        let max_ns: Option<i64> = row.try_get("max_ns").map_err(|e| self.err(e))?;
        Ok(min_ns.zip(max_ns).map(|(lo, hi)| {
            (
                from_ns.unwrap_or(lo),
                to_ns.unwrap_or_else(|| hi + 1), // exclusive upper bound
            )
        }))
    }

    /// Message text for one chat (read-only). Text only leaves the module when
    /// the caller asks for it, and always passes through redaction.
    pub async fn read_message_texts(
        &self,
        chat_guid: &str,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        limit: u32,
    ) -> Result<Vec<String>, ChatAnalyticsError> {
        let rows = sqlx::query(
            r#"
            SELECT message.text AS text,
                   message.attributedBody AS attributed_body
            FROM chat chat
            JOIN chat_message_join cmj ON cmj.chat_id = chat.ROWID
            JOIN message ON message.ROWID = cmj.message_id
            WHERE chat.guid = ?1
              AND message.is_system_message = 0
              AND message.is_finished = 1
              AND (message.associated_message_type IS NULL
                   OR message.associated_message_type = 0)
              AND (?2 IS NULL OR message.date >= ?2)
              AND (?3 IS NULL OR message.date < ?3)
            ORDER BY message.date DESC
            LIMIT ?4
            "#,
        )
        .bind(chat_guid)
        .bind(from_ms.map(unix_ms_to_core_data_ns))
        .bind(to_ms.map(unix_ms_to_core_data_ns))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.err(e))?;
        let mut out = Vec::new();
        for row in rows {
            let text: Option<String> = row.try_get("text").ok().flatten();
            let body: Option<Vec<u8>> = row.try_get("attributed_body").ok().flatten();
            let message_text = text
                .filter(|t| !t.is_empty())
                .or_else(|| body.as_deref().and_then(extract_message_from_attributed_body));
            if let Some(t) = message_text {
                out.push(redact_message_text(&t));
            }
        }
        Ok(out)
    }
}

/// Status surface for `ComputerHistoryService::status()` integration.
#[derive(Debug, Clone, Serialize)]
pub struct ChatAnalyticsStatus {
    pub available: bool,
    pub db_path: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map an open/probe failure to permission-vs-unavailable. TCC denial of
/// `~/Library/Messages` surfaces as EPERM/EACCES or "unable to open database
/// file"; it must stay distinguishable from "db missing".
fn classify_open_error(e: &sqlx::Error) -> ChatAnalyticsError {
    let text = e.to_string();
    let permission = matches!(e, sqlx::Error::Io(io) if {
        matches!(
            io.kind(),
            std::io::ErrorKind::PermissionDenied
        )
    }) || text.contains("unable to open database file")
        || text.contains("Access to the database file is not allowed")
        || text.to_lowercase().contains("permission denied")
        || text.to_lowercase().contains("authorization denied");
    if permission {
        ChatAnalyticsError::permission_denied()
    } else {
        ChatAnalyticsError::unavailable(format!("could not open Messages database: {e}"))
    }
}

fn core_data_ns_to_unix_ms(ns: i64) -> i64 {
    ns.div_euclid(NANOS_PER_SEC) * 1000 + CORE_DATA_EPOCH_SECS * 1000
}

fn unix_ms_to_core_data_ns(ms: i64) -> i64 {
    ((ms / 1000) - CORE_DATA_EPOCH_SECS).saturating_mul(NANOS_PER_SEC)
}

/// ISO-8601 date-time bound -> Core Data nanoseconds. Invalid input is an
/// explicit error (never silently widened).
fn parse_bound(value: Option<&str>) -> Result<Option<i64>, ChatAnalyticsError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let dt = DateTime::parse_from_rfc3339(value).map_err(|_| {
        ChatAnalyticsError::query(format!(
            "{value} must be an ISO-8601 date-time, such as 2026-07-08T12:00:00Z"
        ))
    })?;
    Ok(Some(unix_ms_to_core_data_ns(dt.with_timezone(&Utc).timestamp_millis())))
}

fn clamp_limit(limit: Option<u32>) -> Result<usize, ChatAnalyticsError> {
    match limit.unwrap_or(DEFAULT_CHAT_PAGE_LIMIT as u32) {
        0 => Err(ChatAnalyticsError::query("limit must be at least 1")),
        n => Ok((n as usize).min(MAX_CHAT_PAGE_LIMIT)),
    }
}

/// Whether exact-participant matching is requested.
fn exact_enabled(request: &FindChatsRequest) -> bool {
    request.exact_participants.unwrap_or(false)
}

/// Lowercased, trimmed participant filter, if any were requested.
fn requested_participants(request: &FindChatsRequest) -> Option<Vec<String>> {
    match request.participants.as_deref() {
        Some(list) if !list.is_empty() => Some(
            list.iter()
                .map(|p| p.trim().to_lowercase())
                .filter(|p| !p.is_empty())
                .collect(),
        ),
        _ => None,
    }
}

/// Opaque cursor: `ch` prefix + hex-encoded page offset. Malformed input gets
/// the spec's exact error string so callers can recover with `next_cursor`.
const CURSOR_MALFORMED: &str =
    "cursor is malformed; use next_cursor returned by count_message_activity";

fn decode_cursor(cursor: Option<&str>) -> Result<usize, ChatAnalyticsError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let hex = cursor
        .strip_prefix("ch")
        .ok_or_else(|| ChatAnalyticsError::query(CURSOR_MALFORMED))?;
    let offset = usize::from_str_radix(hex, 16)
        .map_err(|_| ChatAnalyticsError::query(CURSOR_MALFORMED))?;
    Ok(offset)
}

fn encode_cursor(offset: usize) -> String {
    format!("ch{offset:x}")
}

/// Shared ordered half-open buckets covering `[start_ms, end_ms)` at the given
/// interval, in local time (weeks begin Monday). `Total` yields one bucket.
fn build_buckets(start_ms: i64, end_ms: i64, interval: ActivityInterval) -> Vec<ActivityBucket> {
    if interval == ActivityInterval::Total {
        return vec![ActivityBucket {
            start_ms,
            end_ms,
        }];
    }
    let start_local = millis_to_local(start_ms);
    let mut edges = Vec::new();
    let mut cursor = match interval {
        ActivityInterval::Day => start_local
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| dt.and_local_timezone(Local).single()),
        ActivityInterval::Week => {
            let date = start_local.date_naive();
            let days_since_monday = date.weekday().num_days_from_monday() as i64;
            (date - ChronoDuration::days(days_since_monday))
                .and_hms_opt(0, 0, 0)
                .map(|dt| dt.and_local_timezone(Local).single())
        }
        ActivityInterval::Hour => start_local
            .date_naive()
            .and_hms_opt(start_local.hour(), 0, 0)
            .map(|dt| dt.and_local_timezone(Local).single()),
        ActivityInterval::Total => unreachable!(),
    }
    .flatten()
    .unwrap_or(start_local);
    let step = match interval {
        ActivityInterval::Hour => ChronoDuration::hours(1),
        ActivityInterval::Day => ChronoDuration::days(1),
        ActivityInterval::Week => ChronoDuration::weeks(1),
        ActivityInterval::Total => unreachable!(),
    };
    let end_local = millis_to_local(end_ms);
    while cursor < end_local {
        let next = cursor + step;
        let bucket_end = if next > end_local { end_local } else { next };
        edges.push(ActivityBucket {
            start_ms: cursor.with_timezone(&Utc).timestamp_millis(),
            end_ms: bucket_end.with_timezone(&Utc).timestamp_millis(),
        });
        cursor = next;
    }
    if edges.is_empty() {
        edges.push(ActivityBucket {
            start_ms,
            end_ms,
        });
    }
    edges
}

fn millis_to_local(ms: i64) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(|| Utc.timestamp_millis_opt(ms).single().unwrap().with_timezone(&Local))
}

/// Half-open lookup: the containing bucket is the first whose `end_ms` is
/// strictly greater than `ms` (and whose `start_ms` is <= ms).
fn bucket_index(buckets: &[ActivityBucket], ms: i64) -> Option<usize> {
    let index = buckets.partition_point(|b| b.end_ms <= ms);
    if index >= buckets.len() || buckets[index].start_ms > ms {
        None
    } else {
        Some(index)
    }
}

/// Redaction helper for any message text leaving this module.
fn redact_message_text(text: &str) -> String {
    crate::rules::redact_captured_text(text)
}

/// Minimal Apple typedstream reader for `message.attributedBody`.
///
/// Full typedstream (NSKeyedArchiver-flavored) decoding is out of scope; the
/// format used by chat.db stores the message text as a run of
/// length-prefixed UTF-8 chunks after an 8-byte stream header
/// (`04 0B B0 .. 81 <len> <utf-8 bytes> ...`). We scan for those
/// length-prefixed string runs and return the longest plausible text run
/// (longest, because attribute dictionaries may contain short keys like the
/// sender name). Unparseable or empty blobs yield `None` — callers degrade to
/// `message.text`, and aggregation never depends on this succeeding.
pub fn extract_message_from_attributed_body(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 8 {
        return None;
    }
    let mut best: Option<String> = None;
    let mut i = 8; // skip the typedstream header
    while i < bytes.len() {
        let byte = bytes[i];
        if byte != 0x81 {
            i += 1;
            continue;
        }
        // 0x81 marks a string whose length follows as one byte (multi-byte
        // lengths also exist; fall back to a 4-byte big-endian attempt).
        let (len, next) = if let Some(&l) = bytes.get(i + 1) {
            (l as usize, i + 2)
        } else {
            break;
        };
        if len == 0 {
            i = next;
            continue;
        }
        let end = next.checked_add(len)?;
        let chunk = bytes.get(next..end)?;
        if let Ok(text) = std::str::from_utf8(chunk) {
            let printable = text
                .chars()
                .all(|c| !c.is_control() || c == '\n' || c == '\t')
                && text.chars().any(|c| !c.is_whitespace());
            if printable {
                let better = match &best {
                    Some(current) => text.chars().count() > current.chars().count(),
                    None => true,
                };
                if better {
                    best = Some(text.to_string());
                }
            }
        }
        i = end;
    }
    best
}

impl std::fmt::Display for ChatType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ChatType::Direct => "direct",
            ChatType::Group => "group",
        })
    }
}

// ---------------------------------------------------------------------------
// Tests (hermetic: temp chat.db only, never ~/Library/Messages)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteJournalMode;

    /// message.guid is UNIQUE in the real schema; message rows repeat
    /// (chat, date, text) in some tests, so disambiguate with a counter.
    static MESSAGE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn uuid_like_suffix() -> u64 {
        MESSAGE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    struct TestDb {
        analytics: ChatAnalytics,
        pool: Pool<Sqlite>,
        _dir: tempfile::TempDir,
        path: PathBuf,
    }

    async fn build_test_db() -> TestDb {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.db");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query(
            r#"
            CREATE TABLE message (
              ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
              guid TEXT NOT NULL UNIQUE,
              text TEXT,
              attributedBody BLOB,
              handle_id INTEGER DEFAULT 0,
              subject TEXT,
              date INTEGER NOT NULL,
              is_from_me INTEGER NOT NULL DEFAULT 0,
              is_read INTEGER NOT NULL DEFAULT 1,
              is_finished INTEGER NOT NULL DEFAULT 1,
              is_system_message INTEGER NOT NULL DEFAULT 0,
              associated_message_type INTEGER
            );
            CREATE TABLE chat (
              ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
              guid TEXT NOT NULL UNIQUE,
              display_name TEXT,
              service_name TEXT
            );
            CREATE TABLE handle (
              ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
              id TEXT,
              uncanonicalized_id TEXT
            );
            CREATE TABLE chat_handle_join (chat_id INTEGER NOT NULL, handle_id INTEGER NOT NULL);
            CREATE TABLE chat_message_join (chat_id INTEGER NOT NULL, message_id INTEGER NOT NULL);
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        let analytics = ChatAnalytics::open_at(&path).await.unwrap();
        TestDb {
            analytics,
            pool,
            _dir: dir,
            path,
        }
    }

    async fn insert_message(db: &TestDb, chat: &str, date_ns: i64, is_from_me: bool, text: &str) {
        let guid = format!("{chat}-{date_ns}-{}", uuid_like_suffix());
        sqlx::query(
            "INSERT INTO message (guid, text, date, is_from_me) VALUES (?, ?, ?, ?)",
        )
        .bind(guid.clone())
        .bind(text)
        .bind(date_ns)
        .bind(i64::from(is_from_me))
        .execute(&db.pool)
        .await
        .unwrap();
        let message_id: i64 = sqlx::query("SELECT ROWID AS r FROM message WHERE guid = ?")
            .bind(guid)
            .fetch_one(&db.pool)
            .await
            .unwrap()
            .try_get("r")
            .unwrap();
        let chat_id: i64 = sqlx::query("SELECT ROWID AS r FROM chat WHERE guid = ?")
            .bind(chat)
            .fetch_one(&db.pool)
            .await
            .unwrap()
            .try_get("r")
            .unwrap();
        sqlx::query("INSERT INTO chat_message_join (chat_id, message_id) VALUES (?, ?)")
            .bind(chat_id)
            .bind(message_id)
            .execute(&db.pool)
            .await
            .unwrap();
    }

    async fn insert_chat(db: &TestDb, guid: &str, display_name: Option<&str>, handles: &[&str]) {
        sqlx::query("INSERT INTO chat (guid, display_name, service_name) VALUES (?, ?, 'iMessage')")
            .bind(guid)
            .bind(display_name)
            .execute(&db.pool)
            .await
            .unwrap();
        let chat_id: i64 = sqlx::query("SELECT ROWID AS r FROM chat WHERE guid = ?")
            .bind(guid)
            .fetch_one(&db.pool)
            .await
            .unwrap()
            .try_get("r")
            .unwrap();
        for handle in handles {
            sqlx::query("INSERT INTO handle (id, uncanonicalized_id) VALUES (?, ?)")
                .bind(handle)
                .bind(handle)
                .execute(&db.pool)
                .await
                .unwrap();
            let handle_id: i64 =
                sqlx::query("SELECT ROWID AS r FROM handle WHERE id = ? ORDER BY ROWID DESC")
                    .bind(handle)
                    .fetch_one(&db.pool)
                    .await
                    .unwrap()
                    .try_get("r")
                    .unwrap();
            sqlx::query("INSERT INTO chat_handle_join (chat_id, handle_id) VALUES (?, ?)")
                .bind(chat_id)
                .bind(handle_id)
                .execute(&db.pool)
                .await
                .unwrap();
        }
    }

    fn ns_at(iso: &str) -> i64 {
        unix_ms_to_core_data_ns(
            DateTime::parse_from_rfc3339(iso).unwrap().timestamp_millis(),
        )
    }

    #[tokio::test]
    async fn missing_database_is_unavailable() {
        let err = ChatAnalytics::open_at(Path::new("/tmp/does-not-exist-chat.db"))
            .await
            .unwrap_err();
        assert_eq!(
            err.kind,
            ChatAnalyticsErrorKind::Unavailable,
            "missing db must not be reported as permission denial"
        );
    }

    #[tokio::test]
    async fn non_database_file_maps_to_permission_denied() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.db");
        std::fs::write(&path, b"not a database").unwrap();
        // A file that is not SQLite fails on first read; the important
        // property is that the error is a distinguishable variant, not empty.
        let err = ChatAnalytics::open_at(&path).await.unwrap_err();
        assert!(matches!(
            err.kind,
            ChatAnalyticsErrorKind::PermissionDenied | ChatAnalyticsErrorKind::Unavailable
        ));
    }

    #[tokio::test]
    async fn find_chats_filters_by_name_and_participants() {
        let db = build_test_db().await;
        insert_chat(&db, "i;-;+15551230001", Some("Design Crew"), &["+15551230002"])
            .await;
        insert_chat(&db, "i;-;+15551230003", None, &["+15551230003"]).await;
        insert_message(&db, "i;-;+15551230001", ns_at("2026-07-01T10:00:00Z"), false, "hi").await;
        insert_message(&db, "i;-;+15551230003", ns_at("2026-07-02T10:00:00Z"), true, "yo").await;

        let result = db
            .analytics
            .find_chats(&FindChatsRequest {
                chat_name: Some("crew".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.chats.len(), 1);
        assert_eq!(result.chats[0].chat_guid, "i;-;+15551230001");
        assert_eq!(result.chats[0].display_name.as_deref(), Some("Design Crew"));
        assert_eq!(result.chats[0].participants, vec!["+15551230002".to_string()]);

        let by_participant = db
            .analytics
            .find_chats(&FindChatsRequest {
                participants: Some(vec!["+15551230003".into()]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_participant.chats.len(), 1);
        assert_eq!(by_participant.chats[0].chat_guid, "i;-;+15551230003");
    }

    #[tokio::test]
    async fn find_chats_participant_exactness_is_not_inverted() {
        // MAJOR-3 regression: non-exact matching must allow extra
        // participants; exact matching must require the sets be equal.
        let db = build_test_db().await;
        // 1:1 chat (one participant) and a group (two).
        insert_chat(&db, "guid-1to1", None, &["+15550000001"]).await;
        insert_chat(&db, "guid-group", Some("Group"), &["+15550000001", "+15550000002"]).await;
        insert_message(&db, "guid-1to1", ns_at("2026-07-01T10:00:00Z"), false, "a").await;
        insert_message(&db, "guid-group", ns_at("2026-07-01T11:00:00Z"), false, "b").await;

        // Non-exact: both chats contain the participant; the extra member of
        // the group must NOT disqualify it.
        let non_exact = db
            .analytics
            .find_chats(&FindChatsRequest {
                participants: Some(vec!["+15550000001".into()]),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut guids: Vec<&str> =
            non_exact.chats.iter().map(|c| c.chat_guid.as_str()).collect();
        guids.sort();
        assert_eq!(guids, vec!["guid-1to1", "guid-group"]);

        // Exact: only the chat whose participant set equals the request.
        let exact = db
            .analytics
            .find_chats(&FindChatsRequest {
                participants: Some(vec!["+15550000001".into()]),
                exact_participants: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(exact.chats.len(), 1);
        assert_eq!(exact.chats[0].chat_guid, "guid-1to1");

        // Exact with the full group set: only the group.
        let exact_group = db
            .analytics
            .find_chats(&FindChatsRequest {
                participants: Some(vec!["+15550000001".into(), "+15550000002".into()]),
                exact_participants: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(exact_group.chats.len(), 1);
        assert_eq!(exact_group.chats[0].chat_guid, "guid-group");
    }

    #[tokio::test]
    async fn find_chats_single_participant_finds_multi_handle_chat() {
        // Verifier's exact scenario: a 3-handle group chat. A NON-exact
        // single-name query must find it; an EXACT query with the wrong
        // count must not.
        let db = build_test_db().await;
        insert_chat(
            &db,
            "guid-trio",
            Some("Trio"),
            &["+15551110001", "+15551110002", "+15551110003"],
        )
        .await;
        insert_message(&db, "guid-trio", ns_at("2026-07-01T10:00:00Z"), false, "hi").await;

        let non_exact = db
            .analytics
            .find_chats(&FindChatsRequest {
                participants: Some(vec!["+15551110002".into()]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(non_exact.chats.len(), 1);
        assert_eq!(non_exact.chats[0].chat_guid, "guid-trio");

        let exact_wrong_count = db
            .analytics
            .find_chats(&FindChatsRequest {
                participants: Some(vec!["+15551110002".into()]),
                exact_participants: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(exact_wrong_count.chats.is_empty());
    }

    #[tokio::test]
    async fn find_chats_unread_only_uses_unread_predicate() {
        let db = build_test_db().await;
        insert_chat(&db, "guid-a", None, &["+15550001111"]).await;
        insert_chat(&db, "guid-b", None, &["+15550002222"]).await;
        // Unread: is_read=0, from them, finished, not a system message.
        insert_message(&db, "guid-a", ns_at("2026-07-01T10:00:00Z"), false, "ping").await;
        sqlx::query("UPDATE message SET is_read = 0 WHERE text = 'ping'")
            .execute(&db.pool)
            .await
            .unwrap();
        // Read message: must not count as unread.
        insert_message(&db, "guid-b", ns_at("2026-07-01T11:00:00Z"), false, "seen").await;

        let result = db
            .analytics
            .find_chats(&FindChatsRequest {
                unread_only: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.chats.len(), 1);
        assert_eq!(result.chats[0].chat_guid, "guid-a");
        assert_eq!(result.chats[0].unread_count, 1);

        // Own unread-marked messages (is_from_me=1) are never "unread".
        insert_message(&db, "guid-b", ns_at("2026-07-01T12:00:00Z"), true, "mine").await;
        sqlx::query("UPDATE message SET is_read = 0 WHERE text = 'mine'")
            .execute(&db.pool)
            .await
            .unwrap();
        let again = db
            .analytics
            .find_chats(&FindChatsRequest {
                unread_only: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(again.chats.len(), 1);
    }

    #[tokio::test]
    async fn find_chats_cursor_pagination_is_exact() {
        let db = build_test_db().await;
        for i in 0..3 {
            let guid = format!("guid-{i}");
            insert_chat(&db, &guid, None, &[&format!("+1555000000{i}")]).await;
            let iso = format!("2026-07-0{}T10:00:00Z", i + 1);
            insert_message(&db, &guid, ns_at(&iso), false, "m").await;
        }
        let page1 = db.analytics.find_chats(&FindChatsRequest {
            limit: Some(2),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(page1.chats.len(), 2);
        assert_eq!(page1.matching_chat_count, 3);
        assert!(page1.has_more);
        let cursor = page1.next_cursor.clone().unwrap();

        let page2 = db.analytics.find_chats(&FindChatsRequest {
            limit: Some(2),
            cursor: Some(cursor),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(page2.chats.len(), 1);
        assert_eq!(page2.matching_chat_count, 3);
        assert!(!page2.has_more);
        assert!(page2.next_cursor.is_none());

        let mut seen: Vec<&str> = page1
            .chats
            .iter()
            .chain(page2.chats.iter())
            .map(|c| c.chat_guid.as_str())
            .collect();
        seen.sort();
        assert_eq!(seen, vec!["guid-0", "guid-1", "guid-2"]);
    }

    #[tokio::test]
    async fn sent_received_split_and_total_interval() {
        let db = build_test_db().await;
        insert_chat(&db, "guid-a", None, &["+15550001111"]).await;
        insert_message(&db, "guid-a", ns_at("2026-07-01T10:00:00Z"), true, "a1").await;
        insert_message(&db, "guid-a", ns_at("2026-07-01T11:00:00Z"), true, "a2").await;
        insert_message(&db, "guid-a", ns_at("2026-07-02T10:00:00Z"), false, "a3").await;

        let result = db
            .analytics
            .count_message_activity(&CountActivityRequest {
                from: Some("2026-07-01T00:00:00Z".into()),
                to: Some("2026-07-03T00:00:00Z".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.buckets.len(), 1, "total interval yields one bucket");
        assert_eq!(result.counts_by_bucket.total, vec![3]);
        assert_eq!(result.counts_by_bucket.sent, vec![2]);
        assert_eq!(result.counts_by_bucket.received, vec![1]);
        assert_eq!(result.overall_activity, ActivityCount { total: 3, sent: 2, received: 1 });
        assert_eq!(result.matching_chat_count, 1);
    }

    #[tokio::test]
    async fn day_buckets_include_empty_intervals_and_align() {
        let db = build_test_db().await;
        insert_chat(&db, "guid-a", None, &["+15550001111"]).await;
        insert_message(&db, "guid-a", ns_at("2026-07-01T10:00:00Z"), true, "d1").await;
        insert_message(&db, "guid-a", ns_at("2026-07-04T10:00:00Z"), false, "d2").await;

        let result = db
            .analytics
            .count_message_activity(&CountActivityRequest {
                from: Some("2026-07-01T00:00:00Z".into()),
                to: Some("2026-07-06T00:00:00Z".into()),
                interval: Some(ActivityInterval::Day),
                ..Default::default()
            })
            .await
            .unwrap();
        // Buckets are cut at LOCAL midnight (spec: calendar buckets use the
        // caller's time zone), so the count depends on the machine's offset:
        // each local day touched by the range gets exactly one bucket.
        assert_eq!(
            result.buckets.len(),
            result.counts_by_bucket.total.len(),
            "counts stay index-aligned with buckets"
        );
        for window in result.buckets.windows(2) {
            assert_eq!(window[0].end_ms, window[1].start_ms, "buckets are contiguous");
            let end_local = millis_to_local(window[0].end_ms);
            assert_eq!(
                end_local,
                end_local
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_local_timezone(Local)
                    .single()
                    .unwrap(),
                "each bucket ends at local midnight"
            );
        }
        // Half-open: the last bucket ends at or after the requested upper
        // bound, and every message lands in the bucket containing its instant.
        assert!(result.buckets.last().unwrap().end_ms >= result.range_end_ms);
        assert_eq!(result.counts_by_bucket.total.iter().sum::<i64>(), 2);
        let first_with_activity = result
            .counts_by_bucket
            .total
            .iter()
            .position(|c| *c > 0)
            .unwrap();
        assert_eq!(result.counts_by_bucket.sent[first_with_activity], 1);
        let last_with_activity = result
            .counts_by_bucket
            .total
            .iter()
            .rposition(|c| *c > 0)
            .unwrap();
        assert_eq!(result.counts_by_bucket.received[last_with_activity], 1);
        // Empty interior buckets are represented explicitly (spec: "including
        // intervals with no activity").
        assert!(result.counts_by_bucket.total.contains(&0));
    }

    #[tokio::test]
    async fn week_buckets_begin_monday() {
        // 2026-07-01 is a Wednesday; the first bucket must start Monday
        // 2026-06-29T00:00 local.
        let db = build_test_db().await;
        insert_chat(&db, "guid-a", None, &["+15550001111"]).await;
        insert_message(&db, "guid-a", ns_at("2026-07-01T10:00:00Z"), true, "w1").await;

        let result = db
            .analytics
            .count_message_activity(&CountActivityRequest {
                from: Some("2026-07-01T10:00:00Z".into()),
                to: Some("2026-07-14T00:00:00Z".into()),
                interval: Some(ActivityInterval::Week),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.buckets.len(), 3);
        let first = result.buckets[0];
        let start_local = millis_to_local(first.start_ms);
        assert_eq!(start_local.weekday(), chrono::Weekday::Mon);
        assert_eq!(start_local.hour(), 0);
        assert_eq!(result.counts_by_bucket.total, vec![1, 0, 0]);
    }

    #[tokio::test]
    async fn chat_breakdown_ranks_and_paginates() {
        let db = build_test_db().await;
        insert_chat(&db, "guid-a", Some("Alpha"), &["+15550001111"]).await;
        insert_chat(&db, "guid-b", Some("Beta"), &["+15550002222"]).await;
        insert_chat(&db, "guid-c", Some("Gamma"), &["+15550003333"]).await;
        for _ in 0..3 {
            insert_message(&db, "guid-a", ns_at("2026-07-01T10:00:00Z"), true, "x").await;
        }
        insert_message(&db, "guid-b", ns_at("2026-07-01T10:05:00Z"), false, "y").await;
        insert_message(&db, "guid-c", ns_at("2026-07-01T10:07:00Z"), false, "z").await;

        let page1 = db
            .analytics
            .count_message_activity(&CountActivityRequest {
                from: Some("2026-07-01T00:00:00Z".into()),
                to: Some("2026-07-03T00:00:00Z".into()),
                breakdown: Some(ActivityBreakdown::Chat),
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page1.matching_chat_count, 3);
        assert_eq!(page1.chats.len(), 2);
        assert_eq!(page1.chats[0].chat_guid, "guid-a");
        assert_eq!(page1.chats[0].activity, ActivityCount { total: 3, sent: 3, received: 0 });
        assert!(page1.has_more);

        let page2 = db
            .analytics
            .count_message_activity(&CountActivityRequest {
                from: Some("2026-07-01T00:00:00Z".into()),
                to: Some("2026-07-03T00:00:00Z".into()),
                breakdown: Some(ActivityBreakdown::Chat),
                limit: Some(2),
                cursor: page1.next_cursor.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page2.chats.len(), 1);
        assert_eq!(page2.chats[0].activity.total, 1);
        assert!(!page2.has_more);
        assert!(page2.next_cursor.is_none());
    }

    #[tokio::test]
    async fn chat_guid_filter_and_overall_breakdown_guard() {
        let db = build_test_db().await;
        insert_chat(&db, "guid-a", None, &["+15550001111"]).await;
        insert_chat(&db, "guid-b", None, &["+15550002222"]).await;
        insert_message(&db, "guid-a", ns_at("2026-07-01T10:00:00Z"), true, "a").await;
        insert_message(&db, "guid-b", ns_at("2026-07-01T10:00:00Z"), true, "b").await;

        let filtered = db
            .analytics
            .count_message_activity(&CountActivityRequest {
                from: Some("2026-07-01T00:00:00Z".into()),
                to: Some("2026-07-03T00:00:00Z".into()),
                chat_guids: Some(vec!["guid-a".into()]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(filtered.overall_activity.total, 1);
        assert_eq!(filtered.matching_chat_count, 1);

        let guarded = db
            .analytics
            .count_message_activity(&CountActivityRequest {
                breakdown: Some(ActivityBreakdown::Overall),
                cursor: Some(encode_cursor(1)),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(guarded.message.contains("breakdown is chat"));
    }

    #[tokio::test]
    async fn malformed_cursor_is_explicit() {
        let err = decode_cursor(Some("not-a-cursor")).unwrap_err();
        assert!(err
            .message
            .contains("cursor is malformed; use next_cursor returned by count_message_activity"));
    }

    #[tokio::test]
    async fn attributed_body_text_extraction_roundtrip() {
        // Hand-built typedstream-style blob: 8-byte header + string marker.
        let make_blob = |text: &str| {
            let mut bytes = vec![0x04, 0x0b, 0xb0, 0x00, 0x00, 0x00, 0x81, 0x00];
            bytes.push(0x81);
            bytes.push(text.len() as u8);
            bytes.extend_from_slice(text.as_bytes());
            bytes
        };
        assert_eq!(
            extract_message_from_attributed_body(&make_blob("hello there")).as_deref(),
            Some("hello there")
        );
        // Longest run wins over attribute keys.
        let mut bytes = vec![0x04, 0x0b, 0xb0, 0x00, 0x00, 0x00, 0x81, 0x00];
        for text in ["key", "the real message body"] {
            bytes.push(0x81);
            bytes.push(text.len() as u8);
            bytes.extend_from_slice(text.as_bytes());
        }
        assert_eq!(
            extract_message_from_attributed_body(&bytes).as_deref(),
            Some("the real message body")
        );
        // Unparseable blobs degrade to None, never panic.
        assert_eq!(extract_message_from_attributed_body(&[0xff, 0xfe, 0x00]), None);
        assert_eq!(extract_message_from_attributed_body(&[]), None);
    }

    #[tokio::test]
    async fn read_message_texts_redacts_and_prefers_text_column() {
        let db = build_test_db().await;
        insert_chat(&db, "guid-a", None, &["+15550001111"]).await;
        insert_message(
            &db,
            "guid-a",
            ns_at("2026-07-01T10:00:00Z"),
            false,
            "api_key: hunter2secretvalue",
        )
        .await;
        let texts = db
            .analytics
            .read_message_texts("guid-a", None, None, 10)
            .await
            .unwrap();
        assert_eq!(texts.len(), 1);
        assert!(
            !texts[0].contains("hunter2"),
            "text must be redacted: {}",
            texts[0]
        );
        assert!(texts[0].contains("[REDACTED_SECRET]"), "got: {}", texts[0]);
    }

    #[test]
    fn core_data_epoch_roundtrip() {
        let ms = 1_782_864_000_000_i64; // 2026-07-01T00:00:00Z
        let ns = unix_ms_to_core_data_ns(ms);
        assert_eq!(core_data_ns_to_unix_ms(ns), ms);
        // Spec idiom: round(date / 1e9) + 978307200 == unix seconds.
        assert_eq!(ns.div_euclid(NANOS_PER_SEC) + CORE_DATA_EPOCH_SECS, ms / 1000);
    }

    #[test]
    fn bucket_index_is_half_open() {
        let buckets = vec![
            ActivityBucket { start_ms: 0, end_ms: 10 },
            ActivityBucket { start_ms: 10, end_ms: 20 },
        ];
        assert_eq!(bucket_index(&buckets, 0), Some(0));
        assert_eq!(bucket_index(&buckets, 10), Some(1));
        assert_eq!(bucket_index(&buckets, 20), None, "end bound is exclusive");
        assert_eq!(bucket_index(&buckets, 99), None);
    }
}
