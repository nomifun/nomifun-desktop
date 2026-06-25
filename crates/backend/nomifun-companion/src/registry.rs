//! `CompanionRegistry` — the in-memory roster of companion profiles, mirrored to disk as
//! one `companion/companions/{id}/config.json` per companion. Boot does a synchronous [`scan`]
//! of the companions dir; afterwards every mutation (create/patch/remove) saves the
//! profile first and only then updates the map under the write lock, so the
//! map never claims a companion whose file failed to persist.
//!
//! The registry also owns companion short numbers ([`CompanionProfileConfig::seq`]) and
//! their high-watermark, persisted in a registry-private state file
//! ([`SEQ_STATE_FILE`] under the shared dir) that no API config write path
//! can reach: [`create`] allocates the next number from the watermark and
//! [`backfill_missing_seqs`] numbers pre-rollout profiles at boot — both
//! inside the same critical section that mutates the roster, so concurrent
//! creates can never mint the same number.
//!
//! [`scan`]: CompanionRegistry::scan
//! [`create`]: CompanionRegistry::create
//! [`backfill_missing_seqs`]: CompanionRegistry::backfill_missing_seqs

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nomifun_common::AppError;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::profile::CompanionProfileConfig;

/// Maximum companion display-name length, counted in chars (not bytes) so CJK
/// names get the same budget as ASCII ones.
const MAX_NAME_CHARS: usize = 40;

/// File under the shared dir holding the registry-private seq watermark.
pub(crate) const SEQ_STATE_FILE: &str = "companion_seq.json";

/// Registry-private high-watermark for companion short numbers: the largest seq
/// ever allocated on this machine, persisted as `{shared_dir}/companion_seq.json`
/// (`{"last_companion_seq": N}`). It deliberately does NOT live on
/// [`crate::profile::SharedCompanionConfig`]: that object is user-writable
/// wholesale (full-object `PUT /api/companion/config`, future import paths, …), so
/// keeping the watermark there would make "never reuse a deleted companion's
/// number" depend on every present and future config write path remembering
/// to clamp it. A missing/corrupt file self-heals as 0 — the allocation
/// formula additionally takes the largest live seq into account.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub(crate) struct CompanionSeqState {
    pub(crate) last_companion_seq: u64,
}

impl CompanionSeqState {
    /// Load from `{shared_dir}/companion_seq.json`, falling back to 0 when the
    /// file is missing or unreadable.
    pub(crate) fn load(shared_dir: &Path) -> Self {
        crate::fsio::load_json_or_default(&shared_dir.join(SEQ_STATE_FILE))
    }

    /// Atomically persist to `{shared_dir}/companion_seq.json`.
    pub(crate) fn save(&self, shared_dir: &Path) -> std::io::Result<()> {
        crate::fsio::save_json_atomic(shared_dir, SEQ_STATE_FILE, self)
    }
}

/// RFC 7396 JSON merge patch: objects merge recursively, `null` deletes,
/// everything else replaces.
pub(crate) fn json_merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    if let (Some(target_map), Some(patch_map)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch_map {
            if value.is_null() {
                target_map.remove(key);
            } else if value.is_object() && target_map.get(key).is_some_and(|t| t.is_object()) {
                json_merge_patch(target_map.get_mut(key).unwrap(), value);
            } else {
                target_map.insert(key.clone(), value.clone());
            }
        }
    } else {
        *target = patch.clone();
    }
}

