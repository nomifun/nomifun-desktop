//! Shared safety primitives for extracting untrusted zip archives: zip-slip
//! entry-name sanitization, symlink-entry detection, and a decompression-bomb
//! budget (entry-count cap + cumulative actually-written bytes).
//!
//! Used by the skill importer (`nomifun-extension`), the knowledge-base
//! importer (`nomifun-knowledge`) and the companion-bundle importer
//! (`nomifun-companion`). The extract loops themselves stay in the caller
//! crates — their entry whitelists and duplicate-entry policies differ — only
//! the security-critical primitives live here.
//!
//! Deliberately `zip`-crate-free: callers pass the entry *name* and *unix
//! mode*, so this lowest-layer crate does not grow a `zip` dependency.
//!
//! A fourth, in-memory extraction with its own bounded budget lives in
//! `nomifun-workshop`'s `archive.rs` (`enclosed_name`-based); keep the two in
//! sync when the policy changes.

use std::path::{Component, Path, PathBuf};

/// How `':'` bytes in entry names are treated by [`safe_zip_entry_path`].
///
/// A Windows drive prefix (`C:/…`) parses as `Component::Prefix` only on
/// Windows — on Unix it is a plain `Normal` component — so byte checks are
/// the only portable way to enforce a no-drive-prefix policy everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipColonPolicy {
    /// Reject any `':'` byte anywhere in the entry name. Strictest; for
    /// packages whose own exporter never writes one (companion bundles).
    RejectAll,
    /// Reject only `<letter>:` drive prefixes at the start of the first
    /// component; later `':'` bytes stay legal (knowledge exports embed real
    /// on-disk file names, which may legally contain `':'` on Unix).
    RejectDrivePrefix,
}

/// Resolve a zip entry name into a safe relative path, or `None` when writing
/// the entry could escape the destination: empty names, backslashes, absolute
/// paths, `..`/prefix components, and `':'` bytes per `colon_policy` are all
/// rejected. Leading `./` components are normalized away.
pub fn safe_zip_entry_path(name: &str, colon_policy: ZipColonPolicy) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\\') {
        return None;
    }
    if colon_policy == ZipColonPolicy::RejectAll && name.contains(':') {
        return None;
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return None;
    }
    let mut safe_path = PathBuf::new();
    let mut saw_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if !saw_normal {
                    // Held byte-wise on every platform (see ZipColonPolicy).
                    let first = part.to_string_lossy();
                    let bytes = first.as_bytes();
                    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                        return None;
                    }
                    saw_normal = true;
                }
                safe_path.push(part);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    if safe_path.as_os_str().is_empty() {
        return None;
    }
    Some(safe_path)
}

/// True when a zip entry's unix mode marks it a symlink (`S_IFLNK`), which
/// could redirect a subsequent write outside the destination. Pass
/// `entry.unix_mode()`.
pub fn zip_entry_is_symlink(unix_mode: Option<u32>) -> bool {
    unix_mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

/// Which cap a [`ZipExtractionBudget`] blew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipBudgetExceeded {
    /// The archive declares more entries than the budget allows.
    Entries { entries: usize, max_entries: usize },
    /// Cumulative written bytes passed the uncompressed-size cap.
    TotalBytes { max_total_uncompressed_bytes: u64 },
}

impl std::fmt::Display for ZipBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Entries { entries, max_entries } => {
                write!(f, "Zip archive has too many entries ({entries} > {max_entries})")
            }
            Self::TotalBytes {
                max_total_uncompressed_bytes,
            } => write!(
                f,
                "Zip archive expands beyond {max_total_uncompressed_bytes} bytes; \
                 refusing to extract a potential decompression bomb"
            ),
        }
    }
}

/// Decompression-bomb budget for one archive extraction: an entry-count cap
/// checked up front and a cumulative cap on bytes *actually written* — never
/// trust an entry's self-declared size, bomb archives lie about it.
#[derive(Debug)]
pub struct ZipExtractionBudget {
    max_entries: usize,
    max_total_uncompressed_bytes: u64,
    total_written: u64,
}

impl Default for ZipExtractionBudget {
    /// Budget with the default caps shared by all importers.
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES, Self::DEFAULT_MAX_ENTRIES)
    }
}

