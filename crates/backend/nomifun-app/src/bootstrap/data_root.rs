//! Startup data-root resolution: map the channel default to the current
//! on-disk layout and migrate legacy layouts forward, losslessly.
//!
//! ## Layout history
//!
//! * pre-0.3.4: `<app-data>/NomiFun/Nomi<channel-suffix>` (the `Nomi` leaf is
//!   a historic product name), temp fallback `<temp>/nomifun-data/Nomi<suffix>`.
//! * current: `<app-data>/NomiFun<channel-suffix>` — the stable data root is
//!   the vendor directory itself, and non-stable channels are *siblings*
//!   (`NomiFun-dev`), never subdirectories of the stable root.
//!
//! ## Inherited self-exports
//!
//! Every backend boot re-exports the effective `NOMIFUN_DATA_DIR` /
//! `NOMIFUN_WORK_DIR` for in-process and child consumers
//! (`environment.rs`). An in-app restart or auto-update relaunch inherits
//! those exports, and the 0.3.2→0.3.3 Windows updater incident showed what
//! happens when a host reinterprets its own export (the desktop shell used
//! to append `/Nomi`, so the inherited value re-resolved to
//! `…/NomiFun/Nomi/Nomi` — a fresh empty root — while the inherited work dir
//! still named the real dataset, tripping the "work directory is already a
//! NomiFun data root" guard). Env semantics are now literal on every host,
//! which makes self-inheritance idempotent; additionally,
//! [`resolve_startup_data_root`] maps any value that IS a known default
//! location (current, legacy, or the historical double-append junk) back to
//! the channel default so the layout migration still runs on machines whose
//! environment was poisoned by an affected release.
//!
//! ## Migration
//!
//! One-shot, crash-resumable, same-volume `rename`-based move of the legacy
//! dataset into the current root:
//!
//! 1. Exclusive `server.lock` on BOTH roots (waits out the updater-restart
//!    handoff; a live old process defers migration to the next boot).
//! 2. Durable resume marker in the legacy root — the migration gate.
//! 3. Quarantine of the known `Nomi` double-append junk dataset, then a
//!    conflict pre-scan (abort cleanly before anything moved).
//! 4. Entry moves: ordinary entries, then DB sidecars, then the bare
//!    `nomifun-backend.db` last — the same "sidecars before main database"
//!    ordering the reset engine uses, so no crash can strand a database away
//!    from its WAL.
//! 5. Durable-marker rebinding
//!    ([`nomifun_common::factory_reset::rebind_data_root_after_relocation`]) and
//!    a `.relocated-from` marker that triggers the one-shot in-database
//!    absolute-path rewrite after SQLite opens (see `bootstrap::relocation`).
//! 6. Marker removal and legacy-root cleanup.
//!
//! Any deferral or failure keeps the boot on whichever root still holds the
//! database, so the user keeps a working app and the migration retries on
//! the next launch.

use std::path::{Path, PathBuf};

use super::boot_log::{BootNoteLevel, record_boot_note};
use super::server_lock;
use nomifun_common::paths;

/// Resume marker written into the LEGACY root before the first entry moves.
/// Its presence means "a layout migration into the current default root is
/// in progress"; boots must resume it before using either root.
pub const LAYOUT_MIGRATION_PENDING_MARKER: &str =
    ".nomifun-layout-migration.pending";

/// Written into the NEW root once the files have moved; consumed by the
/// post-open database path rewrite (`bootstrap::relocation`).
pub const RELOCATED_FROM_MARKER: &str = ".relocated-from";
/// The consumed form of [`RELOCATED_FROM_MARKER`].
pub const RELOCATED_DONE_MARKER: &str = ".relocated-done";

/// JSON content of [`RELOCATED_FROM_MARKER`] (schema shared with the
/// pre-v3 relocation era so `dataset_roots.rs` classification still holds).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RelocationMarker {
    /// Absolute path of the data root the files were relocated from.
    pub old_root: String,
    /// ms-epoch timestamp of the file relocation (informational).
    #[serde(default)]
    pub relocated_at_ms: i64,
}

/// JSON content of [`LAYOUT_MIGRATION_PENDING_MARKER`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct LayoutMigrationMarker {
    version: u32,
    new_root: String,
    old_root: String,
    started_at_ms: i64,
}

