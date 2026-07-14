//! Persistent high-water mark ("watermark") for processed Telegram updates.
//!
//! ## Why this exists
//!
//! Telegram's long-polling model only confirms a batch of updates when the
//! NEXT `getUpdates` call carries an advanced `offset`. The poll loop used to
//! keep that offset purely in memory (starting at `None`), so if the process
//! restarted after dispatching a batch to the message loop but before issuing
//! the next poll, Telegram redelivered the entire batch on startup — and the
//! channel Agent session (which auto-approves tools) re-executed every
//! contained action. Creation-style actions thus ran twice (one situational
//! cause of the "duplicate companion creation" bug).
//!
//! ## How it works
//!
//! The highest processed `update_id` is persisted per bot, together with the
//! wall-clock time it was written. On startup the poll loop:
//!  1. seeds its `getUpdates` offset with `watermark + 1`, confirming the
//!     pre-restart batch server-side on the very first poll AND filtering it
//!     out of the response, and
//!  2. treats any update the server STILL delivers at or below the watermark
//!     as a server-side `update_id` sequence reset (see below): it is
//!     processed normally and the watermark is rebased onto it.
//!
//! ## update_ids are NOT forever monotonic: the idle-week random reset
//!
//! The Bot API docs state: "If there are no new updates for at least a week,
//! then identifier of the next update will be chosen randomly instead of
//! sequentially." A watermark persisted before such an idle stretch may
//! therefore lie ABOVE every update_id of the new sequence; naively skipping
//! `id <= watermark` would then silently drop all subsequent messages forever
//! (and the watermark would never advance, so restarts wouldn't heal it).
//! Two complementary defenses:
//!
//!  - **Persistence TTL** — `load` discards watermarks older than 5 days.
//!    The random reset requires >= 1 week of inactivity, so 5 days leaves a
//!    comfortable safety margin while still covering every realistic restart
//!    window. Discarding is safe: a bot idle for days has no pending batch
//!    awaiting redelivery, so there is nothing left to deduplicate.
//!  - **Runtime reset detection** — because the offset is always seeded at
//!    `watermark + 1`, genuine crash-window redeliveries are filtered
//!    server-side and never reach the loop. If an update with
//!    `id <= watermark` arrives anyway, it can only be the random sequence
//!    reset: it is processed (never skipped) and the watermark is rebased
//!    onto the new sequence, with a warning.
//!
//! ## Crash-window semantics: prefer loss over duplication
//!
//! The watermark is advanced and persisted IMMEDIATELY AFTER an update is
//! dispatched onto the message loop's queue — not after the agent finishes
//! handling it. Two asymmetric crash windows follow:
//!
//!  - duplication window (tiny): dying between dispatch and persist re-runs
//!    that one update on restart;
//!  - loss window (seconds to minutes): once the watermark is persisted, the
//!    update lives only in the in-memory agent queue until handling actually
//!    completes — a crash anywhere in that span drops the message for good.
//!
//! This trade-off is deliberate: duplicated agent executions (creating companions,
//! conversations, files…) are far more costly than one dropped IM message the
//! user can simply resend.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

/// Discard persisted watermarks older than this. Telegram only randomizes the
/// next update_id after at least one week of inactivity; 5 days keeps a
/// two-day safety margin below that while still covering every realistic
/// restart/redelivery window (which is seconds, not days).
const WATERMARK_TTL_MS: i64 = 5 * 24 * 60 * 60 * 1000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Uniquifies temp-file names if saves ever race (multiple bots, tests).
static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Abstraction over watermark persistence so the reset/record logic can be
/// unit-tested without touching the filesystem.
pub trait WatermarkStore: Send + Sync {
    /// Last processed update_id for the given bot, if any (and still fresh —
    /// stores may apply a TTL, see `FileWatermarkStore`).
    fn load(&self, bot_id: &str) -> Option<i64>;
    /// Persist the last processed update_id for the given bot.
    ///
    /// Failures must be swallowed (logged) — persistence is best-effort and
    /// degrades to the pre-fix behavior (one redelivery window at worst).
    fn save(&self, bot_id: &str, update_id: i64);
}

/// No-op store used when no writable state directory could be derived.
/// Behavior degrades to the pre-fix semantics (memory-only offset).
pub struct NoopWatermarkStore;

impl WatermarkStore for NoopWatermarkStore {
    fn load(&self, _bot_id: &str) -> Option<i64> {
        None
    }

    fn save(&self, _bot_id: &str, _update_id: i64) {}
}

