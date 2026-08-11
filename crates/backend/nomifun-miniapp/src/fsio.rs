//! Atomic temp+rename writes for the mini-app working copy on disk.
//!
//! A verbatim copy of `nomifun-workshop/src/fsio.rs`: unique-temp + rename, so a
//! killed process leaves either the previous complete file or the new complete
//! one — never a spliced document. Async because a working copy is up to 4 MiB
//! and blocking the runtime on it is unacceptable.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Atomically persist `bytes` to `{dir}/{file}` (unique-temp + rename). Creates
/// `dir` if missing. On any failure the temp file is best-effort removed.
pub(crate) async fn save_bytes_atomic(dir: &Path, file: &str, bytes: &[u8]) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let path = dir.join(file);
    let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{file}.tmp.{}.{seq}", std::process::id()));
    let result = async {
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, &path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

/// Read a file to bytes, or `None` when it does not exist. Other IO errors
/// propagate.
pub(crate) async fn read_bytes_opt(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Last-modified time of `path` in ms since the epoch, or `None` when the file is
/// absent. A clock that predates the epoch (or an OS without mtime) reports 0
/// rather than failing: the caller compares it against a publish timestamp, and
/// "unknown, treat as ancient" is the safe direction — it reports "no unpublished
/// changes" instead of nagging about a file nobody edited.
pub(crate) async fn modified_ms_opt(path: &Path) -> std::io::Result<Option<i64>> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let modified = metadata.modified()?;
    let ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    Ok(Some(ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn atomic_write_then_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a").join("b");
        save_bytes_atomic(&sub, "x.bin", b"hello").await.unwrap();
        let read = read_bytes_opt(&sub.join("x.bin")).await.unwrap();
        assert_eq!(read.as_deref(), Some(&b"hello"[..]));
        // no temp files linger
        let leftover = std::fs::read_dir(&sub).unwrap().filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp.")
        });
        assert_eq!(leftover.count(), 0);
    }

    #[tokio::test]
    async fn read_missing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            read_bytes_opt(&dir.path().join("nope"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn modified_ms_is_present_for_a_written_file_and_absent_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            modified_ms_opt(&dir.path().join("nope")).await.unwrap(),
            None
        );
        save_bytes_atomic(dir.path(), "x.html", b"<p/>").await.unwrap();
        let mtime = modified_ms_opt(&dir.path().join("x.html"))
            .await
            .unwrap()
            .expect("a written file has an mtime");
        assert!(mtime > 0, "mtime must be a real epoch value, got {mtime}");
    }
}