const LAYOUT_MIGRATION_MARKER_VERSION: u32 = 1;

/// Same ordering contract as the reset engine: sidecars retire before the
/// main database file, so a crash can never leave an adopted database next
/// to a stale, detached WAL.
const DB_SIDECARS: &[&str] = &[
    "nomifun-backend.db-wal",
    "nomifun-backend.db-shm",
    "nomifun-backend.db-journal",
    "nomifun-backend.db.migrate.lock",
];
const DB_FILE: &str = "nomifun-backend.db";

/// Dataset-identity artifacts that mean "this directory holds (or held) a
/// real dataset worth migrating". Lock files are deliberately excluded: they
/// are ambient and are recreated by any boot (including a failed migration
/// attempt on the destination side).
const DATASET_ARTIFACTS: &[&str] = &[
    DB_FILE,
    "storage-generation",
    "dataset-v3.json",
    ".dataset-v3.bootstrap.json",
];

/// Entries that never move: process-lifetime lock addresses and this
/// migration's own control files. They are recreated where needed and
/// deleted from the legacy root during cleanup.
const MIGRATION_SKIP_ENTRIES: &[&str] = &[
    "server.lock",
    "server.lock.info",
    ".nomifun-work-root.lock",
    ".relocating.lock",
    LAYOUT_MIGRATION_PENDING_MARKER,
];

/// Resolve the data root a host boot should actually use.
///
/// * An explicit, user-chosen path is returned verbatim — no migration.
/// * A path that IS a known self-export/default location (current default,
///   legacy default, or the historical desktop double-append junk under it)
///   is mapped to the channel default first.
/// * For the channel default, the one-shot legacy layout migration runs (or
///   resumes); on deferral the boot continues on the legacy root and the
///   migration retries next launch.
pub fn resolve_startup_data_root(requested: PathBuf) -> PathBuf {
    let default = crate::cli::default_data_dir();
    let requested = if is_known_default_location(&requested) {
        default.clone()
    } else {
        requested
    };
    if requested != default {
        return requested;
    }
    let legacy = crate::cli::legacy_default_data_dir();

    match migrate_legacy_layout(&default, &legacy) {
        Ok(MigrationOutcome::UseNew { migrated }) => {
            if migrated {
                record_boot_note(
                    BootNoteLevel::Info,
                    format!(
                        "migrated the data root from {} to {}",
                        legacy.display(),
                        default.display()
                    ),
                );
            }
            default
        }
        Ok(MigrationOutcome::UseLegacy(reason)) => {
            record_boot_note(
                BootNoteLevel::Warn,
                format!(
                    "data-root layout migration deferred ({reason}); continuing on {}",
                    legacy.display()
                ),
            );
            legacy
        }
        Err(error) => {
            // Fail open onto whichever root still holds the database: the
            // lifecycle gates downstream fail closed, so the worst case is an
            // actionable boot error — never silent data loss.
            let fallback = if legacy.join(DB_FILE).exists()
                && !default.join(DB_FILE).exists()
            {
                legacy.clone()
            } else {
                default.clone()
            };
            record_boot_note(
                BootNoteLevel::Warn,
                format!(
                    "data-root layout migration failed ({error:#}); continuing on {}",
                    fallback.display()
                ),
            );
            fallback
        }
    }
}

/// Whether `path` names a location this application itself would have
/// exported through `NOMIFUN_DATA_DIR` / `NOMIFUN_WORK_DIR` in some release:
/// the current channel default, the legacy channel default, or the
/// double-append junk roots the 0.3.2/0.3.3 desktop shell produced under it.
/// Windows verbatim (`\\?\`) spellings match their plain forms.
pub fn is_known_default_location(path: &Path) -> bool {
    known_default_locations()
        .iter()
        .any(|candidate| paths::paths_equivalent(path, candidate))
}

fn known_default_locations() -> Vec<PathBuf> {
    let legacy = crate::cli::legacy_default_data_dir();
    // The affected desktop releases appended a fixed `Nomi` leaf to their own
    // exported effective root on every restart, so the junk compounds.
    let junk_once = legacy.join("Nomi");
    let junk_twice = junk_once.join("Nomi");
    vec![
        crate::cli::default_data_dir(),
        legacy,
        junk_once,
        junk_twice,
    ]
}