/// File-backed store: one tiny text file per bot under `dir`, containing
/// `"<update_id> <saved_at_ms>"` (format v2; `saved_at_ms` is a Unix
/// timestamp in milliseconds backing the load-time TTL). A missing,
/// corrupted, legacy-format, or expired file degrades to `None`, which means
/// at most one redelivery window — identical to the pre-fix behavior, never
/// worse.
pub struct FileWatermarkStore {
    dir: PathBuf,
}

impl FileWatermarkStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Derive the store location from the process-global runtime root.
    ///
    /// Both hosts (desktop embedded backend and the web binary) call
    /// `nomifun_runtime::init(&data_dir)` before the channel manager starts,
    /// which anchors `runtime_root()` at `{data_dir}/runtime` — so the
    /// watermark lands in `{data_dir}/channel/`, next to the other data-dir
    /// subtrees (knowledge/, companion/, logs/…). If `init` never ran (unit tests,
    /// exotic hosts), `runtime_root()` falls back to the OS cache dir, which
    /// is still stable across restarts.
    pub fn for_app_data() -> Option<Self> {
        let runtime_root = nomifun_runtime::runtime_root()?;
        let base = runtime_root.parent()?.to_path_buf();
        Some(Self::new(base.join("channel")))
    }

    fn file_path(&self, bot_id: &str) -> PathBuf {
        // Telegram bot ids are numeric; filter defensively anyway so an
        // unexpected id can never become a path traversal.
        let safe: String = bot_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        self.dir.join(format!("telegram-update-watermark-{safe}.txt"))
    }
}

impl WatermarkStore for FileWatermarkStore {
    fn load(&self, bot_id: &str) -> Option<i64> {
        let path = self.file_path(bot_id);
        let raw = std::fs::read_to_string(&path).ok()?;
        // Format v2: "<update_id> <saved_at_ms>". The original (v1) format
        // was the bare update_id with no timestamp; without a timestamp the
        // TTL below cannot be enforced, so legacy files are deliberately
        // treated like corruption and degrade to None. Cost: at most one
        // redelivery window after upgrading, same as a fresh install.
        let fields: Vec<&str> = raw.split_whitespace().collect();
        let parsed = match fields.as_slice() {
            [id, saved_at] => id.parse::<i64>().ok().zip(saved_at.parse::<i64>().ok()),
            _ => None,
        };
        let Some((update_id, saved_at)) = parsed else {
            warn!(path = %path.display(), "ignoring corrupted or legacy-format telegram watermark file");
            return None;
        };
        // TTL: after >= 1 week of bot inactivity Telegram picks the next
        // update_id RANDOMLY — possibly below this watermark — so a stale
        // watermark must not be trusted (see module docs). Discarding is
        // safe: days of inactivity mean no pending batch to deduplicate.
        let age_ms = now_ms().saturating_sub(saved_at);
        if age_ms > WATERMARK_TTL_MS {
            warn!(
                path = %path.display(),
                update_id,
                age_ms,
                "discarding stale telegram watermark (update_id sequence may have been randomly reset server-side)"
            );
            return None;
        }
        Some(update_id)
    }

    fn save(&self, bot_id: &str, update_id: i64) {
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            warn!(dir = %self.dir.display(), error = %e, "failed to create telegram watermark dir");
            return;
        }
        let path = self.file_path(bot_id);
        // Write-to-temp + rename: a same-directory rename is atomic, so a
        // crash mid-write can never leave a partial file like "12" (from
        // "123456") that would parse as a valid LOWER watermark and re-open
        // the dedup window. Same pattern as nomifun-companion's
        // fsio::save_json_atomic.
        let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = self.dir.join(format!(".telegram-update-watermark.tmp.{}.{seq}", std::process::id()));
        let payload = format!("{update_id} {}", now_ms());
        let result = std::fs::write(&tmp, payload).and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            warn!(path = %path.display(), error = %e, "failed to persist telegram watermark");
        }
    }
}

/// Build the production store: file-backed when a state directory is
/// derivable, otherwise a no-op (with a warning, since dedup across restarts
/// is then unavailable).
pub fn default_watermark_store() -> Arc<dyn WatermarkStore> {
    match FileWatermarkStore::for_app_data() {
        Some(store) => Arc::new(store),
        None => {
            warn!("no state directory derivable; telegram update dedup will not survive restarts");
            Arc::new(NoopWatermarkStore)
        }
    }
}

/// How an incoming `update_id` relates to the current watermark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateArrival {
    /// Normal forward progress: above the watermark (or no watermark yet).
    New,
    /// `id <= watermark` even though the `getUpdates` offset was seeded past
    /// the watermark. Server-side offset filtering makes genuine
    /// redeliveries unreachable here, so this can only be Telegram's random
    /// update_id sequence reset (bot idle >= 1 week). The update must be
    /// processed — never skipped — and the watermark rebased onto the new
    /// sequence (see module docs).
    SequenceReset,
}

