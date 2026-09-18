//! Canonical startup data-root resolution.
//!
//! Historical self-export locations normalize to the one channel-specific
//! root. The application never probes, opens, imports, or redirects to the
//! retired parallel Agent root.

use std::path::{Path, PathBuf};

use nomifun_common::paths;

/// Retained for the published layout relocation reader.
pub const LAYOUT_MIGRATION_PENDING_MARKER: &str =
    ".nomifun-layout-migration.pending";
/// Retained for the published layout relocation reader.
pub const RELOCATED_FROM_MARKER: &str = ".relocated-from";
/// Retained for the published layout relocation reader.
pub const RELOCATED_DONE_MARKER: &str = ".relocated-done";

/// Published relocation payload kept byte/schema-compatible with the
/// one canonical data-root reader.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RelocationMarker {
    pub old_root: String,
    #[serde(default)]
    pub relocated_at_ms: i64,
}

/// Resolve the only production data root.
pub fn resolve_startup_data_root(requested: PathBuf) -> PathBuf {
    normalize_requested_startup_data_root(requested)
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
/// application for the active channel. Historical values normalize to the
/// current root; their nested contents are never imported individually.
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
        let custom = std::env::temp_dir().join("nomifun-explicit-root");
        assert_eq!(
            normalize_requested_startup_data_root(custom.clone()),
            custom
        );
    }
}