#[derive(Debug)]
enum MigrationOutcome {
    UseNew { migrated: bool },
    UseLegacy(String),
}

fn root_has_any_artifact(root: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| root.join(name).exists())
}

fn migrate_legacy_layout(
    new_root: &Path,
    legacy_root: &Path,
) -> anyhow::Result<MigrationOutcome> {
    use anyhow::Context;

    let legacy_metadata = match std::fs::symlink_metadata(legacy_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MigrationOutcome::UseNew { migrated: false });
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("inspect legacy data root {}", legacy_root.display())
            });
        }
    };
    if legacy_metadata.file_type().is_symlink() || !legacy_metadata.is_dir() {
        // A link or stray file in the legacy location is not a dataset.
        return Ok(MigrationOutcome::UseNew { migrated: false });
    }

    let pending_marker = legacy_root.join(LAYOUT_MIGRATION_PENDING_MARKER);
    let resuming = pending_marker.exists();
    if !resuming {
        if !root_has_any_artifact(legacy_root, DATASET_ARTIFACTS) {
            // Nothing worth migrating (empty dir, stray logs, …). Boot fresh
            // on the new root; residue cleanup is a later concern.
            return Ok(MigrationOutcome::UseNew { migrated: false });
        }
        if new_root.join(DB_FILE).exists() {
            record_boot_note(
                BootNoteLevel::Warn,
                format!(
                    "both {} and the legacy {} contain a database; keeping the current root and leaving the legacy dataset untouched",
                    new_root.display(),
                    legacy_root.display()
                ),
            );
            return Ok(MigrationOutcome::UseNew { migrated: false });
        }
        if root_has_any_artifact(new_root, DATASET_ARTIFACTS) {
            // Dataset-ish artifacts without a database in the destination:
            // not something this migration produced (its own gate is the
            // pending marker). Refuse to merge into the unknown.
            return Ok(MigrationOutcome::UseLegacy(format!(
                "current default root {} already contains unrelated dataset artifacts",
                new_root.display()
            )));
        }
    }

    // Exclusive authority over both roots. The legacy acquisition waits out
    // the same restart-handoff window a normal boot does, so an auto-update
    // relaunch migrates as soon as the old process exits; a genuinely live
    // old instance defers the migration instead of racing it.
    std::fs::create_dir_all(new_root).with_context(|| {
        format!("create data root {}", new_root.display())
    })?;
    let legacy_lock = match server_lock::acquire_server_lock(legacy_root) {
        Ok(lock) => lock,
        Err(error) => {
            // A concurrently RUNNING backend owns the legacy root: boot there
            // and let the normal server-lock contention UX apply. In the
            // pathological case where the holder was another host's
            // in-flight migration (two different hosts launched in the same
            // instant), the worst outcome is an empty dataset bootstrapped
            // at the drained legacy root; the migrated data stays intact in
            // the new root, and the next launch keeps the new root (the
            // "both roots contain a database" guard warns instead of
            // merging). Data is never lost either way.
            return Ok(MigrationOutcome::UseLegacy(format!(
                "legacy data root is still in use: {error:#}"
            )));
        }
    };
    let new_lock = server_lock::acquire_server_lock(new_root)
        .context("lock the current default data root for migration")?;

    if resuming
        && new_root.join(DB_FILE).exists()
        && legacy_root.join(DB_FILE).exists()
    {
        // The move transfers the database (never copies it), so two live
        // database files across an interrupted migration can only mean
        // external interference. Never merge into the unknown.
        return Ok(MigrationOutcome::UseLegacy(
            "an interrupted migration found databases in BOTH roots; refusing to merge automatically"
                .to_owned(),
        ));
    }

    // The 0.3.2/0.3.3 desktop double-append bug created a junk dataset
    // literally named `Nomi` INSIDE the legacy root (`…/NomiFun/Nomi/Nomi`).
    // For the stable channel its move target would be the legacy root
    // itself, so it must be quarantined first. A `Nomi` entry that contains
    // a real database is ambiguous and aborts the migration instead — before
    // the resume marker is written, so the abort leaves no residue.
    let junk = legacy_root.join("Nomi");
    if junk.is_dir()
        && !paths::paths_equivalent(&junk, new_root)
    {
        if junk.join(DB_FILE).exists() {
            return Ok(MigrationOutcome::UseLegacy(format!(
                "unexpected nested dataset with a database at {}; refusing to migrate automatically",
                junk.display()
            )));
        }
        let quarantine_parent =
            legacy_root.join(nomifun_common::factory_reset::RETIRED_DATASETS_DIR);
        std::fs::create_dir_all(&quarantine_parent).with_context(|| {
            format!("create {}", quarantine_parent.display())
        })?;
        let quarantine = quarantine_parent.join(format!(
            "double-append-junk-{}",
            nomifun_common::now_ms()
        ));
        std::fs::rename(&junk, &quarantine).with_context(|| {
            format!(
                "quarantine double-append junk {} -> {}",
                junk.display(),
                quarantine.display()
            )
        })?;
        record_boot_note(
            BootNoteLevel::Warn,
            format!(
                "quarantined a double-append junk dataset ({} -> {})",
                junk.display(),
                quarantine.display()
            ),
        );
    }

    // Plan the moves: ordinary entries first, database family last (sidecars
    // before the bare .db). The conflict pre-scan runs BEFORE the resume
    // marker is written: on the FIRST run any conflict aborts cleanly — no
    // marker, nothing moved, the next boot re-evaluates from scratch. On a
    // RESUME the migration must keep making progress toward the database
    // gate, so a conflicted entry (something recreated it while the
    // migration was interrupted) is left behind in the legacy root with a
    // boot note.
    let mut ordinary: Vec<std::ffi::OsString> = Vec::new();
    for entry in std::fs::read_dir(legacy_root).with_context(|| {
        format!("enumerate legacy data root {}", legacy_root.display())
    })? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if MIGRATION_SKIP_ENTRIES
            .iter()
            .any(|skip| name_str == *skip)
            || name_str == DB_FILE
            || DB_SIDECARS.iter().any(|sidecar| name_str == *sidecar)
        {
            continue;
        }
        ordinary.push(name);
    }
    let mut conflicted: Vec<std::ffi::OsString> = Vec::new();
    for name in &ordinary {
        let destination = new_root.join(name);
        if destination.exists() {
            if !resuming {
                return Ok(MigrationOutcome::UseLegacy(format!(
                    "destination already contains {}; refusing to merge",
                    destination.display()
                )));
            }
            record_boot_note(
                BootNoteLevel::Warn,
                format!(
                    "layout migration left {} behind: the destination already contains an entry with that name",
                    legacy_root.join(name).display()
                ),
            );
            conflicted.push(name.clone());
        }
    }
    ordinary.retain(|name| !conflicted.contains(name));

    if !resuming {
        let marker = LayoutMigrationMarker {
            version: LAYOUT_MIGRATION_MARKER_VERSION,
            new_root: paths::marker_string(new_root),
            old_root: paths::marker_string(legacy_root),
            started_at_ms: nomifun_common::now_ms(),
        };
        let bytes = serde_json::to_vec_pretty(&marker)
            .context("serialize layout-migration marker")?;
        std::fs::write(&pending_marker, bytes).with_context(|| {
            format!(
                "write layout-migration marker {}",
                pending_marker.display()
            )
        })?;
    }

    let move_entry = |name: &std::ffi::OsStr| -> anyhow::Result<()> {
        let source = legacy_root.join(name);
        if !source.exists() {
            // Already moved by a previous (crashed) attempt.
            return Ok(());
        }
        let destination = new_root.join(name);
        std::fs::rename(&source, &destination).with_context(|| {
            format!(
                "move {} -> {}",
                source.display(),
                destination.display()
            )
        })
    };
    for name in &ordinary {
        move_entry(name)?;
    }
    for sidecar in DB_SIDECARS {
        move_entry(std::ffi::OsStr::new(sidecar))?;
    }
    move_entry(std::ffi::OsStr::new(DB_FILE))?;

    // Rebind the durable lifecycle markers to the new root, then arm the
    // one-shot in-database path rewrite for the first post-open boot.
    let migrated_database = new_root.join(DB_FILE).exists();
    if root_has_any_artifact(new_root, DATASET_ARTIFACTS) {
        let warnings =
            nomifun_common::factory_reset::rebind_data_root_after_relocation(
                new_root,
                legacy_root,
            )
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        for warning in warnings {
            record_boot_note(BootNoteLevel::Warn, warning);
        }
    }
    if migrated_database {
        let marker_path = new_root.join(RELOCATED_FROM_MARKER);
        if marker_path.exists() {
            record_boot_note(
                BootNoteLevel::Warn,
                "overwriting a stale .relocated-from marker with the current relocation".to_owned(),
            );
        }
        let marker = RelocationMarker {
            old_root: paths::marker_string(legacy_root),
            relocated_at_ms: nomifun_common::now_ms(),
        };
        let bytes = serde_json::to_vec_pretty(&marker)
            .context("serialize .relocated-from marker")?;
        std::fs::write(&marker_path, bytes).with_context(|| {
            format!("write {}", marker_path.display())
        })?;
    }

    // Close the gate, release the lock handles, then sweep the residue.
    std::fs::remove_file(&pending_marker).with_context(|| {
        format!(
            "remove layout-migration marker {}",
            pending_marker.display()
        )
    })?;
    drop(legacy_lock);
    drop(new_lock);
    cleanup_legacy_residue(legacy_root);
    Ok(MigrationOutcome::UseNew { migrated: true })
}

