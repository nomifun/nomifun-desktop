//! Canonical startup data-root resolution.
//!
//! Historical self-export locations normalize to the one channel-specific
//! root. The application never probes, opens, imports, or redirects to the
//! retired parallel Agent root.

use std::path::{Path, PathBuf};

use anyhow::Context;
use nomifun_common::paths;

/// Resolve the only production data root.
pub fn resolve_startup_data_root(requested: PathBuf) -> anyhow::Result<PathBuf> {
    let requested = normalize_requested_startup_data_root(requested);
    let requested = canonical_startup_path(&requested)?;
    if let Some(control) = nomifun_common::storage_relocation::resolve_control_dir(&requested)?
        && let Some(location) = nomifun_common::storage_relocation::resolve_storage_location(&control)? {
        return Ok(location.data_dir);
    }
    Ok(requested)
}

/// Preserve CLI-relative paths and existing host aliases while looking up one
/// canonical control scope. Resolve existing ancestors without creating data.
fn canonical_startup_path(requested: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!requested.as_os_str().is_empty(), "startup data root must not be empty");
    let absolute = std::path::absolute(requested)?;
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match paths::canonicalize_simplified(ancestor) {
            Ok(mut canonical) => {
                for name in missing.into_iter().rev() {
                    canonical.push(name);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A committed locator lives outside the movable roots. Keep
                // its original absolute address available for lookup even
                // when that drive/share is offline; the coordinator still
                // validates the exact committed destination before using it.
                if ancestor.file_name().is_none() || ancestor.parent().is_none() {
                    return Ok(absolute);
                }
                missing.push(ancestor.file_name().context("startup data root has no existing ancestor")?.to_os_string());
                ancestor = ancestor.parent().context("startup data root has no existing ancestor")?;
            }
            Err(error) => return Err(error).with_context(|| format!("resolve startup data root {}", requested.display())),
        }
    }
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

    #[test]
    fn storage_relocation_locator_preserves_relative_startup_paths_without_creating_data() {
        let current = std::env::current_dir().unwrap();
        let fixture = tempfile::tempdir_in(&current).unwrap();
        let relative = fixture.path().strip_prefix(&current).unwrap();
        let expected = paths::canonicalize_simplified(fixture.path()).unwrap();
        assert_eq!(resolve_startup_data_root(relative.to_path_buf()).unwrap(), expected);
        let missing = relative.join("not-created").join("data");
        assert_eq!(resolve_startup_data_root(missing.clone()).unwrap(), expected.join("not-created").join("data"));
        assert!(!current.join(missing).exists());
    }

    #[cfg(windows)]
    #[test]
    fn storage_relocation_original_drive_can_be_offline_before_locator_lookup() {
        let unavailable_drive = ('A'..='Z').map(|drive| PathBuf::from(format!("{drive}:\\")))
            .find(|root| matches!(root.try_exists(), Ok(false)));
        let Some(drive) = unavailable_drive else { return; };
        let requested = drive.join(format!("nomifun-offline-origin-{}", uuid::Uuid::now_v7()));
        assert_eq!(canonical_startup_path(&requested).unwrap(), requested);
        assert!(!requested.exists());
    }
}
