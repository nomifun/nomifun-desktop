//! Atomic temp+rename writes for Creative Studio asset binaries. The helper is
//! asynchronous because payloads can be tens of MB and must not block the
//! runtime.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn atomic_write_then_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a").join("b");
        save_bytes_atomic(&sub, "x.bin", b"hello").await.unwrap();
        let read = tokio::fs::read(sub.join("x.bin")).await.unwrap();
        assert_eq!(read, b"hello");
        // no temp files linger
        let leftover = std::fs::read_dir(&sub).unwrap().filter(|e| {
            e.as_ref().unwrap().file_name().to_string_lossy().contains(".tmp.")
        });
        assert_eq!(leftover.count(), 0);
    }
}