/// Trimmed, non-empty, at most [`MAX_NAME_CHARS`] chars — or `BadRequest`.
fn validate_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("companion name must not be empty".into()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::BadRequest(format!(
            "companion name must be at most {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name.to_owned())
}

/// The largest seq carried by any companion in the map (0 when none carries one).
/// Lets allocation self-heal a stale/clobbered watermark while the
/// highest-numbered companion is still alive.
fn max_live_seq(companions: &HashMap<String, CompanionProfileConfig>) -> u64 {
    companions.values().filter_map(|p| p.seq).max().unwrap_or(0)
}

pub struct CompanionRegistry {
    companions_dir: PathBuf,
    /// Shared multi-companion home (`{data_dir}/companion/shared`) — where the seq
    /// watermark state file ([`SEQ_STATE_FILE`]) is persisted.
    shared_dir: PathBuf,
    /// In-memory seq watermark, mirrored to disk via [`CompanionSeqState`].
    /// Registry-owned and only ever advanced.
    ///
    /// Lock order: this lock is always acquired BEFORE the roster map below.
    watermark: RwLock<u64>,
    inner: RwLock<HashMap<String, CompanionProfileConfig>>,
}

impl CompanionRegistry {
    /// Synchronous boot-time scan: every subdirectory of `companions_dir` is loaded
    /// as a profile. Corrupt/missing configs (empty id sentinel) and dirs
    /// whose name does not match the embedded id are warned about and
    /// skipped — a broken profile must never brick boot or shadow a good one.
    ///
    /// Callers should follow up with [`backfill_missing_seqs`] once inside an
    /// async context to number any pre-seq profiles.
    ///
    /// [`backfill_missing_seqs`]: CompanionRegistry::backfill_missing_seqs
    pub fn scan(companions_dir: PathBuf, shared_dir: PathBuf) -> Self {
        let mut companions = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&companions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let dir_name = entry.file_name().to_string_lossy().into_owned();
                let profile = CompanionProfileConfig::load(&path);
                if profile.id.is_empty() || profile.id != dir_name {
                    tracing::warn!(
                        dir = %path.display(),
                        id = %profile.id,
                        "companion profile corrupt or id does not match its directory; skipping"
                    );
                    continue;
                }
                companions.insert(profile.id.clone(), profile);
            }
        }
        let watermark = CompanionSeqState::load(&shared_dir).last_companion_seq;
        Self {
            companions_dir,
            shared_dir,
            watermark: RwLock::new(watermark),
            inner: RwLock::new(companions),
        }
    }

    /// All companions, oldest first (`created_at` ascending, id as tie-break so the
    /// order is stable even for same-millisecond creations).
    pub async fn list(&self) -> Vec<CompanionProfileConfig> {
        let mut companions: Vec<CompanionProfileConfig> = self.inner.read().await.values().cloned().collect();
        companions.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        companions
    }

    /// Root of the per-companion directories (`{data_dir}/companion/companions`) — where
    /// non-config per-companion artifacts (e.g. the DIY figure image) live too.
    pub(crate) fn companions_dir(&self) -> &Path {
        &self.companions_dir
    }

    /// 伙伴工作区树根：companions_dir 的兄弟目录 `{data_dir}/companion/workspaces`。
    /// （companions_dir == `{data_dir}/companion/companions`，取 parent 再 join。）
    /// 见名知意的每伙伴工作目录落在此树下，与 home 目录解耦。
    pub(crate) fn workspaces_dir(&self) -> std::path::PathBuf {
        self.companions_dir
            .parent()
            .map(|p| p.join("workspaces"))
            .unwrap_or_else(|| self.companions_dir.join("workspaces"))
    }

    pub async fn get(&self, id: &str) -> Option<CompanionProfileConfig> {
        self.inner.read().await.get(id).cloned()
    }

    /// Companion ids in the same order as [`list`](Self::list).
    pub async fn ids(&self) -> Vec<String> {
        self.list().await.into_iter().map(|p| p.id).collect()
    }

    /// 解析"代表全家发声"的伙伴 id（单一事实源，learner 与 evolution 引擎共用）。
    /// 存活的显式默认体优先；否则首个注册伙伴；空 roster 返回空串。
    /// liveness 检查同时修掉"默认体已删除却仍被当 owner"的潜伏问题。
    pub async fn resolve_default(&self, default_companion_id: &str) -> String {
        let ids = self.ids().await;
        if !default_companion_id.is_empty() && ids.iter().any(|id| id == default_companion_id) {
            return default_companion_id.to_owned();
        }
        ids.into_iter().next().unwrap_or_default()
    }

    /// Create a companion: validate the name, allocate its short number from the
    /// registry watermark, persist `{companions_dir}/{id}/config.json`, then insert
    /// into the map under the write lock. Allocation and both saves happen
    /// inside one critical section so two concurrent creates can never mint
    /// the same number. The watermark is only advanced after the profile
    /// saved successfully — a failed create has zero persistent side effects,
    /// so retrying it never burns numbers.
    pub async fn create(&self, name: &str, character: &str) -> Result<CompanionProfileConfig, AppError> {
        let name = validate_name(name)?;
        let mut profile = CompanionProfileConfig::new(&name, character);
        let dir = self.companions_dir.join(&profile.id);
        // Lock order: watermark before the roster map (see struct docs).
        let mut watermark = self.watermark.write().await;
        let mut companions = self.inner.write().await;
        // Never reuse: one past the watermark or the largest live seq,
        // whichever is bigger.
        let seq = (*watermark).max(max_live_seq(&companions)) + 1;
        profile.seq = Some(seq);
        profile
            .save(&dir)
            .map_err(|e| AppError::Internal(format!("save companion profile: {e}")))?;
        self.advance_watermark(&mut watermark, seq);
        companions.insert(profile.id.clone(), profile.clone());
        Ok(profile)
    }

    /// Backfill short numbers for profiles written before the seq rollout
    /// (or minted by the legacy migration without one), oldest first
    /// (`created_at`, id as tie-break — the [`list`](Self::list) order).
    /// Idempotent: a companion that already carries a seq is never renumbered.
    /// Also heals a lagging watermark (e.g. a deleted/corrupt state file) by
    /// advancing it to the largest live seq. Meant to run once per boot,
    /// right after [`scan`](Self::scan).
    pub async fn backfill_missing_seqs(&self) {
        // Lock order: watermark before the roster map (see struct docs).
        let mut watermark = self.watermark.write().await;
        let mut companions = self.inner.write().await;
        let mut missing: Vec<(i64, String)> = companions
            .values()
            .filter(|p| p.seq.is_none())
            .map(|p| (p.created_at, p.id.clone()))
            .collect();
        missing.sort();
        let mut next = (*watermark).max(max_live_seq(&companions)) + 1;
        for (_, id) in missing {
            let Some(profile) = companions.get_mut(&id) else { continue };
            profile.seq = Some(next);
            if let Err(e) = profile.save(&self.companions_dir.join(&id)) {
                // The map must never claim state the disk doesn't have:
                // leave the profile unnumbered. And never hand its number to
                // a younger companion — seq is immutable once persisted, so that
                // would permanently invert the numbering against created_at
                // order. Stop numbering here instead: this companion and every
                // younger one retry, in order, on the next boot.
                tracing::warn!(
                    companion_id = %id, error = %e,
                    "backfill companion seq: save failed; deferring this and all younger companions to the next boot"
                );
                profile.seq = None;
                break;
            }
            next += 1;
        }
        let live_max = max_live_seq(&companions);
        self.advance_watermark(&mut watermark, live_max);
    }

    /// Advance the in-memory watermark to `seq` (never backwards) and
    /// persist the state file. A failed save only warns: the number also
    /// lives on the companion profile itself, so monotonicity survives through the
    /// live-max term until the next successful save.
    fn advance_watermark(&self, watermark: &mut u64, seq: u64) {
        if seq <= *watermark {
            return;
        }
        *watermark = seq;
        if let Err(e) = (CompanionSeqState { last_companion_seq: seq }).save(&self.shared_dir) {
            tracing::warn!(error = %e, "save companion seq watermark failed");
        }
    }

    /// RFC 7396 partial update of one profile. `id`, `seq` and `created_at`
    /// are immutable — whatever the patch says, they are restored from the
    /// current profile before saving.
    pub async fn patch(&self, id: &str, patch: serde_json::Value) -> Result<CompanionProfileConfig, AppError> {
        if !patch.is_object() {
            return Err(AppError::BadRequest("companion patch must be a JSON object".into()));
        }
        let mut companions = self.inner.write().await;
        let current = companions
            .get(id)
            .ok_or_else(|| AppError::NotFound(format!("companion '{id}' not found")))?;
        let mut value = serde_json::to_value(current)
            .map_err(|e| AppError::Internal(format!("serialize companion profile: {e}")))?;
        json_merge_patch(&mut value, &patch);
        let mut merged: CompanionProfileConfig = serde_json::from_value(value)
            .map_err(|e| AppError::BadRequest(format!("invalid companion patch: {e}")))?;
        merged.id = current.id.clone();
        merged.seq = current.seq;
        merged.created_at = current.created_at;
        merged.name = validate_name(&merged.name)?;
        merged
            .save(&self.companions_dir.join(&merged.id))
            .map_err(|e| AppError::Internal(format!("save companion profile: {e}")))?;
        companions.insert(merged.id.clone(), merged.clone());
        Ok(merged)
    }

    /// Remove a companion from the map and delete its directory (an already-missing
    /// directory is tolerated). Returns the removed profile.
    pub async fn remove(&self, id: &str) -> Result<CompanionProfileConfig, AppError> {
        let mut companions = self.inner.write().await;
        let profile = companions
            .remove(id)
            .ok_or_else(|| AppError::NotFound(format!("companion '{id}' not found")))?;
        match std::fs::remove_dir_all(self.companions_dir.join(id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(AppError::Internal(format!("remove companion dir: {e}"))),
        }
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry over `{dir}/companions` with its watermark state at
    /// `{dir}/shared/companion_seq.json` (the production sibling layout).
    fn scan_at(dir: &std::path::Path) -> CompanionRegistry {
        scan_companions_at(dir, "companions")
    }

    /// Same as [`scan_at`] but over `{dir}/{companions}`.
    fn scan_companions_at(dir: &std::path::Path, companions: &str) -> CompanionRegistry {
        CompanionRegistry::scan(dir.join(companions), dir.join("shared"))
    }

    fn registry(dir: &std::path::Path) -> CompanionRegistry {
        scan_at(dir)
    }

    #[test]
    fn merge_patch_merges_nested_and_replaces_scalars() {
        let mut base = serde_json::json!({
            "appearance": {"companion_enabled": false, "companion_x": 10, "quiet_start": ""},
            "learn": {"enabled": true, "interval_minutes": 60}
        });
        json_merge_patch(
            &mut base,
            &serde_json::json!({"appearance": {"companion_x": 99, "companion_y": 42}}),
        );
        assert_eq!(base["appearance"]["companion_x"], 99);
        assert_eq!(base["appearance"]["companion_y"], 42);
        assert_eq!(base["appearance"]["companion_enabled"], false);
        assert_eq!(base["learn"]["interval_minutes"], 60);
    }

    #[tokio::test]
    async fn resolve_default_prefers_alive_explicit_then_first() {
        let dir = tempfile::tempdir().unwrap();
        let reg = CompanionRegistry::scan(dir.path().join("companions"), dir.path().join("shared"));
        // 空 roster → 空串
        assert_eq!(reg.resolve_default("").await, "");
        let _a = reg.create("甲", "ink").await.unwrap();
        let b = reg.create("乙", "ink").await.unwrap();
        let first = reg.ids().await.into_iter().next().unwrap();
        // 显式默认体且存活 → 用之
        assert_eq!(reg.resolve_default(&b.id).await, b.id);
        // 显式默认体已删（不在 roster）→ 回退首个注册
        assert_eq!(reg.resolve_default("companion_ghost").await, first);
        // 空默认体 → 首个注册
        assert_eq!(reg.resolve_default("").await, first);
    }

    #[tokio::test]
    async fn create_persists_and_lists() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        assert!(reg.list().await.is_empty());

        let companion = reg.create("  毛球  ", "ink").await.unwrap();
        assert!(companion.id.starts_with("companion_"));
        assert_eq!(companion.name, "毛球"); // trimmed
        assert_eq!(companion.character, "ink");
        assert_eq!(companion.seq, Some(1));

        // Persisted on disk under {companions_dir}/{id}/config.json.
        let on_disk = CompanionProfileConfig::load(&dir.path().join("companions").join(&companion.id));
        assert_eq!(on_disk, companion);

        assert_eq!(reg.get(&companion.id).await.unwrap(), companion);
        assert_eq!(reg.ids().await, vec![companion.id.clone()]);
    }

    #[tokio::test]
    async fn list_sorts_by_created_at_ascending() {
        let dir = tempfile::tempdir().unwrap();
        let companions_dir = dir.path().join("companions");
        // Hand-build two profiles with crafted created_at, newer one first
        // alphabetically so the sort genuinely exercises created_at.
        let mut newer = CompanionProfileConfig::new("新宠", "boo");
        newer.created_at = 2_000;
        newer.save(&companions_dir.join(&newer.id)).unwrap();
        let mut older = CompanionProfileConfig::new("老宠", "mochi");
        older.created_at = 1_000;
        older.save(&companions_dir.join(&older.id)).unwrap();

        let reg = scan_at(dir.path());
        let listed = reg.list().await;
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, older.id);
        assert_eq!(listed[1].id, newer.id);
        assert_eq!(reg.ids().await, vec![older.id, newer.id]);
    }

    #[tokio::test]
    async fn name_validation_rejects_empty_and_over_40_chars() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());

        assert!(matches!(reg.create("", "ink").await, Err(AppError::BadRequest(_))));
        assert!(matches!(reg.create("   ", "ink").await, Err(AppError::BadRequest(_))));

        // 40 chars (counted in chars, not bytes) is fine, 41 is not.
        let ok = "宠".repeat(40);
        let too_long = "宠".repeat(41);
        let companion = reg.create(&ok, "ink").await.unwrap();
        assert_eq!(companion.name.chars().count(), 40);
        assert!(matches!(reg.create(&too_long, "ink").await, Err(AppError::BadRequest(_))));

        // patch enforces the same rules.
        let err = reg.patch(&companion.id, serde_json::json!({"name": too_long})).await;
        assert!(matches!(err, Err(AppError::BadRequest(_))));
        let err = reg.patch(&companion.id, serde_json::json!({"name": "  "})).await;
        assert!(matches!(err, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn patch_renames_but_never_changes_id_or_created_at() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        let companion = reg.create("旧名", "ink").await.unwrap();

        let patched = reg
            .patch(
                &companion.id,
                serde_json::json!({
                    "name": "新名",
                    "id": "companion_evil",
                    "seq": 99,
                    "created_at": 1,
                    "appearance": {"companion_enabled": true, "companion_x": 7}
                }),
            )
            .await
            .unwrap();
        assert_eq!(patched.id, companion.id);
        assert_eq!(patched.seq, companion.seq, "seq is immutable through patches");
        assert_eq!(patched.created_at, companion.created_at);
        assert_eq!(patched.name, "新名");
        assert!(patched.appearance.companion_enabled);
        assert_eq!(patched.appearance.companion_x, Some(7));
        // Untouched fields survive the merge.
        assert_eq!(patched.character, "ink");

        // Persisted and visible through the map.
        let on_disk = CompanionProfileConfig::load(&dir.path().join("companions").join(&companion.id));
        assert_eq!(on_disk, patched);
        assert_eq!(reg.get(&companion.id).await.unwrap(), patched);

        assert!(matches!(
            reg.patch("companion_missing", serde_json::json!({"name": "x"})).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            reg.patch(&companion.id, serde_json::json!(42)).await,
            Err(AppError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn remove_deletes_dir_and_returns_profile() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        let companion = reg.create("一郎", "ink").await.unwrap();
        let keep = reg.create("二郎", "boo").await.unwrap();
        let companion_dir = dir.path().join("companions").join(&companion.id);
        assert!(companion_dir.exists());

        let removed = reg.remove(&companion.id).await.unwrap();
        assert_eq!(removed.id, companion.id);
        assert!(!companion_dir.exists());
        assert!(reg.get(&companion.id).await.is_none());
        assert!(reg.get(&keep.id).await.is_some());

        assert!(matches!(reg.remove(&companion.id).await, Err(AppError::NotFound(_))));

        // An already-missing directory is tolerated.
        std::fs::remove_dir_all(dir.path().join("companions").join(&keep.id)).unwrap();
        let removed = reg.remove(&keep.id).await.unwrap();
        assert_eq!(removed.id, keep.id);
    }

    #[tokio::test]
    async fn scan_skips_corrupt_and_mismatched_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let companions_dir = dir.path().join("companions");

        // Good profile in a dir matching its id.
        let good = CompanionProfileConfig::new("好宠", "ink");
        good.save(&companions_dir.join(&good.id)).unwrap();
        // Corrupt config.json -> empty-id sentinel -> skipped.
        let corrupt_dir = companions_dir.join("companion_corrupt");
        std::fs::create_dir_all(&corrupt_dir).unwrap();
        std::fs::write(corrupt_dir.join("config.json"), "{not json").unwrap();
        // Valid profile but stored in a dir that doesn't match its id.
        let homeless = CompanionProfileConfig::new("流浪", "boo");
        homeless.save(&companions_dir.join("companion_wrong_home")).unwrap();
        // Empty dir (no config.json at all).
        std::fs::create_dir_all(companions_dir.join("companion_empty")).unwrap();
        // Stray file at the top level.
        std::fs::write(companions_dir.join("stray.txt"), "?").unwrap();

        let reg = scan_at(dir.path());
        assert_eq!(reg.ids().await, vec![good.id.clone()]);
        assert_eq!(reg.get(&good.id).await.unwrap(), good);

        // A missing companions dir scans to an empty registry.
        let empty = scan_companions_at(dir.path(), "nonexistent");
        assert!(empty.list().await.is_empty());
    }

    #[tokio::test]
    async fn create_allocates_monotonic_seq_never_reusing_deleted_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());

        let first = reg.create("一号", "ink").await.unwrap();
        let second = reg.create("二号", "boo").await.unwrap();
        assert_eq!(first.seq, Some(1));
        assert_eq!(second.seq, Some(2));

        // Deleting the highest-numbered companion must not free its number.
        reg.remove(&second.id).await.unwrap();
        let third = reg.create("三号", "mochi").await.unwrap();
        assert_eq!(third.seq, Some(3));

        // The watermark is persisted in the registry's own state file (never
        // in the user-writable shared config, which the registry must not
        // touch at all)…
        assert_eq!(CompanionSeqState::load(&dir.path().join("shared")).last_companion_seq, 3);
        assert!(!crate::profile::SharedCompanionConfig::config_path(&dir.path().join("shared")).exists());
        // …and the number on the profile itself.
        let on_disk = CompanionProfileConfig::load(&dir.path().join("companions").join(&third.id));
        assert_eq!(on_disk.seq, Some(3));

        // A rescan (fresh process) keeps counting past the watermark even
        // when the highest-numbered companion is gone.
        reg.remove(&third.id).await.unwrap();
        let reg2 = registry(dir.path());
        let fourth = reg2.create("四号", "boo").await.unwrap();
        assert_eq!(fourth.seq, Some(4));
    }

    #[tokio::test]
    async fn backfill_numbers_unnumbered_companions_by_created_at_and_keeps_existing() {
        let dir = tempfile::tempdir().unwrap();
        let companions_dir = dir.path().join("companions");
        // Two pre-rollout profiles (no seq), saved newest-first so the
        // backfill order genuinely follows created_at, plus one companion that
        // already carries a number.
        let mut newer = CompanionProfileConfig::new("新宠", "boo");
        newer.created_at = 2_000;
        newer.save(&companions_dir.join(&newer.id)).unwrap();
        let mut older = CompanionProfileConfig::new("老宠", "mochi");
        older.created_at = 1_000;
        older.save(&companions_dir.join(&older.id)).unwrap();
        let mut numbered = CompanionProfileConfig::new("有号", "ink");
        numbered.created_at = 1_500;
        numbered.seq = Some(5);
        numbered.save(&companions_dir.join(&numbered.id)).unwrap();

        let reg = scan_at(dir.path());
        reg.backfill_missing_seqs().await;

        // Missing numbers continue past the largest live one, oldest first;
        // an already-numbered companion is never renumbered.
        assert_eq!(reg.get(&older.id).await.unwrap().seq, Some(6));
        assert_eq!(reg.get(&newer.id).await.unwrap().seq, Some(7));
        assert_eq!(reg.get(&numbered.id).await.unwrap().seq, Some(5));
        // Persisted to each profile's config.json and to the watermark.
        assert_eq!(CompanionProfileConfig::load(&companions_dir.join(&older.id)).seq, Some(6));
        assert_eq!(CompanionProfileConfig::load(&companions_dir.join(&newer.id)).seq, Some(7));
        assert_eq!(CompanionSeqState::load(&dir.path().join("shared")).last_companion_seq, 7);

        // Idempotent: a second run changes no number.
        reg.backfill_missing_seqs().await;
        assert_eq!(reg.get(&older.id).await.unwrap().seq, Some(6));
        assert_eq!(reg.get(&newer.id).await.unwrap().seq, Some(7));
        assert_eq!(reg.get(&numbered.id).await.unwrap().seq, Some(5));
    }

    #[tokio::test]
    async fn failed_create_does_not_advance_watermark() {
        let dir = tempfile::tempdir().unwrap();
        // A regular file where the companions dir should be makes every profile
        // save fail (create_dir_all over a file errors on all platforms).
        std::fs::write(dir.path().join("companions"), "blocker").unwrap();
        let reg = scan_at(dir.path());

        assert!(matches!(reg.create("一号", "ink").await, Err(AppError::Internal(_))));
        // Zero persistent side effects: no watermark advanced, empty roster.
        assert_eq!(CompanionSeqState::load(&dir.path().join("shared")).last_companion_seq, 0);
        assert!(reg.list().await.is_empty());

        // The retry (after the cause is fixed) still gets #1 — a failed
        // create burns no number.
        std::fs::remove_file(dir.path().join("companions")).unwrap();
        let companion = reg.create("一号", "ink").await.unwrap();
        assert_eq!(companion.seq, Some(1));
        assert_eq!(CompanionSeqState::load(&dir.path().join("shared")).last_companion_seq, 1);
    }

    #[tokio::test]
    async fn backfill_save_failure_stops_instead_of_renumbering_younger_companions() {
        let dir = tempfile::tempdir().unwrap();
        let companions_dir = dir.path().join("companions");
        let mut a = CompanionProfileConfig::new("老大", "ink");
        a.created_at = 1_000;
        a.save(&companions_dir.join(&a.id)).unwrap();
        let mut b = CompanionProfileConfig::new("老二", "boo");
        b.created_at = 2_000;
        b.save(&companions_dir.join(&b.id)).unwrap();
        let mut c = CompanionProfileConfig::new("老三", "mochi");
        c.created_at = 3_000;
        c.save(&companions_dir.join(&c.id)).unwrap();

        let reg = scan_at(dir.path());
        // Break the middle companion's home: a regular file at its dir path makes
        // (only) its save fail.
        std::fs::remove_dir_all(companions_dir.join(&b.id)).unwrap();
        std::fs::write(companions_dir.join(&b.id), "blocker").unwrap();

        reg.backfill_missing_seqs().await;

        // A got #1; B's save failed; C must NOT take #2 — numbering stops at
        // the failure so the created_at order survives to the next retry
        // (seq is immutable, a swap would be permanent).
        assert_eq!(reg.get(&a.id).await.unwrap().seq, Some(1));
        assert_eq!(reg.get(&b.id).await.unwrap().seq, None);
        assert_eq!(reg.get(&c.id).await.unwrap().seq, None);
        assert_eq!(CompanionSeqState::load(&dir.path().join("shared")).last_companion_seq, 1);

        // Next boot (cause fixed): B and C get #2/#3, still in age order.
        std::fs::remove_file(companions_dir.join(&b.id)).unwrap();
        reg.backfill_missing_seqs().await;
        assert_eq!(reg.get(&b.id).await.unwrap().seq, Some(2));
        assert_eq!(reg.get(&c.id).await.unwrap().seq, Some(3));
        assert_eq!(CompanionSeqState::load(&dir.path().join("shared")).last_companion_seq, 3);
    }
}
