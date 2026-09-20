//! Cross-platform path spelling helpers shared by every host and lifecycle
//! component.
//!
//! On Windows, `std::fs::canonicalize` returns verbatim (`\\?\`-prefixed)
//! extended-length paths. Those are correct for filesystem syscalls but leak
//! badly everywhere else: persisted lifecycle markers, `NOMIFUN_DATA_DIR` /
//! `NOMIFUN_WORK_DIR` env exports, error dialogs, and API responses all end up
//! showing `\\?\C:\Users\...`. Worse, *string* comparisons between a stored
//! marker and a freshly canonicalized path silently diverge when one side has
//! the prefix and the other does not.
//!
//! The rules encoded here:
//! * [`canonicalize_simplified`] is the project-wide replacement for
//!   `std::fs::canonicalize` whenever the result is stored, exported,
//!   compared, or displayed. It resolves the real path and then strips the
//!   verbatim prefix when the path is losslessly representable without it
//!   (via `dunce::simplified`).
//! * [`simplified`] normalizes an already-resolved path for comparison or
//!   display without touching the filesystem.
//! * [`paths_equivalent`] compares two path *spellings* of already-canonical
//!   paths, tolerating the `\\?\` prefix on either side. On Windows it also
//!   compares the OS identity of existing local paths, because package file
//!   virtualization can expose one directory through both `%LOCALAPPDATA%`
//!   and `Packages\<identity>\LocalCache\Local` without a lexical relationship.
//!   Durable markers written through either view must keep matching the same
//!   directory.

use std::path::{Path, PathBuf};

/// Canonicalize `path` and strip the Windows verbatim prefix from the result
/// when it is losslessly representable without it. Use this instead of
/// `std::fs::canonicalize` for any value that is persisted, exported through
/// the environment, compared as a string, or shown to the user.
pub fn canonicalize_simplified(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)?;
    Ok(simplified(&canonical))
}

/// Strip the Windows verbatim (`\\?\`) prefix from an already-resolved path
/// when possible. On non-Windows platforms this is the identity function.
pub fn simplified(path: &Path) -> PathBuf {
    dunce::simplified(path).to_path_buf()
}

/// Compare two spellings of already-canonicalized paths, tolerating the
/// Windows verbatim prefix on either side and package-virtualized aliases.
///
/// The lexical comparison is sufficient on ordinary filesystems. Windows can
/// preserve two different canonical spellings for the same package-virtualized
/// local directory, so an existing drive-backed pair falls back to the stable
/// file identity reported by the OS. UNC paths deliberately remain lexical so
/// a marker comparison never initiates access to an unrelated network root.
pub fn paths_equivalent(a: &Path, b: &Path) -> bool {
    let a = dunce::simplified(a);
    let b = dunce::simplified(b);
    if a == b {
        return true;
    }
    #[cfg(windows)]
    {
        windows_local_paths_share_identity(a, b)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn windows_local_paths_share_identity(a: &Path, b: &Path) -> bool {
    use std::path::{Component, Prefix};

    let is_drive_path = |path: &Path| {
        matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        )
    };
    is_drive_path(a)
        && is_drive_path(b)
        && same_file::is_same_file(a, b).unwrap_or(false)
}

/// String-typed convenience for durable markers: does the stored spelling
/// refer to the same canonical path as `canonical`?
pub fn stored_path_matches(stored: &str, canonical: &Path) -> bool {
    !stored.is_empty() && paths_equivalent(Path::new(stored), canonical)
}

/// The canonical string spelling used when persisting a path into a durable
/// marker: simplified (never `\\?\`-prefixed) display form.
pub fn marker_string(canonical: &Path) -> String {
    simplified(canonical).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplified_is_identity_for_plain_paths() {
        let plain = if cfg!(windows) {
            PathBuf::from(r"C:\Users\example\AppData\Local\NomiFun")
        } else {
            PathBuf::from("/home/example/.local/share/NomiFun")
        };
        assert_eq!(simplified(&plain), plain);
    }

    #[cfg(windows)]
    #[test]
    fn simplified_strips_verbatim_prefix() {
        assert_eq!(
            simplified(Path::new(r"\\?\C:\Users\example\NomiFun")),
            PathBuf::from(r"C:\Users\example\NomiFun")
        );
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_and_plain_spellings_are_equivalent() {
        assert!(paths_equivalent(
            Path::new(r"\\?\C:\Users\example\NomiFun"),
            Path::new(r"C:\Users\example\NomiFun"),
        ));
        assert!(stored_path_matches(
            r"\\?\C:\Users\example\NomiFun",
            Path::new(r"C:\Users\example\NomiFun"),
        ));
        assert!(!paths_equivalent(
            Path::new(r"\\?\C:\Users\example\NomiFun\Nomi"),
            Path::new(r"C:\Users\example\NomiFun"),
        ));
    }

    #[cfg(windows)]
    #[test]
    fn existing_windows_aliases_are_equivalent_by_file_identity() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = canonicalize_simplified(dir.path()).unwrap();
        let case_alias = PathBuf::from(canonical.to_string_lossy().to_uppercase());
        assert_ne!(simplified(&case_alias), canonical);
        assert!(paths_equivalent(&case_alias, &canonical));
        assert!(stored_path_matches(
            &case_alias.display().to_string(),
            &canonical,
        ));

        let other = tempfile::tempdir().unwrap();
        assert!(!paths_equivalent(other.path(), &canonical));
    }

    #[test]
    fn empty_stored_value_never_matches() {
        assert!(!stored_path_matches("", Path::new("/")));
    }

    #[test]
    fn canonicalize_simplified_has_no_verbatim_prefix() {
        let dir = std::env::temp_dir();
        let canonical = canonicalize_simplified(&dir).unwrap();
        assert!(
            !canonical.display().to_string().starts_with(r"\\?\"),
            "canonicalize_simplified must not leak the verbatim prefix: {}",
            canonical.display()
        );
    }

    #[test]
    fn marker_string_round_trips_through_stored_path_matches() {
        let dir = canonicalize_simplified(&std::env::temp_dir()).unwrap();
        let stored = marker_string(&dir);
        assert!(stored_path_matches(&stored, &dir));
    }
}