/// Best-effort removal of the emptied legacy root (lock files and the
/// directory itself). Failures are irrelevant: the directory no longer holds
/// a dataset and nothing resolves it anymore.
fn cleanup_legacy_residue(legacy_root: &Path) {
    for name in MIGRATION_SKIP_ENTRIES {
        let _ = std::fs::remove_file(legacy_root.join(name));
    }
    let _ = std::fs::remove_dir(legacy_root);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nomifun-dataroot-{tag}-{}",
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_dataset(root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join(DB_FILE), b"db-bytes").unwrap();
        std::fs::write(root.join("nomifun-backend.db-wal"), b"wal").unwrap();
        std::fs::write(root.join("storage-generation"), b"gen").unwrap();
        std::fs::write(root.join("server.lock"), b"").unwrap();
        std::fs::create_dir_all(root.join("conversations/ws-1")).unwrap();
        std::fs::write(root.join("conversations/ws-1/file.txt"), b"hello").unwrap();
        std::fs::create_dir_all(root.join("logs")).unwrap();
        std::fs::write(root.join("logs/app.log"), b"log").unwrap();
    }

    #[test]
    fn missing_legacy_root_boots_the_new_root_untouched() {
        let parent = temp_root("fresh");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();
        assert!(matches!(
            outcome,
            MigrationOutcome::UseNew { migrated: false }
        ));
        assert!(!new_root.exists(), "no dirs are created for a fresh install");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn legacy_dataset_moves_up_into_the_new_root() {
        let parent = temp_root("uplevel");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        seed_dataset(&legacy);

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();

        assert!(matches!(outcome, MigrationOutcome::UseNew { migrated: true }));
        assert_eq!(std::fs::read(new_root.join(DB_FILE)).unwrap(), b"db-bytes");
        assert_eq!(
            std::fs::read(new_root.join("conversations/ws-1/file.txt")).unwrap(),
            b"hello"
        );
        assert!(new_root.join("logs/app.log").is_file());
        assert!(
            new_root.join(RELOCATED_FROM_MARKER).is_file(),
            "database moves must arm the in-database path rewrite"
        );
        assert!(
            !legacy.exists(),
            "the drained legacy root is removed, got residue: {:?}",
            std::fs::read_dir(&legacy)
                .map(|entries| entries
                    .filter_map(|e| e.ok().map(|e| e.file_name()))
                    .collect::<Vec<_>>())
        );
        let marker: RelocationMarker = serde_json::from_slice(
            &std::fs::read(new_root.join(RELOCATED_FROM_MARKER)).unwrap(),
        )
        .unwrap();
        assert!(nomifun_common::paths::stored_path_matches(
            &marker.old_root,
            &nomifun_common::paths::simplified(&legacy),
        ));
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn sibling_channel_dataset_moves_out_of_the_vendor_dir() {
        // dev-channel shape: NomiFun/Nomi-dev -> NomiFun-dev (sibling).
        let parent = temp_root("sibling");
        let new_root = parent.join("NomiFun-dev");
        let legacy = parent.join("NomiFun").join("Nomi-dev");
        seed_dataset(&legacy);

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();

        assert!(matches!(outcome, MigrationOutcome::UseNew { migrated: true }));
        assert_eq!(std::fs::read(new_root.join(DB_FILE)).unwrap(), b"db-bytes");
        assert!(!legacy.exists());
        assert!(
            parent.join("NomiFun").is_dir(),
            "the stable vendor dir itself is left alone"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn double_append_junk_without_database_is_quarantined() {
        let parent = temp_root("junk");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        seed_dataset(&legacy);
        // The 0.3.3 crash residue: NomiFun/Nomi/Nomi with locks + generation
        // but no database.
        let junk = legacy.join("Nomi");
        std::fs::create_dir_all(junk.join("logs")).unwrap();
        std::fs::write(junk.join("server.lock"), b"").unwrap();
        std::fs::write(junk.join("storage-generation"), b"junk-gen").unwrap();

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();

        assert!(matches!(outcome, MigrationOutcome::UseNew { migrated: true }));
        assert_eq!(std::fs::read(new_root.join(DB_FILE)).unwrap(), b"db-bytes");
        let retired = new_root
            .join(nomifun_common::factory_reset::RETIRED_DATASETS_DIR);
        let quarantined = std::fs::read_dir(&retired)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("double-append-junk-")
            })
            .expect("junk dataset must be quarantined under retired-datasets");
        assert!(
            quarantined.path().join("storage-generation").is_file(),
            "junk bytes are preserved, not deleted"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn nested_junk_with_a_database_defers_to_manual_handling() {
        let parent = temp_root("junkdb");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        seed_dataset(&legacy);
        let junk = legacy.join("Nomi");
        std::fs::create_dir_all(&junk).unwrap();
        std::fs::write(junk.join(DB_FILE), b"ambiguous").unwrap();

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();

        assert!(matches!(outcome, MigrationOutcome::UseLegacy(_)));
        assert!(
            legacy.join(DB_FILE).is_file(),
            "nothing is moved when the layout is ambiguous"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn interrupted_migration_resumes_and_completes() {
        let parent = temp_root("resume");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        seed_dataset(&legacy);

        // Simulate a crash mid-phase-1: marker written, one ordinary entry
        // moved, database family still in the legacy root.
        std::fs::create_dir_all(&new_root).unwrap();
        let marker = LayoutMigrationMarker {
            version: LAYOUT_MIGRATION_MARKER_VERSION,
            new_root: paths::marker_string(&new_root),
            old_root: paths::marker_string(&legacy),
            started_at_ms: nomifun_common::now_ms(),
        };
        std::fs::write(
            legacy.join(LAYOUT_MIGRATION_PENDING_MARKER),
            serde_json::to_vec_pretty(&marker).unwrap(),
        )
        .unwrap();
        std::fs::rename(legacy.join("logs"), new_root.join("logs")).unwrap();

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();

        assert!(matches!(outcome, MigrationOutcome::UseNew { migrated: true }));
        assert_eq!(std::fs::read(new_root.join(DB_FILE)).unwrap(), b"db-bytes");
        assert!(new_root.join("logs/app.log").is_file());
        assert!(!legacy.exists());
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn existing_database_in_the_new_root_wins_without_touching_legacy() {
        let parent = temp_root("bothdb");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        seed_dataset(&legacy);
        std::fs::write(new_root.join(DB_FILE), b"current").unwrap();

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();

        assert!(matches!(
            outcome,
            MigrationOutcome::UseNew { migrated: false }
        ));
        assert_eq!(std::fs::read(new_root.join(DB_FILE)).unwrap(), b"current");
        assert_eq!(std::fs::read(legacy.join(DB_FILE)).unwrap(), b"db-bytes");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn destination_conflict_aborts_before_moving_anything() {
        let parent = temp_root("conflict");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        seed_dataset(&legacy);
        // An unrelated `conversations` dir already sits in the destination.
        std::fs::create_dir_all(new_root.join("conversations")).unwrap();

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();

        assert!(matches!(outcome, MigrationOutcome::UseLegacy(_)));
        assert!(
            legacy.join(DB_FILE).is_file()
                && legacy.join("conversations/ws-1/file.txt").is_file(),
            "a conflict must abort before the dataset moves"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn live_legacy_instance_defers_the_migration() {
        let parent = temp_root("live");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        seed_dataset(&legacy);
        let held = server_lock::acquire_server_lock(&legacy).unwrap();

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();

        assert!(matches!(outcome, MigrationOutcome::UseLegacy(_)));
        assert!(legacy.join(DB_FILE).is_file());
        drop(held);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn known_default_locations_match_verbatim_spellings() {
        let legacy = crate::cli::legacy_default_data_dir();
        assert!(is_known_default_location(&legacy));
        assert!(is_known_default_location(&legacy.join("Nomi")));
        assert!(is_known_default_location(&legacy.join("Nomi").join("Nomi")));
        assert!(is_known_default_location(&crate::cli::default_data_dir()));
        assert!(!is_known_default_location(Path::new("/somewhere/else")));
        #[cfg(windows)]
        {
            let verbatim = PathBuf::from(format!(
                r"\\?\{}",
                legacy.display()
            ));
            assert!(is_known_default_location(&verbatim));
        }
    }

    /// End-to-end: a REAL finalized v3 dataset (receipt + owner + binding,
    /// written by the same factory_reset code the app uses — including the
    /// pre-fix Windows verbatim spellings) must migrate and then pass the
    /// fail-closed dataset identity gates against the NEW root.
    #[tokio::test]
    async fn migrated_finalized_dataset_passes_the_lifecycle_gates() {
        use nomifun_common::factory_reset;

        let parent = temp_root("lifecycle");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        std::fs::create_dir_all(&legacy).unwrap();

        let database =
            nomifun_db::init_database(&legacy.join(DB_FILE)).await.unwrap();
        database.close().await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(legacy.join("storage-generation"), &generation).unwrap();
        factory_reset::write_v3_dataset_receipt(&legacy, &generation).unwrap();
        factory_reset::ensure_current_v3_work_root_owner(&legacy, &legacy)
            .unwrap();

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();
        assert!(matches!(outcome, MigrationOutcome::UseNew { migrated: true }));

        assert_eq!(
            factory_reset::inspect_v3_dataset_receipt(&new_root, &new_root)
                .unwrap(),
            factory_reset::DatasetReceiptStatus::Current,
            "the rebound receipt must bind the dataset to the new root"
        );
        factory_reset::require_current_v3_dataset_for_work_dir(
            &new_root, &new_root,
        )
        .expect("the migrated dataset must pass the fail-closed identity gate");
        factory_reset::ensure_current_v3_work_root_owner(&new_root, &new_root)
            .expect("the rebound owner marker must accept the new data root");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// A dataset bound to an EXTERNAL work root keeps that binding across the
    /// migration, and the external owner marker is re-pointed at the new
    /// data root.
    #[tokio::test]
    async fn migration_rebinds_an_external_work_root_owner() {
        use nomifun_common::factory_reset;

        let parent = temp_root("external");
        let new_root = parent.join("NomiFun");
        let legacy = new_root.join("Nomi");
        let external_work = parent.join("external-work");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&external_work).unwrap();

        let database =
            nomifun_db::init_database(&legacy.join(DB_FILE)).await.unwrap();
        database.close().await;
        let generation = uuid::Uuid::now_v7().to_string();
        std::fs::write(legacy.join("storage-generation"), &generation).unwrap();
        factory_reset::write_v3_dataset_receipt_for_work_dir(
            &legacy,
            &external_work,
            &generation,
        )
        .unwrap();
        factory_reset::ensure_current_v3_work_root_owner(
            &legacy,
            &external_work,
        )
        .unwrap();
        nomifun_common::dir_config::set_work_dir(&legacy, &external_work)
            .unwrap();

        let outcome = migrate_legacy_layout(&new_root, &legacy).unwrap();
        assert!(matches!(outcome, MigrationOutcome::UseNew { migrated: true }));

        assert_eq!(
            factory_reset::inspect_v3_dataset_receipt(&new_root, &external_work)
                .unwrap(),
            factory_reset::DatasetReceiptStatus::Current,
            "the receipt must stay bound to the external work root"
        );
        factory_reset::ensure_current_v3_work_root_owner(
            &new_root,
            &external_work,
        )
        .expect("the external owner marker must accept the new data root");
        let persisted =
            nomifun_common::dir_config::checked_persisted_work_dir(&new_root)
                .unwrap()
                .expect("dir-config must survive the migration");
        assert!(nomifun_common::paths::paths_equivalent(
            &persisted,
            &external_work
        ));
        let _ = std::fs::remove_dir_all(&parent);
    }
}
