//! One-shot relocation of the legacy temp-rooted data dir to the per-user
//! application-data location.
//!
//! Historic builds defaulted the data dir to `<system temp>/nomifun-data/Nomi`
//! — anything that cleans the temp dir (OS cleanup, reboot on some distros,
//! disk-cleanup tools) silently destroyed all user data. The default now
//! resolves through `dirs::data_local_dir()` (see `main.rs`); this module
//! moves an existing temp-rooted install to the new default exactly once.
//!
//! Crash-safety follows the staging + marker pattern of
//! `nomifun-companion/src/migrate.rs`, hardened with a cross-process lock and a
//! live-writer probe:
//!
//! 1. **Gate** — skip when `NOMIFUN_DATA_DIR` is set (checked by the caller in
//!    [`effective_data_dir`]), when the new root already has a database, or
//!    when the legacy root has no database (a cleaned temp dir must be
//!    tolerated, it is the whole point of this exercise). The gate keys on
//!    the **`.db` only**: a relocation marker in the new root *without* the
//!    database is the signature of a commit that was interrupted between the
//!    marker rename and the db-family rename — the stale marker is deleted
//!    and the relocation re-runs from scratch. (`.relocated-done` without a
//!    db is anomalous but handled the same way: delete and re-run.)
//! 2. **Lock** — take an exclusive lock file (`.relocating.lock`,
//!    `create_new`) in the new root. Relocation runs *before* Tauri's
//!    single-instance plugin, so a double-launch (or a still-running old
//!    instance) could otherwise race two copies and adopt a torn one. If the
//!    lock is held, this launch skips the relocation and starts from the
//!    legacy dir (retried next launch). A lock file older than one hour is
//!    considered abandoned by a dead process and is preempted — immediately
//!    so when probe residue (step 3) proves its holder died mid-probe.
//! 3. **Probe** — rename the legacy `nomifun-backend.db` to itself +
//!    `.probe` and back. On Windows a file held open by SQLite (no
//!    `FILE_SHARE_DELETE`) cannot be renamed, so a rename failure means an
//!    older instance is still writing — skip the relocation this launch and
//!    start from the legacy dir. Probe residue from a crash in this window
//!    is healed on the next run (see [`heal_probe_residue`]).
//! 4. **Stage** — copy (never move) every keeper entry into
//!    `<new>/.relocating/`; the legacy dir stays complete the whole time.
//!    Regenerable trees ([`EXCLUDED_ENTRIES`]) are skipped. Symlinks and
//!    NTFS junctions (workspace `.claude/skills/*` and `.nomi/knowledge/*`
//!    links) are skipped too — see [`copy_dir_recursive`].
//! 5. **Commit** — rename staged entries into the new root: ordinary entries
//!    first, then the `.relocated-from` marker, then the database family with
//!    the bare `.db` last. The gate keys on that `.db`, so a crash anywhere
//!    earlier simply re-runs the relocation from scratch; once the `.db` is in
//!    place the marker is guaranteed to already be there for the backend's
//!    path rewrite (`nomifun_app::bootstrap::rewrite_relocated_paths`).
//! 6. **Legacy dir is kept** as a backup for at least one release cycle.
//!
//! Any failure falls back to starting from the legacy dir — never block the
//! launch, never leave a half-adopted new root: a failed attempt sweeps its
//! staging scraps AND any `.relocated-from` marker it may have planted
//! (only while the new root has no `.db` — an adopted root keeps its marker
//! for the backend rewrite).
//!
//! Outcomes are queued via `nomifun_app::bootstrap::record_boot_note` and
//! logged by the backend right after tracing comes up, so the result is
//! visible in `{data_dir}/logs/` and not only on the (usually invisible)
//! stderr.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nomifun_app::bootstrap::{
    BootNoteLevel, RELOCATED_DONE_MARKER, RELOCATED_FROM_MARKER, RelocationMarker, record_boot_note,
};

/// The main database; its presence defines "this root holds real data".
const DB_FILE: &str = "nomifun-backend.db";

/// The db plus its WAL sidecars (present only after an unclean shutdown).
/// Staged together, renamed into place together, bare `.db` last.
const DB_FILES: [&str; 3] = ["nomifun-backend.db", "nomifun-backend.db-wal", "nomifun-backend.db-shm"];

/// Scratch dir inside the NEW root where everything is staged before the
/// commit renames. Residue from a crashed run is wiped and redone — safe,
/// because staging only ever holds copies.
const STAGING_DIR_NAME: &str = ".relocating";

/// Exclusive lock file in the NEW root guarding the whole relocation against
/// concurrent processes (started before tauri single-instance can help).
const LOCK_FILE_NAME: &str = ".relocating.lock";