impl ZipExtractionBudget {
    /// Cumulative uncompressed-bytes cap across all entries of one archive.
    /// A tiny zip expanding past this is a decompression bomb, not user data.
    pub const DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
    /// Entry-count cap for one archive.
    pub const DEFAULT_MAX_ENTRIES: usize = 20_000;

    /// Budget with injectable caps, so tests can exercise the guards without
    /// multi-hundred-MiB fixtures.
    pub fn new(max_total_uncompressed_bytes: u64, max_entries: usize) -> Self {
        Self {
            max_entries,
            max_total_uncompressed_bytes,
            total_written: 0,
        }
    }

    /// Check the archive's declared entry count before extracting anything.
    pub fn check_entry_count(&self, entries: usize) -> Result<(), ZipBudgetExceeded> {
        if entries > self.max_entries {
            return Err(ZipBudgetExceeded::Entries {
                entries,
                max_entries: self.max_entries,
            });
        }
        Ok(())
    }

    /// Record the bytes actually written for one entry (`io::copy`'s return)
    /// and fail once the cumulative total passes the cap.
    pub fn record_written(&mut self, written: u64) -> Result<(), ZipBudgetExceeded> {
        self.total_written = self.total_written.saturating_add(written);
        if self.total_written > self.max_total_uncompressed_bytes {
            return Err(ZipBudgetExceeded::TotalBytes {
                max_total_uncompressed_bytes: self.max_total_uncompressed_bytes,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_accepts_normal_and_stripped_nested() {
        for policy in [ZipColonPolicy::RejectAll, ZipColonPolicy::RejectDrivePrefix] {
            assert_eq!(safe_zip_entry_path("a/b.md", policy).unwrap(), PathBuf::from("a/b.md"));
            // Leading `./` is normalized away.
            assert_eq!(
                safe_zip_entry_path("./a/b.md", policy).unwrap(),
                PathBuf::from("a/b.md")
            );
        }
    }

    #[test]
    fn safe_path_rejects_traversal_absolute_and_drive_prefix() {
        for policy in [ZipColonPolicy::RejectAll, ZipColonPolicy::RejectDrivePrefix] {
            for bad in [
                "",
                "..",
                "../evil",
                "a/../b",
                "/abs/path",
                "a\\b",
                "\\\\server\\share",
                "C:/evil.md",
                "c:",
            ] {
                assert!(
                    safe_zip_entry_path(bad, policy).is_none(),
                    "must reject unsafe zip entry name under {policy:?}: {bad:?}"
                );
            }
        }
    }

    #[test]
    fn colon_policy_diverges_only_on_non_prefix_colons() {
        // A ':' beyond the drive-prefix position is a legal Unix file name…
        assert_eq!(
            safe_zip_entry_path("files/a:b.md", ZipColonPolicy::RejectDrivePrefix).unwrap(),
            PathBuf::from("files/a:b.md")
        );
        // …but the strict policy rejects every ':' byte.
        assert!(safe_zip_entry_path("files/a:b.md", ZipColonPolicy::RejectAll).is_none());
    }

    #[test]
    fn symlink_mode_detection() {
        assert!(zip_entry_is_symlink(Some(0o120777)));
        assert!(!zip_entry_is_symlink(Some(0o100644)));
        assert!(!zip_entry_is_symlink(Some(0o040755)));
        assert!(!zip_entry_is_symlink(None));
    }

    #[test]
    fn budget_caps_entries_and_written_bytes() {
        let budget = ZipExtractionBudget::new(100, 4);
        assert!(budget.check_entry_count(4).is_ok());
        assert!(matches!(
            budget.check_entry_count(5),
            Err(ZipBudgetExceeded::Entries { entries: 5, max_entries: 4 })
        ));

        let mut budget = ZipExtractionBudget::new(100, 4);
        assert!(budget.record_written(60).is_ok());
        assert!(budget.record_written(40).is_ok(), "cap is inclusive");
        assert!(matches!(
            budget.record_written(1),
            Err(ZipBudgetExceeded::TotalBytes { max_total_uncompressed_bytes: 100 })
        ));
    }
}
