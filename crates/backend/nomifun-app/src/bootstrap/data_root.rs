//! Shared startup data-root resolution and Fresh-v4 cutover entry point.
//!
//! Known historical self-export paths are normalized to the current
//! channel-specific canonical root. The Fresh-v4 coordinator then either
//! creates a new root or atomically renames the entire existing canonical
//! root to its opaque timestamped sibling before creating v4 at the original
//! path. No legacy entry is enumerated, copied, imported, or restored.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_common::paths;
use nomifun_v4_root::FreshV4OperationKind;

use super::boot_log::{BootNoteLevel, record_boot_note};

/// Retained only for the published pre-v4 relocation reader, which remains
/// compiled until its owning legacy slice is removed.
pub const LAYOUT_MIGRATION_PENDING_MARKER: &str =
    ".nomifun-layout-migration.pending";
/// Retained for the published pre-v4 relocation reader.
pub const RELOCATED_FROM_MARKER: &str = ".relocated-from";
/// Retained for the published pre-v4 relocation reader.
pub const RELOCATED_DONE_MARKER: &str = ".relocated-done";

/// Published pre-v4 relocation payload retained byte/schema-compatible for
/// the legacy reader. Fresh-v4 never creates or consumes this marker.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RelocationMarker {
    pub old_root: String,
    #[serde(default)]
    pub relocated_at_ms: i64,
}

/// Resolve and complete the pre-service Fresh-v4 root operation.
///
/// This retains the existing `PathBuf` return surface for binary callers.
/// Fresh-v4 failures are fail-stop because continuing would allow a legacy
/// data-layer path to open or mutate the canonical root.
pub fn resolve_startup_data_root(requested: PathBuf) -> PathBuf {
    let canonical = normalize_requested_startup_data_root(requested);
    let allow_dev_recovery = crate::channel::channel() != "stable"
        && is_known_default_location(&canonical);
    let outcome = match super::v4_root::bootstrap_data_root(&canonical) {
        Ok(outcome) => outcome,
        Err(error) if allow_dev_recovery && is_expected_dev_contract_drift(&error) => {
            let archive = archive_stale_dev_root(&canonical)
                .unwrap_or_else(|archive_error| {
                    panic!(
                        "{error:#}; stale development root could not be archived: \
                         {archive_error:#}"
                    )
                });
            record_boot_note(
                BootNoteLevel::Warn,
                format!(
                    "archived stale development Fresh-v4 root at {}",
                    archive.display()
                ),
            );
            super::v4_root::bootstrap_data_root(&canonical).unwrap_or_else(|retry_error| {
                panic!(
                    "Fresh-v4 development root rebuild failed after archiving {}: \
                     {retry_error:#}",
                    archive.display()
                )
            })
        }
        Err(error) => panic!("{error:#}"),
    };

    match outcome.operation_kind {
        Some(FreshV4OperationKind::Fresh) => record_boot_note(
            BootNoteLevel::Info,
            format!(
                "initialized a fresh v4 data root at {}",
                canonical.display()
            ),
        ),
        Some(FreshV4OperationKind::Cutover) => record_boot_note(
            BootNoteLevel::Info,
            format!(
                "completed the opaque whole-root v4 cutover at {}",
                canonical.display()
            ),
        ),
        None => {}
    }
    canonical
}

fn is_expected_dev_contract_drift(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}");
    message.contains("ready root schema_metadata does not match the embedded Fresh-v4 contract")
        || message.contains("Fresh-v4 ready marker application build digest does not match this build")
}

/// Preserve, then replace, only the active default development root whose
/// Fresh-v4 contract predates the current pre-Stable source. This deliberately
/// does not inspect, import, or delete descendants; the whole directory is
/// moved as one sibling so the old dataset remains available for diagnosis.
fn archive_stale_dev_root(data_root: &Path) -> anyhow::Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(data_root)
        .map_err(|error| anyhow::anyhow!("inspect stale development root: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "stale development root is not a real directory: {}",
            data_root.display()
        );
    }
    let parent = data_root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("stale development root has no parent"))?;
    let basename = data_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && *value != "." && *value != "..")
        .ok_or_else(|| anyhow::anyhow!("stale development root has an invalid basename"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut archive = parent.join(format!("{basename}.stale-v4-archive-{timestamp}"));
    if archive.exists() {
        archive = parent.join(format!(
            "{basename}.stale-v4-archive-{timestamp}-{}",
            uuid::Uuid::now_v7()
        ));
    }
    if archive.parent() != Some(parent) || archive == *data_root {
        anyhow::bail!("stale development archive target escaped the data root parent");
    }
    std::fs::rename(data_root, &archive).map_err(|error| {
        anyhow::anyhow!(
            "atomically archive stale development root {} -> {}: {error}",
            data_root.display(),
            archive.display()
        )
    })?;
    Ok(archive)
}

pub(super) fn normalize_requested_startup_data_root(
    requested: PathBuf,
) -> PathBuf {
    if is_known_default_location(&requested) {
        crate::cli::default_data_dir()
    } else {
        requested
    }
}

/// Whether `path` names a current or historical location exported by this
/// application for the active channel. Historical values are normalized only;
/// Fresh-v4 never opens their nested contents or migrates individual entries.
pub fn is_known_default_location(path: &Path) -> bool {
    known_default_locations()
        .iter()
        .any(|candidate| paths::paths_equivalent(path, candidate))
}

fn known_default_locations() -> Vec<PathBuf> {
    let legacy = crate::cli::legacy_default_data_dir();
    let junk_once = legacy.join("Nomi");
    let junk_twice = junk_once.join("Nomi");
    vec![
        crate::cli::default_data_dir(),
        legacy,
        junk_once,
        junk_twice,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn historical_self_exports_normalize_to_current_channel_root() {
        let current = crate::cli::default_data_dir();
        let legacy = crate::cli::legacy_default_data_dir();

        for historical in [
            legacy.clone(),
            legacy.join("Nomi"),
            legacy.join("Nomi").join("Nomi"),
        ] {
            assert_eq!(
                normalize_requested_startup_data_root(historical),
                current
            );
        }
    }

    #[test]
    fn explicit_custom_root_is_preserved() {
        let custom = std::env::temp_dir().join("nomifun-explicit-v4-root");
        assert_eq!(
            normalize_requested_startup_data_root(custom.clone()),
            custom
        );
    }

    #[test]
    fn known_default_locations_match_windows_verbatim_spelling() {
        let legacy = crate::cli::legacy_default_data_dir();
        assert!(is_known_default_location(&legacy));
        assert!(is_known_default_location(&crate::cli::default_data_dir()));
        #[cfg(windows)]
        {
            let verbatim =
                PathBuf::from(format!(r"\\?\{}", legacy.display()));
            assert!(is_known_default_location(&verbatim));
        }
    }

    #[test]
    fn stale_dev_root_archive_moves_the_whole_directory_without_deleting_it() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("NomiFun-dev");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("sentinel"), b"old").unwrap();

        let archive = archive_stale_dev_root(&root).unwrap();
        assert!(!root.exists());
        assert_eq!(std::fs::read(archive.join("sentinel")).unwrap(), b"old");
        assert!(
            archive
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("NomiFun-dev.stale-v4-archive-")
        );
    }
}