/// A lock file older than this is treated as left behind by a dead process
/// and preempted. Deliberately simple (mtime-based): a live relocation takes
/// seconds, not an hour.
const STALE_LOCK_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Suffix of the transient probe rename of the legacy db (see
/// [`legacy_db_quiescent`]).
const PROBE_SUFFIX: &str = ".probe";

/// Entries that are regenerable (re-extracted, re-materialized or disposable)
/// and therefore deliberately NOT relocated.
const EXCLUDED_ENTRIES: &[&str] = &[
    "runtime",                     // bundled bun, re-extracted on launch
    "bun-cache",                   // bun install cache
    "bun-tmp",                     // bun scratch space
    "builtin-skills",              // re-materialized from embedded assets
    "logs",                        // tracing output
    "preview-history",             // office preview snapshots
    "nomi-sessions",               // nomi agent scratch sessions
    "nomi-health-check-sessions",  // provider health-check scratch
    "browser-profile",             // legacy chromium profile (pre-Playwright-MCP);
                                   // no longer created — kept to relocate/clean
                                   // residue from upgraded installs
    "nomifun-backend.db.migrate.lock", // advisory lock, must stay with its db
    "nomifun-backend.db.probe",    // transient quiescence-probe residue
    "server.lock",                 // per-data-dir server lock: lives on the open
                                   // handle, not the file; copying it can hit the
                                   // holder's LockFileEx range on Windows
    "server.lock.info",            // holder breadcrumb, stale outside its dir
];

#[derive(Debug, PartialEq, Eq)]
pub enum RelocateOutcome {
    /// Data was copied to the new root; the legacy dir is kept as a backup.
    Relocated,
    /// Old and new root are the same path (e.g. `dirs` fell back to temp).
    SkippedSameDir,
    /// The new root already has a database (with or without markers).
    SkippedTargetInUse,
    /// The legacy root does not exist or holds no database (cleaned temp).
    SkippedNoLegacyData,
    /// Another process holds the relocation lock; start from the legacy dir
    /// this launch and retry on the next one.
    SkippedLockHeld,
    /// The legacy database is held open by a live writer (an old instance
    /// still running); start from the legacy dir and retry next launch.
    SkippedLegacyDbBusy,
}

/// Whether legacy-temp relocation should run at all. Skipped when the data dir
/// is explicitly overridden (`NOMIFUN_DATA_DIR`) OR when this is a non-stable
/// channel: dev/beta/canary run from an isolated sibling dir
/// (`…/NomiFun/Nomi-dev`) and must NOT adopt the legacy temp-rooted *stable*
/// install, which would cross-contaminate the very state we isolated. Pure, for
/// unit testing (the live channel is compile-time, so the predicate is tested
/// directly rather than through `effective_data_dir`).
fn should_relocate(env_override: bool, is_stable: bool) -> bool {
    !env_override && is_stable
}

/// Resolve the data dir the app should actually use, relocating a legacy
/// temp-rooted install into `default_dir` when needed. Never fails: on any
/// relocation error the app starts from the (still intact) legacy dir,
/// exactly as previous builds did.
pub fn effective_data_dir(default_dir: PathBuf) -> PathBuf {
    // Skip relocation entirely under an explicit data-dir override or a
    // non-stable channel — either way the legacy stable install must not be
    // adopted into this root.
    let env_override = std::env::var_os("NOMIFUN_DATA_DIR").is_some();
    if !should_relocate(env_override, nomifun_app::channel::is_stable()) {
        return default_dir;
    }

    let legacy_root = std::env::temp_dir().join("nomifun-data").join("Nomi");
    match relocate(&legacy_root, &default_dir) {
        Ok(RelocateOutcome::Relocated) => {
            boot_note(
                BootNoteLevel::Info,
                format!(
                    "data relocated {} -> {} (legacy dir kept as backup)",
                    legacy_root.display(),
                    default_dir.display()
                ),
            );
            default_dir
        }
        Ok(RelocateOutcome::SkippedLockHeld) => {
            // The gate proved: legacy db present, new db absent. Without the
            // lock this launch must not touch the new root — run from the
            // legacy data and let a later launch finish the relocation.
            boot_note(
                BootNoteLevel::Warn,
                format!(
                    "data relocation skipped: another process holds {} — starting from legacy dir {} (retried next launch)",
                    default_dir.join(LOCK_FILE_NAME).display(),
                    legacy_root.display()
                ),
            );
            legacy_root
        }
        Ok(RelocateOutcome::SkippedLegacyDbBusy) => {
            boot_note(
                BootNoteLevel::Warn,
                format!(
                    "data relocation skipped: legacy database is held by a running instance — starting from legacy dir {} (retried next launch)",
                    legacy_root.display()
                ),
            );
            legacy_root
        }
        // Steady states (already relocated, nothing to relocate, same dir):
        // silent, this is every normal boot.
        Ok(_) => default_dir,
        Err(e) => {
            // relocate() already swept its scraps (staging + unadopted marker).
            boot_note(
                BootNoteLevel::Warn,
                format!(
                    "data relocation failed ({e}); starting from legacy dir {}",
                    legacy_root.display()
                ),
            );
            legacy_root
        }
    }
}

/// Stderr immediately (visible in dev) + queued for the backend log (visible
/// in production, where stderr goes nowhere — see module docs).
fn boot_note(level: BootNoteLevel, message: String) {
    eprintln!("nomifun-desktop: {message}");
    record_boot_note(level, message);
}

/// Copy a legacy data root into a new root (gate + lock + probe + staging +
/// ordered commit). See the module docs for the crash-window reasoning. On
/// error, scraps planted in the new root are swept before returning.
fn relocate(old_root: &Path, new_root: &Path) -> std::io::Result<RelocateOutcome> {
    let outcome = relocate_inner(old_root, new_root);
    if outcome.is_err() {
        cleanup_failed_relocation(new_root);
    }
    outcome
}

fn relocate_inner(old_root: &Path, new_root: &Path) -> std::io::Result<RelocateOutcome> {
    if old_root == new_root {
        return Ok(RelocateOutcome::SkippedSameDir);
    }
    // The gate keys on the `.db` ONLY. Markers without a database are residue
    // of an interrupted commit and are cleaned up below — treating them as
    // "target in use" would strand the user on a brand-new empty database
    // while their real data sits one rename short in the legacy dir.
    if new_root.join(DB_FILE).exists() {
        return Ok(RelocateOutcome::SkippedTargetInUse);
    }
    // Crash residue of the quiescence probe counts as legacy data (the db is
    // merely misnamed); it is healed under the lock below.
    let old_db = old_root.join(DB_FILE);
    if !old_db.exists() && !probe_path(&old_db).exists() {
        return Ok(RelocateOutcome::SkippedNoLegacyData);
    }

    // ----- lock: exclusive across processes -----
    std::fs::create_dir_all(new_root)?;
    // Probe residue without the db proves the previous relocation died
    // INSIDE the probe's rename window — which sits in the lock's critical
    // section, so the lock file it left behind is dead no matter how fresh
    // its mtime is. Preempt immediately: deferring to the legacy root would
    // boot a legacy dir whose db is still misnamed `.probe`, and the backend
    // would plant a fresh empty database there.
    let stale_after = if !old_db.exists() && probe_path(&old_db).exists() {
        Duration::ZERO
    } else {
        STALE_LOCK_MAX_AGE
    };
    let Some(_lock) = try_acquire_relocation_lock(new_root, stale_after)? else {
        return Ok(RelocateOutcome::SkippedLockHeld);
    };
    // `_lock` (RAII) releases on every exit below, including errors. A crash
    // leaves the file behind; the next launch within the stale window starts
    // from the legacy dir, after it the lock is preempted.

    // Heal probe residue only while holding the lock — a concurrent healer
    // could otherwise race the probing process's rename-back.
    heal_probe_residue(old_root)?;
    if !old_db.exists() {
        return Ok(RelocateOutcome::SkippedNoLegacyData);
    }

    // Marker cleanup must also happen under the lock: a concurrent process
    // mid-commit has its marker in place before its `.db` — deleting it from
    // outside the lock would break that instance's backend path rewrite.
    for marker_name in [RELOCATED_FROM_MARKER, RELOCATED_DONE_MARKER] {
        let marker = new_root.join(marker_name);
        if marker.exists() {
            std::fs::remove_file(&marker)?;
        }
    }

    // ----- probe: skip while an old instance is still writing the db -----
    if !legacy_db_quiescent(&old_db)? {
        return Ok(RelocateOutcome::SkippedLegacyDbBusy);
    }

    // ----- stage: copy keepers, sources untouched -----
    let staging = new_root.join(STAGING_DIR_NAME);
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    let mut entries: Vec<(OsString, bool, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(old_root)? {
        let entry = entry?;
        let name = entry.file_name();
        if is_excluded(&name) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            // See copy_dir_recursive: links are rebuilt by their owners.
            continue;
        }
        entries.push((name, file_type.is_dir(), entry.path()));
    }
    // Stage the database family FIRST: the quiescence probe just proved no
    // writer holds it, and capturing the db bytes immediately minimizes the
    // window in which a concurrently launched instance (running from the
    // legacy root after a lock-held skip) could begin writing mid-copy.
    entries.sort_by_key(|(name, _, _)| !is_db_file(name));

    let mut staged: Vec<OsString> = Vec::new();
    for (name, is_dir, path) in entries {
        let target = staging.join(&name);
        if is_dir {
            copy_dir_recursive(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
        staged.push(name);
    }

    // The marker rides through staging like everything else, so it can never
    // appear in the new root ahead of the data it describes.
    let marker = RelocationMarker {
        old_root: old_root.display().to_string(),
        relocated_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
    };
    let marker_json = serde_json::to_vec(&marker).map_err(std::io::Error::other)?;
    std::fs::write(staging.join(RELOCATED_FROM_MARKER), marker_json)?;

    // ----- commit: ordinary entries → marker → db family (.db last) -----
    // Pre-existing targets can only be scraps of a crashed earlier attempt
    // (the gate proved there is no `.db` in the new root), so replacing them
    // with the fresh copies is always correct.
    let (db_family, others): (Vec<_>, Vec<_>) = staged.into_iter().partition(|name| is_db_file(name));
    for name in others {
        move_into_place(&staging.join(&name), &new_root.join(&name))?;
    }
    move_into_place(&staging.join(RELOCATED_FROM_MARKER), &new_root.join(RELOCATED_FROM_MARKER))?;
    for file in ["nomifun-backend.db-shm", "nomifun-backend.db-wal", DB_FILE] {
        if db_family.iter().any(|name| name.as_os_str() == OsStr::new(file)) {
            move_into_place(&staging.join(file), &new_root.join(file))?;
        }
    }

    // Scratch cleanup is best-effort: the `.db` already gates re-runs.
    let _ = std::fs::remove_dir_all(&staging);
    Ok(RelocateOutcome::Relocated)
}

/// Best-effort sweep after a failed attempt: staging scraps, and the
/// `.relocated-from` marker — but ONLY while the new root holds no `.db`.
/// Once the `.db` is in place the root is adopted and its marker must
/// survive for the backend's path rewrite (and no error path can run after
/// the `.db` rename anyway; this is defense in depth).
fn cleanup_failed_relocation(new_root: &Path) {
    let _ = std::fs::remove_dir_all(new_root.join(STAGING_DIR_NAME));
    if !new_root.join(DB_FILE).exists() {
        let _ = std::fs::remove_file(new_root.join(RELOCATED_FROM_MARKER));
    }
}

/// Held for the duration of a relocation attempt; the lock file is removed
/// on drop (success, failure, or unwind). After a hard crash the file stays
/// and is preempted once older than the staleness window.
struct RelocationLock {
    path: PathBuf,
}

impl Drop for RelocationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Try to take the exclusive relocation lock in `new_root`.
///
/// * `Ok(Some(_))` — acquired (possibly after preempting a lock file whose
///   mtime is older than `stale_after`).
/// * `Ok(None)` — a fresh lock exists: another live process is relocating
///   (or one died less than `stale_after` ago — acceptably conservative,
///   that launch simply runs from the legacy dir).
/// * `Err(_)` — filesystem trouble other than contention.
fn try_acquire_relocation_lock(new_root: &Path, stale_after: Duration) -> std::io::Result<Option<RelocationLock>> {
    let path = new_root.join(LOCK_FILE_NAME);
    for attempt in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(Some(RelocationLock { path })),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if attempt > 0 {
                    // We preempted once and someone re-created it first:
                    // genuine live contention.
                    return Ok(None);
                }
                let stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|mtime| std::time::SystemTime::now().duration_since(mtime).ok())
                    .map(|age| age >= stale_after)
                    // Unreadable mtime / clock skew into the future → treat
                    // as fresh (conservative: skip, retry next launch).
                    .unwrap_or(false);
                if !stale {
                    return Ok(None);
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    // Another preemptor got there first; loop and race for
                    // create_new — exactly one process wins.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Ok(None),
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// `<db>.probe` — sibling path used by the rename probe.
fn probe_path(db: &Path) -> PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push(PROBE_SUFFIX);
    db.with_file_name(name)
}

/// Active-writer probe: rename the legacy db to `<db>.probe` and back.
///
/// SQLite keeps its database handle open for the process lifetime and (on
/// Windows) without `FILE_SHARE_DELETE`, so the first rename fails while an
/// old instance is still running → `Ok(false)`, skip this launch. On Unix a
/// rename succeeds regardless (POSIX semantics); there the lock file is the
/// effective guard and this probe is a cheap no-op.
///
/// The rename-back is retried (a transient scanner can pin the file for a
/// moment); if it keeps failing the bytes are restored via copy and the db
/// is reported busy. A hard crash exactly between the two renames leaves
/// `<db>.probe` behind — [`heal_probe_residue`] renames it back on the next
/// run before anything else looks at the legacy root.
fn legacy_db_quiescent(old_db: &Path) -> std::io::Result<bool> {
    let probe = probe_path(old_db);
    if std::fs::rename(old_db, &probe).is_err() {
        return Ok(false);
    }
    let mut restore_err: Option<std::io::Error> = None;
    for _ in 0..10 {
        match std::fs::rename(&probe, old_db) {
            Ok(()) => return Ok(true),
            Err(e) => {
                restore_err = Some(e);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    // Something IS touching the file: restore the canonical name by copy
    // (succeeds alongside a share-read holder) and report busy — do not
    // migrate while a third party interferes.
    match std::fs::copy(&probe, old_db) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(false)
        }
        // Unrestorable (next launch heals via heal_probe_residue): surface
        // the rename error rather than guessing.
        Err(_) => Err(restore_err.expect("restore loop ran at least once")),
    }
}

/// Undo residue of a crash inside [`legacy_db_quiescent`]'s rename window:
/// if the legacy db only exists under its `.probe` name, rename it back.
///
/// A probe NEXT TO an existing db (crash between the restore-copy and the
/// probe unlink in `legacy_db_quiescent` — contents identical at that point)
/// is deliberately left untouched: deleting a data-bearing file on a
/// heuristic is never worth the tidiness. It is excluded from staging
/// ([`EXCLUDED_ENTRIES`]) and harmless.
fn heal_probe_residue(old_root: &Path) -> std::io::Result<()> {
    let old_db = old_root.join(DB_FILE);
    let probe = probe_path(&old_db);
    if probe.exists() && !old_db.exists() {
        std::fs::rename(&probe, &old_db)?;
    }
    Ok(())
}

fn is_excluded(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    // `.relocat*` defends against a legacy root that was itself a relocation
    // target in some earlier experiment (and covers the lock file).
    EXCLUDED_ENTRIES.iter().any(|e| name == *e) || name.starts_with(".relocat")
}

/// Member of the database family ([`DB_FILES`]): staged first, committed last.
fn is_db_file(name: &OsStr) -> bool {
    DB_FILES.iter().any(|f| name == OsStr::new(f))
}

/// Remove whatever occupies `path` — file, directory, or link — without
/// following links. Missing path is fine.
fn remove_existing(path: &Path) -> std::io::Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let file_type = meta.file_type();
    if file_type.is_dir() {
        std::fs::remove_dir_all(path)
    } else if file_type.is_symlink() {
        // Windows: directory links (junctions / dir-symlinks) are removed
        // with remove_dir, file symlinks with remove_file. Unix: remove_file.
        std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path))
    } else {
        std::fs::remove_file(path)
    }
}

/// `fs::rename` with target displacement and a copy+remove fallback (rename
/// can fail across volumes or when Windows holds a mapping on the source).
fn move_into_place(from: &Path, to: &Path) -> std::io::Result<()> {
    remove_existing(to)?;
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            let file_type = std::fs::symlink_metadata(from)?.file_type();
            if file_type.is_symlink() {
                // Same rationale as copy_dir_recursive: a link that even
                // rename refused is skipped, not copied. Drop the staged
                // link so the staging sweep can finish.
                let _ = std::fs::remove_dir(from).or_else(|_| std::fs::remove_file(from));
                Ok(())
            } else if file_type.is_dir() {
                copy_dir_recursive(from, to)?;
                std::fs::remove_dir_all(from)
            } else {
                std::fs::copy(from, to)?;
                std::fs::remove_file(from)
            }
        }
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        // Symlinks and NTFS junctions are SKIPPED, not copied. The legacy
        // tree contains them by design: `conversations/<ws>/.claude/skills/*`
        // (junctions made by nomifun-extension's skill_service via
        // junction::create) and `<ws>/.nomi/knowledge/*` mounts (plus their
        // pre-rename `<ws>/.nomifun/knowledge/*` leftovers — equally links,
        // equally skipped; the mount engine sweeps them at next sync). On
        // Windows a junction reports `is_symlink() == true` and
        // `is_dir() == false`, so it would land in the `fs::copy` branch and
        // fail the whole staging pass. Skipping is safe: these links are
        // re-created idempotently by ensure_auto_workspace_skill_links /
        // ensure_mounts_for_target the next time a task starts in that
        // workspace, and the link TARGETS (the real skill/knowledge dirs)
        // are relocated as ordinary directories.
        if file_type.is_symlink() {
            continue;
        }
        let target = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocates_only_for_stable_channel_without_override() {
        // The whole truth table: relocation runs ONLY when stable AND no override.
        assert!(should_relocate(false, true), "stable + no override → relocate");
        assert!(!should_relocate(true, true), "explicit override → skip");
        assert!(!should_relocate(false, false), "non-stable channel → skip (isolated dir)");
        assert!(!should_relocate(true, false), "override + non-stable → skip");
    }

    /// A legacy data root: db + WAL sidecar, migratable domains, and a few
    /// regenerable trees that must be left behind.
    fn build_legacy(root: &Path) {
        std::fs::create_dir_all(root.join("conversations").join("conv_1")).unwrap();
        std::fs::write(root.join("conversations").join("conv_1").join("notes.md"), "notes").unwrap();
        std::fs::create_dir_all(root.join("companion").join("shared")).unwrap();
        std::fs::write(root.join("companion").join("shared").join("memory.db"), "companion-db").unwrap();
        std::fs::create_dir_all(root.join("knowledge").join("kb_1")).unwrap();
        std::fs::write(root.join("knowledge").join("kb_1").join("doc.md"), "doc").unwrap();
        std::fs::write(root.join(DB_FILE), "main-db").unwrap();
        std::fs::write(root.join("nomifun-backend.db-wal"), "wal").unwrap();
        std::fs::write(root.join("encryption_key"), "key").unwrap();
        // Regenerable: must not be copied.
        std::fs::create_dir_all(root.join("runtime")).unwrap();
        std::fs::write(root.join("runtime").join("bun.exe"), "bun").unwrap();
        std::fs::create_dir_all(root.join("logs")).unwrap();
        std::fs::write(root.join("logs").join("app.log"), "log").unwrap();
        std::fs::create_dir_all(root.join("bun-cache")).unwrap();
        std::fs::write(root.join("nomifun-backend.db.migrate.lock"), "").unwrap();
    }

    /// Directory link helper mirroring production primitives: NTFS junction
    /// on Windows (what skill_service creates — no privilege needed), plain
    /// dir symlink on Unix. Returns false when the platform/CI sandbox
    /// refuses link creation, so callers can skip (same pragmatic pattern as
    /// nomifun-extension's symlink tests).
    #[cfg(windows)]
    fn create_dir_link(target: &Path, link: &Path) -> bool {
        junction::create(target, link).is_ok()
    }
    #[cfg(unix)]
    fn create_dir_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[test]
    fn relocates_and_keeps_legacy_as_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);

        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::Relocated);

        // Data arrived.
        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "main-db");
        assert_eq!(std::fs::read_to_string(new.join("nomifun-backend.db-wal")).unwrap(), "wal");
        assert_eq!(
            std::fs::read_to_string(new.join("conversations").join("conv_1").join("notes.md")).unwrap(),
            "notes"
        );
        assert_eq!(
            std::fs::read_to_string(new.join("companion").join("shared").join("memory.db")).unwrap(),
            "companion-db"
        );
        assert_eq!(
            std::fs::read_to_string(new.join("knowledge").join("kb_1").join("doc.md")).unwrap(),
            "doc"
        );
        assert_eq!(std::fs::read_to_string(new.join("encryption_key")).unwrap(), "key");

        // Regenerable trees stayed behind.
        assert!(!new.join("runtime").exists());
        assert!(!new.join("logs").exists());
        assert!(!new.join("bun-cache").exists());
        assert!(!new.join("nomifun-backend.db.migrate.lock").exists());

        // Marker points at the legacy root; staging and lock gone.
        let marker: RelocationMarker =
            serde_json::from_str(&std::fs::read_to_string(new.join(RELOCATED_FROM_MARKER)).unwrap()).unwrap();
        assert_eq!(marker.old_root, old.display().to_string());
        assert!(!new.join(STAGING_DIR_NAME).exists());
        assert!(!new.join(LOCK_FILE_NAME).exists());

        // Legacy dir untouched (copy, not move).
        assert_eq!(std::fs::read_to_string(old.join(DB_FILE)).unwrap(), "main-db");
        assert!(old.join("conversations").join("conv_1").join("notes.md").exists());

        // Re-run is a no-op gate hit.
        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::SkippedTargetInUse);
    }

    /// P1: symlinks/junctions inside the legacy tree (workspace skill links,
    /// knowledge mounts) are skipped — the staging copy must neither fail on
    /// them nor materialize them in the new root.
    #[test]
    fn symlinked_entries_are_skipped_not_copied() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);

        let link_target = tmp.path().join("skill-src");
        std::fs::create_dir_all(&link_target).unwrap();
        std::fs::write(link_target.join("SKILL.md"), "skill").unwrap();

        // Nested link, where skill_service actually puts them...
        let skills_dir = old.join("conversations").join("conv_1").join(".claude").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let nested_link = skills_dir.join("my-skill");
        // ...and a top-level one, exercising the stage loop's own skip.
        let top_link = old.join("linked-top");

        if !create_dir_link(&link_target, &nested_link) || !create_dir_link(&link_target, &top_link) {
            // CI sandboxes may forbid link creation; on Windows dev machines
            // the junction path always works (no privilege required).
            eprintln!("skipping symlinked_entries_are_skipped_not_copied: cannot create dir links");
            return;
        }

        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::Relocated);

        // Links did not travel.
        let new_nested = new
            .join("conversations")
            .join("conv_1")
            .join(".claude")
            .join("skills")
            .join("my-skill");
        assert!(std::fs::symlink_metadata(&new_nested).is_err());
        assert!(std::fs::symlink_metadata(new.join("linked-top")).is_err());

        // Sibling real data still arrived, including the link's parent dir.
        assert!(new.join("conversations").join("conv_1").join(".claude").join("skills").is_dir());
        assert_eq!(
            std::fs::read_to_string(new.join("conversations").join("conv_1").join("notes.md")).unwrap(),
            "notes"
        );
        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "main-db");

        // The legacy links themselves are untouched.
        assert!(std::fs::symlink_metadata(&nested_link).unwrap().file_type().is_symlink());
    }

    /// P2: the gate keys on the `.db` only — a database in the new root
    /// means "in use" regardless of markers, and an adopted root keeps its
    /// marker (the backend still needs it for the path rewrite).
    #[test]
    fn gate_skips_when_target_has_db() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);
        std::fs::create_dir_all(&new).unwrap();

        std::fs::write(new.join(DB_FILE), "fresh").unwrap();
        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::SkippedTargetInUse);
        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "fresh");

        // db + marker: an adopted root awaiting (or mid) backend rewrite.
        std::fs::write(new.join(RELOCATED_FROM_MARKER), "{\"old_root\":\"x\"}").unwrap();
        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::SkippedTargetInUse);
        assert_eq!(
            std::fs::read_to_string(new.join(RELOCATED_FROM_MARKER)).unwrap(),
            "{\"old_root\":\"x\"}",
            "marker of an adopted root must survive"
        );
    }

    /// P2: `.relocated-from` WITHOUT the db is the signature of a commit
    /// interrupted between the marker rename and the db rename. The stale
    /// marker must be dropped and the relocation re-run — otherwise the user
    /// boots on a brand-new empty database.
    #[test]
    fn marker_without_db_means_interrupted_commit_and_reruns() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);
        std::fs::create_dir_all(&new).unwrap();

        // Debris of the interrupted commit: marker + some already-renamed dirs.
        std::fs::write(new.join(RELOCATED_FROM_MARKER), "{\"old_root\":\"stale\"}").unwrap();
        std::fs::create_dir_all(new.join("conversations")).unwrap();
        std::fs::write(new.join("conversations").join("stale.txt"), "stale").unwrap();

        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::Relocated);

        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "main-db");
        assert!(!new.join("conversations").join("stale.txt").exists());
        // Fresh marker from the re-run, pointing at the real legacy root.
        let marker: RelocationMarker =
            serde_json::from_str(&std::fs::read_to_string(new.join(RELOCATED_FROM_MARKER)).unwrap()).unwrap();
        assert_eq!(marker.old_root, old.display().to_string());
    }

    /// P2: `.relocated-done` without a db is anomalous (the done marker is
    /// only ever produced on a root that had a db) — handled the same way:
    /// delete and re-run, never treat the empty root as "in use".
    #[test]
    fn done_marker_without_db_reruns_relocation() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join(RELOCATED_DONE_MARKER), "{}").unwrap();

        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::Relocated);

        assert!(!new.join(RELOCATED_DONE_MARKER).exists());
        assert!(new.join(RELOCATED_FROM_MARKER).exists());
        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "main-db");
    }

    #[test]
    fn gate_skips_when_legacy_missing_or_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");

        // Legacy root entirely absent (temp already cleaned).
        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::SkippedNoLegacyData);
        assert!(!new.exists());

        // Legacy root exists but has no database.
        std::fs::create_dir_all(old.join("logs")).unwrap();
        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::SkippedNoLegacyData);
        assert!(!new.exists());
    }

    #[test]
    fn gate_skips_same_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        build_legacy(&root);
        assert_eq!(relocate(&root, &root).unwrap(), RelocateOutcome::SkippedSameDir);
    }

    /// P3: a fresh foreign lock means another live process is relocating —
    /// this one must not touch the new root (and must not steal the lock).
    #[test]
    fn fresh_foreign_lock_skips_relocation() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join(LOCK_FILE_NAME), "").unwrap();

        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::SkippedLockHeld);

        assert!(new.join(LOCK_FILE_NAME).exists(), "foreign lock must not be removed");
        assert!(!new.join(DB_FILE).exists(), "no data may be adopted without the lock");
        assert!(!new.join(STAGING_DIR_NAME).exists());
        assert_eq!(std::fs::read_to_string(old.join(DB_FILE)).unwrap(), "main-db");
    }

    /// P3: lock lifecycle — fresh locks block, stale locks are preempted,
    /// the guard releases on drop.
    #[test]
    fn stale_lock_is_preempted_and_guard_releases() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("new");
        std::fs::create_dir_all(&root).unwrap();
        let lock_path = root.join(LOCK_FILE_NAME);
        std::fs::write(&lock_path, "").unwrap();

        // Fresh (just created) under the production 1h window → unavailable.
        assert!(
            try_acquire_relocation_lock(&root, STALE_LOCK_MAX_AGE).unwrap().is_none(),
            "a fresh lock must not be preempted"
        );
        assert!(lock_path.exists());

        // Zero staleness window: any existing lock counts as dead → preempted.
        let guard = try_acquire_relocation_lock(&root, Duration::ZERO)
            .unwrap()
            .expect("stale lock must be preemptible");
        assert!(lock_path.exists(), "preemption re-creates the lock for the new owner");

        // Release on drop, then a clean re-acquire works.
        drop(guard);
        assert!(!lock_path.exists());
        let reacquired = try_acquire_relocation_lock(&root, STALE_LOCK_MAX_AGE).unwrap();
        assert!(reacquired.is_some());
    }

    /// P3 (Windows): the quiescence probe must report busy while a handle
    /// without FILE_SHARE_DELETE is open — exactly how SQLite holds its db —
    /// and quiescent (with the file restored) once it is dropped.
    #[cfg(windows)]
    #[test]
    fn probe_detects_live_writer_and_restores_the_db() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;

        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(DB_FILE);
        std::fs::write(&db, "main-db").unwrap();

        let held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE) // no FILE_SHARE_DELETE
            .open(&db)
            .unwrap();
        assert_eq!(legacy_db_quiescent(&db).unwrap(), false);
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "main-db", "db must stay in place");

        drop(held);
        assert_eq!(legacy_db_quiescent(&db).unwrap(), true);
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "main-db");
        assert!(!probe_path(&db).exists());
    }

    /// P3 (Windows) end-to-end: a running old instance (live db handle)
    /// defers the relocation; once it exits the next run migrates.
    #[cfg(windows)]
    #[test]
    fn live_writer_on_legacy_db_defers_relocation() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;

        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);

        let held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(old.join(DB_FILE))
            .unwrap();
        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::SkippedLegacyDbBusy);
        assert!(!new.join(DB_FILE).exists());
        assert!(!new.join(LOCK_FILE_NAME).exists(), "lock released after the deferred run");

        drop(held);
        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::Relocated);
        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "main-db");
    }

    /// P3: residue of a crash inside the probe window (db renamed to
    /// `.probe`, process died) is healed and the relocation proceeds.
    #[test]
    fn probe_residue_is_healed_before_migrating() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);
        std::fs::rename(old.join(DB_FILE), old.join("nomifun-backend.db.probe")).unwrap();

        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::Relocated);

        assert_eq!(std::fs::read_to_string(old.join(DB_FILE)).unwrap(), "main-db", "legacy db restored");
        assert!(!old.join("nomifun-backend.db.probe").exists());
        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "main-db");
        assert!(!new.join("nomifun-backend.db.probe").exists(), "probe name must never be staged");
    }

    /// P3: a crash mid-probe leaves BOTH the `.probe` rename and a fresh
    /// lock file behind. The residue proves the lock's holder is dead, so it
    /// is preempted immediately — deferring to the legacy root would boot a
    /// data dir whose db is still misnamed and seed a fresh empty database.
    #[test]
    fn probe_residue_implies_dead_lock_holder_and_is_preempted() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);
        std::fs::rename(old.join(DB_FILE), old.join("nomifun-backend.db.probe")).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join(LOCK_FILE_NAME), "").unwrap();

        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::Relocated);

        assert_eq!(std::fs::read_to_string(old.join(DB_FILE)).unwrap(), "main-db", "legacy db healed");
        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "main-db");
        assert!(!new.join(LOCK_FILE_NAME).exists(), "preempted lock released after the run");
    }

    #[test]
    fn staging_residue_and_partial_targets_are_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        build_legacy(&old);

        // Debris of a crashed earlier attempt: staging junk plus an already
        // renamed (stale) conversations dir — but no `.db`, so the gate
        // re-runs and the fresh copies must replace the scraps.
        std::fs::create_dir_all(new.join(STAGING_DIR_NAME).join("conversations")).unwrap();
        std::fs::write(new.join(STAGING_DIR_NAME).join("junk.txt"), "junk").unwrap();
        std::fs::create_dir_all(new.join("conversations")).unwrap();
        std::fs::write(new.join("conversations").join("stale.txt"), "stale").unwrap();

        assert_eq!(relocate(&old, &new).unwrap(), RelocateOutcome::Relocated);

        assert!(!new.join(STAGING_DIR_NAME).exists());
        assert!(!new.join("conversations").join("stale.txt").exists());
        assert_eq!(
            std::fs::read_to_string(new.join("conversations").join("conv_1").join("notes.md")).unwrap(),
            "notes"
        );
        assert_eq!(std::fs::read_to_string(new.join(DB_FILE)).unwrap(), "main-db");
    }

    #[test]
    fn failure_leaves_legacy_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        build_legacy(&old);
        // Make the new root unusable: a FILE occupies its path, so
        // create_dir_all on the new root must fail.
        let new = tmp.path().join("new");
        std::fs::write(&new, "blocker").unwrap();

        assert!(relocate(&old, &new).is_err());

        // Legacy data is fully intact for the fallback launch.
        assert_eq!(std::fs::read_to_string(old.join(DB_FILE)).unwrap(), "main-db");
        assert!(old.join("conversations").join("conv_1").join("notes.md").exists());
    }

    /// P2: the failure sweep removes staging scraps AND an unadopted marker,
    /// but never the marker of a root that holds a `.db`.
    #[test]
    fn failed_attempt_cleanup_clears_marker_only_without_db() {
        let tmp = tempfile::tempdir().unwrap();
        let new = tmp.path().join("new");
        std::fs::create_dir_all(new.join(STAGING_DIR_NAME)).unwrap();
        std::fs::write(new.join(STAGING_DIR_NAME).join("junk.txt"), "junk").unwrap();
        std::fs::write(new.join(RELOCATED_FROM_MARKER), "{}").unwrap();

        cleanup_failed_relocation(&new);
        assert!(!new.join(STAGING_DIR_NAME).exists());
        assert!(!new.join(RELOCATED_FROM_MARKER).exists());

        // Adopted root (db in place): the marker must survive the sweep.
        std::fs::write(new.join(DB_FILE), "main-db").unwrap();
        std::fs::write(new.join(RELOCATED_FROM_MARKER), "{}").unwrap();
        cleanup_failed_relocation(&new);
        assert!(new.join(RELOCATED_FROM_MARKER).exists());
    }

    #[test]
    fn env_override_skips_relocation_entirely() {
        // Only this test touches the env var, and only effective_data_dir
        // reads it — safe under the parallel test runner.
        // SAFETY: no other thread in this test binary mutates the env.
        unsafe { std::env::set_var("NOMIFUN_DATA_DIR", r"X:\custom-root") };
        let dir = PathBuf::from(r"X:\custom-root\Nomi");
        let resolved = effective_data_dir(dir.clone());
        unsafe { std::env::remove_var("NOMIFUN_DATA_DIR") };
        assert_eq!(resolved, dir);
    }
}
