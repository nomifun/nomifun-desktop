//! Shared, safety-hardened zip extraction. Used by skill import
//! ([`crate::skill_service`]). Guards against zip-slip (path traversal),
//! symlink entries, and decompression bombs (entry-count and cumulative
//! uncompressed-size caps) so an untrusted archive can never write outside
//! `destination` or exhaust the disk. The security primitives (entry-name
//! sanitization, symlink detection, bomb budget) are the shared
//! [`nomifun_common::zip_safe`] hardening also used by the knowledge and
//! companion importers.

use std::io;
use std::path::{Path, PathBuf};

use nomifun_common::zip_safe::{self, ZipColonPolicy, ZipExtractionBudget};

use crate::error::ExtensionError;

/// Extract every entry of `archive_path` into `destination`, rejecting any entry
/// whose name escapes `destination` (absolute, `..`, backslash, drive prefix),
/// that is a symlink, or that would blow the default bomb caps
/// ([`ZipExtractionBudget::DEFAULT_MAX_ENTRIES`] entries /
/// [`ZipExtractionBudget::DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES`] cumulative
/// uncompressed bytes).
/// Synchronous — run under `tokio::task::spawn_blocking` off the reactor.
pub(crate) fn extract_zip_archive(archive_path: &Path, destination: &Path) -> Result<(), ExtensionError> {
    extract_zip_archive_with_budget(archive_path, destination, ZipExtractionBudget::default())
}

/// [`extract_zip_archive`] with an injectable budget, split out so tests can
/// exercise the bomb guards without multi-hundred-MiB fixtures.
fn extract_zip_archive_with_budget(
    archive_path: &Path,
    destination: &Path,
    mut budget: ZipExtractionBudget,
) -> Result<(), ExtensionError> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;

    budget
        .check_entry_count(archive.len())
        .map_err(|e| ExtensionError::InvalidSkillPath(e.to_string()))?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let entry_name = entry.name().to_string();
        reject_zip_symlink(&entry)?;
        let relative_path = safe_zip_entry_path(&entry_name)?;
        let output_path = destination.join(relative_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&output_path)?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(&output_path)?;
        // The budget tracks ACTUAL bytes written (io::copy's return), not the
        // entry's self-declared size — bomb archives lie about their sizes.
        let written = io::copy(&mut entry, &mut output)?;
        budget
            .record_written(written)
            .map_err(|e| ExtensionError::InvalidSkillPath(e.to_string()))?;
    }

    Ok(())
}

/// Resolve a zip entry name to a safe relative path, or reject it. Rejects
/// empty names, backslashes, absolute paths, drive prefixes, and any
/// `..`/root component (shared [`zip_safe`] policy).
pub(crate) fn safe_zip_entry_path(name: &str) -> Result<PathBuf, ExtensionError> {
    zip_safe::safe_zip_entry_path(name, ZipColonPolicy::RejectDrivePrefix)
        .ok_or_else(|| ExtensionError::PathTraversal(name.to_string()))
}

/// Reject symlink entries (unix mode `S_IFLNK`), which could otherwise redirect
/// a subsequent write outside `destination`.
fn reject_zip_symlink(entry: &zip::read::ZipFile<'_>) -> Result<(), ExtensionError> {
    if zip_safe::zip_entry_is_symlink(entry.unix_mode()) {
        return Err(ExtensionError::PathTraversal(entry.name().to_string()));
    }
    Ok(())
}

fn zip_error(err: zip::result::ZipError) -> ExtensionError {
    ExtensionError::InvalidSkillPath(format!("Invalid zip archive: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn safe_path_accepts_normal_and_stripped_nested() {
        assert_eq!(safe_zip_entry_path("a/b.md").unwrap(), PathBuf::from("a/b.md"));
        // Leading `./` is normalized away.
        assert_eq!(safe_zip_entry_path("./a/b.md").unwrap(), PathBuf::from("a/b.md"));
    }

    #[test]
    fn safe_path_rejects_traversal_and_absolute() {
        for bad in ["", "..", "../evil", "a/../b", "/abs/path", "a\\b", "\\\\server\\share", "C:/evil.md"] {
            assert!(
                safe_zip_entry_path(bad).is_err(),
                "must reject unsafe zip entry name: {bad:?}"
            );
        }
    }

    #[test]
    fn extract_writes_nested_tree_and_top_level_files() {
        let tmp = TempDir::new().unwrap();
        let zip_path = tmp.path().join("test.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("VERSION", opts).unwrap();
            w.write_all(b"9.9.9").unwrap();
            w.start_file("skills/tdd/SKILL.md", opts).unwrap();
            w.write_all(b"---\nname: tdd\n---\n").unwrap();
            w.finish().unwrap();
        }

        let dest = tmp.path().join("out");
        extract_zip_archive(&zip_path, &dest).unwrap();

        assert_eq!(std::fs::read_to_string(dest.join("VERSION")).unwrap(), "9.9.9");
        assert!(dest.join("skills/tdd/SKILL.md").is_file());
    }

    /// Decompression-bomb guard: an entry that inflates past the cumulative
    /// uncompressed cap aborts the extraction, measured by bytes actually
    /// written rather than the entry's self-declared size.
    #[test]
    fn extract_rejects_archive_exceeding_uncompressed_cap() {
        let tmp = TempDir::new().unwrap();
        let zip_path = tmp.path().join("bomb.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            // Highly compressible 64 KiB body → tiny archive, big expansion.
            w.start_file("big.bin", opts).unwrap();
            w.write_all(&vec![0u8; 64 * 1024]).unwrap();
            w.finish().unwrap();
        }

        // 4 KiB test cap: the 64 KiB expansion must be refused...
        let dest = tmp.path().join("out");
        let err = extract_zip_archive_with_budget(
            &zip_path,
            &dest,
            ZipExtractionBudget::new(4 * 1024, ZipExtractionBudget::DEFAULT_MAX_ENTRIES),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("decompression bomb"),
            "unexpected error: {err}"
        );

        // ...while a sufficient cap extracts the same archive fine.
        let dest_ok = tmp.path().join("out-ok");
        extract_zip_archive_with_budget(
            &zip_path,
            &dest_ok,
            ZipExtractionBudget::new(128 * 1024, ZipExtractionBudget::DEFAULT_MAX_ENTRIES),
        )
        .unwrap();
        assert!(dest_ok.join("big.bin").is_file());
    }

    /// Entry-count guard: too many entries are refused before extraction.
    #[test]
    fn extract_rejects_archive_exceeding_entry_cap() {
        let tmp = TempDir::new().unwrap();
        let zip_path = tmp.path().join("many.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            for i in 0..8 {
                w.start_file(format!("f{i}.txt"), opts).unwrap();
                w.write_all(b"x").unwrap();
            }
            w.finish().unwrap();
        }

        let dest = tmp.path().join("out");
        let err = extract_zip_archive_with_budget(
            &zip_path,
            &dest,
            ZipExtractionBudget::new(ZipExtractionBudget::DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES, 4),
        )
        .unwrap_err();
        assert!(err.to_string().contains("too many entries"), "unexpected error: {err}");
        // Nothing was written: the count check runs before any extraction.
        assert!(!dest.exists());
    }
}
