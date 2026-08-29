//! Shared startup data-root resolution and Fresh-v4 cutover entry point.
//!
//! Known historical self-export paths are normalized to the current
//! channel-specific canonical root. The Fresh-v4 coordinator then either
//! creates a new root or atomically renames the entire existing canonical
//! root to its opaque timestamped sibling before creating v4 at the original
//! path. No legacy entry is enumerated, copied, imported, or restored.

use std::path::{Path, PathBuf};

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
    let outcome = super::v4_root::bootstrap_data_root(&canonical)
        .unwrap_or_else(|error| panic!("{error:#}"));

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
}
