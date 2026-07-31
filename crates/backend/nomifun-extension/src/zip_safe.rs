//! Shared, safety-hardened zip extraction. Used by skill import
//! ([`crate::skill_service`]). Guards against zip-slip (path traversal),
//! symlink entries, and decompression bombs (entry-count and cumulative
//! uncompressed-size caps) so an untrusted archive can never write outside
//! `destination` or exhaust the disk.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::ExtensionError;

/// Cumulative uncompressed-bytes cap across all entries of one archive.
/// A tiny zip expanding past this is a decompression bomb, not a skill pack.
const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
/// Entry-count cap for one archive.
const MAX_ENTRIES: usize = 20_000;

/// Extract every entry of `archive_path` into `destination`, rejecting any entry
/// whose name escapes `destination` (absolute, `..`, backslash), that is a
/// symlink, or that would blow the bomb caps ([`MAX_ENTRIES`] entries /
/// [`MAX_TOTAL_UNCOMPRESSED_BYTES`] cumulative uncompressed bytes).
/// Synchronous — run under `tokio::task::spawn_blocking` off the reactor.
pub(crate) fn extract_zip_archive(archive_path: &Path, destination: &Path) -> Result<(), ExtensionError> {
    extract_zip_archive_with_limits(archive_path, destination, MAX_TOTAL_UNCOMPRESSED_BYTES, MAX_ENTRIES)
}

/// [`extract_zip_archive`] with injectable caps, split out so tests can
/// exercise the bomb guards without multi-hundred-MiB fixtures.
fn extract_zip_archive_with_limits(
    archive_path: &Path,
    destination: &Path,
    max_total_uncompressed_bytes: u64,
    max_entries: usize,
) -> Result<(), ExtensionError> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;

    if archive.len() > max_entries {
        return Err(ExtensionError::InvalidSkillPath(format!(
            "Zip archive has too many entries ({} > {max_entries})",
            archive.len()
        )));
    }

    let mut total_uncompressed: u64 = 0;
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
        // Track ACTUAL bytes written (io::copy's return), not the entry's
        // self-declared size — bomb archives lie about their sizes.
        let written = io::copy(&mut entry, &mut output)?;
        total_uncompressed = total_uncompressed.saturating_add(written);
        if total_uncompressed > max_total_uncompressed_bytes {
            return Err(ExtensionError::InvalidSkillPath(format!(
                "Zip archive expands beyond {max_total_uncompressed_bytes} bytes; \
                 refusing to extract a potential decompression bomb"
            )));
        }
    }

    Ok(())
}

/// Resolve a zip entry name to a safe relative path, or reject it. Rejects
/// empty names, backslashes, absolute paths, and any `..`/root component.
pub(crate) fn safe_zip_entry_path(name: &str) -> Result<PathBuf, ExtensionError> {
    if name.is_empty() || name.contains('\\') {
        return Err(ExtensionError::PathTraversal(name.to_string()));
    }

    let path = Path::new(name);
    if path.is_absolute() {
        return Err(ExtensionError::PathTraversal(name.to_string()));
    }

    let mut safe_path = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe_path.push(part),
            Component::CurDir => {}
            _ => return Err(ExtensionError::PathTraversal(name.to_string())),
        }
    }

    if safe_path.as_os_str().is_empty() {
        return Err(ExtensionError::PathTraversal(name.to_string()));
    }

    Ok(safe_path)
}

/// Reject symlink entries (unix mode `S_IFLNK`), which could otherwise redirect
/// a subsequent write outside `destination`.
fn reject_zip_symlink(entry: &zip::read::ZipFile<'_>) -> Result<(), ExtensionError> {
    if let Some(mode) = entry.unix_mode()
        && mode & 0o170000 == 0o120000
    {
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
        for bad in ["", "..", "../evil", "a/../b", "/abs/path", "a\\b", "\\\\server\\share"] {
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
        let err = extract_zip_archive_with_limits(&zip_path, &dest, 4 * 1024, MAX_ENTRIES).unwrap_err();
        assert!(
            err.to_string().contains("decompression bomb"),
            "unexpected error: {err}"
        );

        // ...while a sufficient cap extracts the same archive fine.
        let dest_ok = tmp.path().join("out-ok");
        extract_zip_archive_with_limits(&zip_path, &dest_ok, 128 * 1024, MAX_ENTRIES).unwrap();
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
        let err =
            extract_zip_archive_with_limits(&zip_path, &dest, MAX_TOTAL_UNCOMPRESSED_BYTES, 4).unwrap_err();
        assert!(err.to_string().contains("too many entries"), "unexpected error: {err}");
        // Nothing was written: the count check runs before any extraction.
        assert!(!dest.exists());
    }
}