/// Pure watermark logic over processed update_ids.
///
/// Owned by the poll loop (single task), so no interior mutability is needed.
#[derive(Debug, Default)]
pub struct UpdateWatermark {
    last_processed: Option<i64>,
}

impl UpdateWatermark {
    pub fn new(last_processed: Option<i64>) -> Self {
        Self { last_processed }
    }

    /// Classify an incoming update relative to the watermark.
    pub fn classify(&self, update_id: i64) -> UpdateArrival {
        match self.last_processed {
            Some(w) if update_id <= w => UpdateArrival::SequenceReset,
            _ => UpdateArrival::New,
        }
    }

    /// Record `update_id` as processed: forward advance in the normal case,
    /// backward rebase after a sequence reset. Every delivered update is
    /// recorded — there is no skip path (see module docs).
    pub fn record(&mut self, update_id: i64) {
        self.last_processed = Some(update_id);
    }

    /// The `getUpdates` offset that confirms everything processed so far:
    /// `last_processed + 1`, or `None` when nothing was ever processed.
    pub fn next_offset(&self) -> Option<i64> {
        self.last_processed.map(|w| w + 1)
    }

    pub fn last_processed(&self) -> Option<i64> {
        self.last_processed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- UpdateWatermark: classify / record ----------------------------------

    #[test]
    fn fresh_watermark_classifies_everything_new() {
        let wm = UpdateWatermark::new(None);
        assert_eq!(wm.classify(1), UpdateArrival::New);
        assert_eq!(wm.classify(i64::MAX), UpdateArrival::New);
        assert_eq!(wm.next_offset(), None);
        assert_eq!(wm.last_processed(), None);
    }

    #[test]
    fn at_or_below_watermark_is_sequence_reset_above_is_new() {
        let wm = UpdateWatermark::new(Some(100));
        assert_eq!(wm.classify(99), UpdateArrival::SequenceReset);
        assert_eq!(wm.classify(100), UpdateArrival::SequenceReset);
        assert_eq!(wm.classify(101), UpdateArrival::New);
    }

    #[test]
    fn record_advances_forward() {
        let mut wm = UpdateWatermark::new(Some(100));
        wm.record(105);
        assert_eq!(wm.last_processed(), Some(105));
        assert_eq!(wm.next_offset(), Some(106));
    }

    #[test]
    fn record_rebchs_backward_after_sequence_reset() {
        // Idle-week scenario: watermark at 100_000, Telegram restarts the
        // sequence at a random lower id. The update is processed and the
        // watermark follows the NEW sequence — skipping here would drop
        // every subsequent message forever.
        let mut wm = UpdateWatermark::new(Some(100_000));
        assert_eq!(wm.classify(57), UpdateArrival::SequenceReset);
        wm.record(57);
        assert_eq!(wm.last_processed(), Some(57));
        assert_eq!(wm.next_offset(), Some(58));
        // The new sequence then progresses normally.
        assert_eq!(wm.classify(58), UpdateArrival::New);
        wm.record(58);
        assert_eq!(wm.last_processed(), Some(58));
    }

    #[test]
    fn next_offset_is_last_plus_one() {
        let mut wm = UpdateWatermark::new(None);
        wm.record(41);
        assert_eq!(wm.next_offset(), Some(42));
    }

    // -- FileWatermarkStore: persistence roundtrip ---------------------------

    fn temp_store() -> (tempfile::TempDir, FileWatermarkStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileWatermarkStore::new(dir.path().join("channel"));
        (dir, store)
    }

    #[test]
    fn save_load_roundtrip() {
        let (_guard, store) = temp_store();
        assert_eq!(store.load("12345"), None);
        store.save("12345", 777);
        assert_eq!(store.load("12345"), Some(777));
        // Overwrite advances.
        store.save("12345", 778);
        assert_eq!(store.load("12345"), Some(778));
    }

    #[test]
    fn saved_file_uses_v2_format_with_timestamp() {
        let (_guard, store) = temp_store();
        let before = now_ms();
        store.save("42", 900);
        let raw = std::fs::read_to_string(store.file_path("42")).unwrap();
        let fields: Vec<&str> = raw.split_whitespace().collect();
        assert_eq!(fields.len(), 2, "v2 format is '<update_id> <saved_at_ms>': {raw:?}");
        assert_eq!(fields[0].parse::<i64>().unwrap(), 900);
        let saved_at = fields[1].parse::<i64>().unwrap();
        assert!(saved_at >= before && saved_at <= now_ms());
    }

    #[test]
    fn atomic_save_leaves_no_temp_files_behind() {
        let (_guard, store) = temp_store();
        store.save("7", 1);
        store.save("7", 2);
        let entries: Vec<String> = std::fs::read_dir(&store.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["telegram-update-watermark-7.txt".to_string()]);
        assert_eq!(store.load("7"), Some(2));
    }

    #[test]
    fn bots_are_isolated() {
        let (_guard, store) = temp_store();
        store.save("bot_a", 10);
        store.save("bot_b", 20);
        assert_eq!(store.load("bot_a"), Some(10));
        assert_eq!(store.load("bot_b"), Some(20));
    }

    #[test]
    fn corrupted_file_degrades_to_none() {
        let (_guard, store) = temp_store();
        store.save("9", 1); // ensure dir exists
        for junk in ["not-a-number", "1 2 3", "100 not-a-ts", ""] {
            std::fs::write(store.file_path("9"), junk).unwrap();
            assert_eq!(store.load("9"), None, "junk content {junk:?} must degrade to None");
        }
    }

    #[test]
    fn legacy_v1_single_number_format_degrades_to_none() {
        // v1 files carry no timestamp, so the TTL cannot be enforced on
        // them; they are deliberately dropped (one redelivery window at
        // worst — same as a fresh install).
        let (_guard, store) = temp_store();
        store.save("8", 1); // ensure dir exists
        std::fs::write(store.file_path("8"), "123456").unwrap();
        assert_eq!(store.load("8"), None);
    }

    #[test]
    fn stale_watermark_past_ttl_is_discarded() {
        let (_guard, store) = temp_store();
        store.save("6", 1); // ensure dir exists
        // 6 days old: past the 5-day TTL — Telegram may have randomly reset
        // the sequence (>= 1 week idle), so the watermark must not be trusted.
        let six_days_ago = now_ms() - 6 * 24 * 60 * 60 * 1000;
        std::fs::write(store.file_path("6"), format!("500 {six_days_ago}")).unwrap();
        assert_eq!(store.load("6"), None);
        // 4 days old: within TTL, still valid.
        let four_days_ago = now_ms() - 4 * 24 * 60 * 60 * 1000;
        std::fs::write(store.file_path("6"), format!("500 {four_days_ago}")).unwrap();
        assert_eq!(store.load("6"), Some(500));
    }

    #[test]
    fn bot_id_is_sanitized_for_filenames() {
        let (_guard, store) = temp_store();
        let evil = "..\\..\\evil/../id";
        store.save(evil, 5);
        assert_eq!(store.load(evil), Some(5));
        // The written file must stay inside the store dir.
        let path = store.file_path(evil);
        assert!(path.starts_with(&store.dir));
        assert!(path.file_name().unwrap().to_string_lossy().contains("evilid"));
    }

    #[test]
    fn restart_roundtrip_seeds_offset_past_processed_batch() {
        // Restart scenario at the logic level: process batch [1,2,3], then
        // "restart". Dedup of the pre-restart batch is server-side — the
        // reloaded watermark seeds offset 4 so Telegram filters [1,2,3] out
        // of the first poll response.
        let (_guard, store) = temp_store();
        let bot = "555";

        let mut wm = UpdateWatermark::new(store.load(bot));
        for id in [1i64, 2, 3] {
            assert_eq!(wm.classify(id), UpdateArrival::New);
            wm.record(id);
            store.save(bot, id);
        }

        // "Restart": fresh watermark seeded from disk.
        let wm = UpdateWatermark::new(store.load(bot));
        assert_eq!(wm.next_offset(), Some(4));
    }

    #[test]
    fn sequence_reset_roundtrip_through_store() {
        // Idle-week scenario end-to-end at the logic level: watermark 1000
        // persisted, Telegram delivers a randomly reset id 57 despite the
        // seeded offset. It is processed, the watermark rebases, and the
        // rebased value survives a restart.
        let (_guard, store) = temp_store();
        let bot = "777";
        store.save(bot, 1000);

        let mut wm = UpdateWatermark::new(store.load(bot));
        assert_eq!(wm.next_offset(), Some(1001));
        assert_eq!(wm.classify(57), UpdateArrival::SequenceReset);
        wm.record(57);
        store.save(bot, 57);

        let wm = UpdateWatermark::new(store.load(bot));
        assert_eq!(wm.last_processed(), Some(57));
        assert_eq!(wm.next_offset(), Some(58));
    }

    #[test]
    fn noop_store_loads_none() {
        let store = NoopWatermarkStore;
        store.save("1", 99);
        assert_eq!(store.load("1"), None);
    }
}
