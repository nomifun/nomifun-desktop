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
//! Two further extractions carry their own bounded budgets and do not use this
//! module; keep them in sync when the policy changes:
//! `nomi-browser-engine`'s `acquire.rs` (`enclosed_name`-based, writes to disk)
//! and `nomifun-ai-agent`'s `artifact_store.rs` `valid_zip` (in-memory
//! validation only — it never writes, so its raw entry names are not paths).

use std::ffi::{OsStr, OsString};
use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf};

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
    /// Reject `<letter>:` drive prefixes, plus — on Windows only — every other
    /// `':'` byte as well.
    ///
    /// Knowledge exports embed real on-disk file names, which may legally
    /// contain `':'` on Unix, so a Unix host keeps accepting `a:b.md`. Windows
    /// cannot: there `name:stream` opens an *alternate data stream* on `name`
    /// rather than a file called `name:stream`. That makes a non-prefix colon
    /// unsafe in both directions — `payload.exe:x.md` slips past an extension
    /// whitelist (`Path::extension` reads `Some("md")`) while writing into a
    /// stream on `payload.exe`, and a legitimately colon-named Unix export
    /// would land in a stream that is invisible to `read_dir` and reads back
    /// empty. Rejecting is what turns that silent loss into a real error.
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
    // `RejectDrivePrefix` still bans every ':' on Windows, where a non-prefix
    // colon means an alternate data stream rather than a file name (see
    // `ZipColonPolicy::RejectDrivePrefix`). The drive-prefix check below then
    // covers both policies on Unix, where ':' is an ordinary name byte.
    if name.contains(':') && (colon_policy == ZipColonPolicy::RejectAll || cfg!(windows)) {
        return None;
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return None;
    }
    let mut parts: Vec<&OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if parts.is_empty() {
                    // Held byte-wise on every platform (see ZipColonPolicy).
                    let first = part.to_string_lossy();
                    let bytes = first.as_bytes();
                    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                        return None;
                    }
                }
                parts.push(part);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    // Joined by hand rather than with `PathBuf::push`: `push` re-parses each
    // component as a path, so on Windows a *later* `a:b.md` reads as a
    // drive-relative `Prefix::Disk` and replaces the whole buffer
    // (`files/a:b.md` -> `a:b.md`), turning a contained entry into one that
    // `destination.join(..)` resolves outside the destination — zip-slip.
    //
    // Given the checks above (no backslash, and no `':'` on Windows) no
    // surviving component can still re-parse as a prefix, so today this is
    // defense in depth rather than the primary guard — the colon rejection is.
    // It is kept because it is what makes the containment property hold
    // *locally*, without depending on those earlier checks staying exactly as
    // strict: loosening the colon policy would make `push` unsafe again.
    let mut joined = OsString::new();
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            joined.push(MAIN_SEPARATOR_STR);
        }
        joined.push(part);
    }
    let safe_path = PathBuf::from(joined);
    // Defense in depth: re-parsing the assembled path must still yield a
    // prefix-free relative path, so callers' `destination.join(..)` stays put.
    if safe_path.is_absolute() || safe_path.components().any(|c| !matches!(c, Component::Normal(_))) {
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
        // The strict policy rejects every ':' byte on every platform.
        assert!(safe_zip_entry_path("files/a:b.md", ZipColonPolicy::RejectAll).is_none());

        // `RejectDrivePrefix` diverges only on Unix, where ':' is an ordinary
        // name byte. On Windows `a:b.md` names an alternate data stream on `a`,
        // so it is rejected under both policies.
        let rel = safe_zip_entry_path("files/a:b.md", ZipColonPolicy::RejectDrivePrefix);
        if cfg!(windows) {
            assert!(rel.is_none(), "Windows must reject an ADS-style entry name");
            return;
        }
        let rel = rel.expect("a non-prefix ':' is a legal Unix file name");
        assert_eq!(rel, PathBuf::from("files/a:b.md"));

        // The parent must survive: `PathBuf::push("a:b.md")` on Windows reads a
        // drive-relative prefix and would replace the buffer, yielding a bare
        // `a:b.md` that escapes the destination on join (see the join comment).
        assert_eq!(rel.parent(), Some(Path::new("files")));
        assert_eq!(rel.components().count(), 2);
        let joined = Path::new("dest").join(&rel);
        assert!(
            joined.starts_with("dest"),
            "sanitized entry must stay inside the destination: {joined:?}"
        );
    }

    /// The ADS bypass that motivates the Windows arm of `RejectDrivePrefix`:
    /// `Path::extension` reads `Some("md")` for `payload.exe:x.md`, so a `.md`
    /// whitelist alone would admit a write into a stream on `payload.exe`.
    #[test]
    fn ads_style_name_never_passes_an_extension_whitelist() {
        let name = "files/payload.exe:x.md";
        assert_eq!(
            Path::new(name).extension().and_then(|e| e.to_str()),
            Some("md"),
            "premise of this test: the ADS suffix mimics a .md extension"
        );
        assert!(safe_zip_entry_path(name, ZipColonPolicy::RejectAll).is_none());
        let rel = safe_zip_entry_path(name, ZipColonPolicy::RejectDrivePrefix);
        assert_eq!(
            rel.is_none(),
            cfg!(windows),
            "the ADS name must be rejected exactly where streams exist: {rel:?}"
        );
    }

    /// On Unix a `<letter>:` sequence past the first component is a legal name
    /// that must nonetheless never escape on join. On Windows every such name
    /// is a stream reference and is rejected outright.
    #[test]
    fn safe_path_keeps_deep_drive_like_components_contained() {
        for name in ["files/c:/evil.md", "a/b/z:x", "files/c:"] {
            let rel = safe_zip_entry_path(name, ZipColonPolicy::RejectDrivePrefix);
            if cfg!(windows) {
                assert!(rel.is_none(), "Windows must reject stream-style name: {name:?}");
                continue;
            }
            let rel = rel.unwrap_or_else(|| panic!("must accept legal deep name: {name:?}"));
            assert!(
                rel.is_relative() && rel.components().all(|c| matches!(c, Component::Normal(_))),
                "sanitized {name:?} must stay prefix-free relative: {rel:?}"
            );
            let joined = Path::new("dest").join(&rel);
            assert!(
                joined.starts_with("dest"),
                "sanitized {name:?} escaped the destination: {joined:?}"
            );
            assert!(rel.starts_with(name.split('/').next().unwrap()), "lost the parent of {name:?}");
        }
    }

    /// Regression guard for path assembly. Deliberately does NOT use a `':'`
    /// name: the colon policy rejects those on Windows *before* the join runs,
    /// so a colon-based test here would pass even with the original
    /// `PathBuf::push` bug reinstated — it would mask the defect it claims to
    /// pin. (Verified by mutation: reinstating `push` leaves this suite green,
    /// because with colons and backslashes already rejected no component can
    /// re-parse as a prefix. See the join comment — that code is now defense in
    /// depth, and the colon rejection is the load-bearing guard.)
    ///
    /// What this does pin is the containment property every caller relies on:
    /// an accepted name keeps all of its components and stays inside the
    /// destination on join.
    #[test]
    fn join_preserves_every_component_on_both_platforms() {
        for name in ["files/notes/deep/a.md", "a/b/c/d/e.md", "files/plain.md"] {
            for policy in [ZipColonPolicy::RejectAll, ZipColonPolicy::RejectDrivePrefix] {
                let rel = safe_zip_entry_path(name, policy)
                    .unwrap_or_else(|| panic!("must accept {name:?} under {policy:?}"));
                assert_eq!(
                    rel.components().count(),
                    name.split('/').count(),
                    "join lost a component of {name:?}: {rel:?}"
                );
                assert_eq!(rel, PathBuf::from(name.replace('/', MAIN_SEPARATOR_STR)));
                assert!(rel.is_relative(), "{rel:?} must stay relative");
                let joined = Path::new("dest").join(&rel);
                assert!(joined.starts_with("dest"), "{name:?} escaped: {joined:?}");
            }
        }
    }

    /// The `PathBuf::push` hazard itself, asserted against the standard library
    /// rather than against our sanitizer, so the reason the hand-rolled join
    /// exists stays documented and checked. If a future Rust stopped replacing
    /// the buffer here, this would flag that the hand-join is no longer needed.
    #[test]
    #[cfg(windows)]
    fn push_replaces_the_buffer_on_a_drive_relative_component() {
        let mut built = PathBuf::new();
        built.push("files");
        built.push("a:b.md");
        assert_eq!(
            built,
            PathBuf::from("a:b.md"),
            "premise of the hand-rolled join: push() drops the parent here"
        );
        assert!(!built.is_absolute(), "and is_absolute() does not catch it");
        assert!(
            !Path::new("C:\\dest").join(&built).starts_with("C:\\dest"),
            "which is what makes it a zip-slip escape"
        );
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
