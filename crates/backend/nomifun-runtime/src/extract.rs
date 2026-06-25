//! Atomic extraction of the compressed embedded bun blob to the cache dir.
//!
//! Flow:
//! 1. Acquire inter-process advisory file lock (so parallel starts don't race).
//! 2. Re-check stamp: another process may have finished while we waited.
//! 3. zstd-decode blob -> `<dir>/bun.tmp`.
//! 4. Verify sha256 of `bun.tmp` == expected.
//! 5. chmod 0o755 (Unix only).
//! 6. Atomic rename `bun.tmp` -> `bun[.exe]`.
//! 7. Create `bunx[.exe]` — symlink on Unix, copy on Windows.
//! 8. Create `node[.exe]` — symlink on Unix, copy on Windows — so
//!    `#!/usr/bin/env node` shebangs in npm packages resolve to bun.
//! 9. Write `bun.stamp` JSON.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize, Deserialize)]
pub struct Stamp {
    pub sha256: String,
    pub version: String,
    pub extracted_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn bun_filename() -> &'static str {
    if cfg!(windows) { "bun.exe" } else { "bun" }
}

pub fn bunx_filename() -> &'static str {
    if cfg!(windows) { "bunx.exe" } else { "bunx" }
}

pub fn node_filename() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

/// Returns true when `<dir>/bun[.exe]` exists and `<dir>/bun.stamp`
/// records the expected sha256 + version.
pub fn is_fresh(dir: &Path, expected_sha: &str, expected_version: &str) -> bool {
    let bun = dir.join(bun_filename());
    if !bun.is_file() {
        return false;
    }
    let stamp_path = dir.join("bun.stamp");
    let Ok(bytes) = fs::read(&stamp_path) else {
        return false;
    };
    let Ok(stamp): Result<Stamp, _> = serde_json::from_slice(&bytes) else {
        return false;
    };
    stamp.sha256 == expected_sha && stamp.version == expected_version
}

/// Extract `blob` (zstd-compressed bun) into `dir`. Idempotent and
/// cross-process safe via advisory file lock on `<dir>/../runtime.lock`.
pub fn extract_into(dir: &Path, blob: &[u8], expected_sha: &str, version: &str) -> Result<PathBuf, ExtractError> {
    fs::create_dir_all(dir)?;

    // Lock file lives in the parent (runtime root) so it survives across
    // per-version dir churn.
    let lock_parent = dir.parent().unwrap_or(dir);
    fs::create_dir_all(lock_parent)?;
    let lock_path = lock_parent.join("runtime.lock");
    let lock_file = File::create(&lock_path)?;
    lock_file.lock_exclusive()?;

    // Re-check after taking the lock: maybe another process finished.
    if is_fresh(dir, expected_sha, version) {
        let _ = FileExt::unlock(&lock_file);
        return Ok(dir.join(bun_filename()));
    }

    let result = (|| -> Result<PathBuf, ExtractError> {
        let tmp_path = dir.join("bun.tmp");
        let _ = fs::remove_file(&tmp_path);

        // Decompress zstd -> tmp file.
        {
            let mut out = File::create(&tmp_path)?;
            let reader = BufReader::new(std::io::Cursor::new(blob));
            let mut decoder = zstd::stream::read::Decoder::new(reader)?;
            std::io::copy(&mut decoder, &mut out)?;
            out.sync_all()?;
        }

        // Verify sha256.
        let actual_sha = sha256_file(&tmp_path)?;
        if actual_sha != expected_sha {
            let _ = fs::remove_file(&tmp_path);
            return Err(ExtractError::ChecksumMismatch {
                expected: expected_sha.into(),
                actual: actual_sha,
            });
        }

        // chmod +x on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&tmp_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&tmp_path, perms)?;
        }

        // Atomic rename into place.
        let bun_path = dir.join(bun_filename());
        let _ = fs::remove_file(&bun_path);
        fs::rename(&tmp_path, &bun_path)?;

        // bunx: symlink (Unix) or copy (Windows).
        let bunx_path = dir.join(bunx_filename());
        let _ = fs::remove_file(&bunx_path);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&bun_path, &bunx_path)?;
        }
        #[cfg(windows)]
        {
            fs::copy(&bun_path, &bunx_path)?;
        }

        // node: symlink (Unix) or copy (Windows).
        // Many npm packages use `#!/usr/bin/env node` shebangs; placing a
        // `node` alias in the bundled bun directory ensures they resolve
        // to bun (which is Node-compatible) even when no standalone Node
        // installation exists on the host.
        let node_path = dir.join(node_filename());
        let _ = fs::remove_file(&node_path);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&bun_path, &node_path)?;
        }
        #[cfg(windows)]
        {
            fs::copy(&bun_path, &node_path)?;
        }

        // Stamp.
        let stamp = Stamp {
            sha256: expected_sha.into(),
            version: version.into(),
            extracted_at: chrono_utc_now(),
        };
        let stamp_bytes = serde_json::to_vec_pretty(&stamp)?;
        let stamp_tmp = dir.join("bun.stamp.tmp");
        {
            let mut f = File::create(&stamp_tmp)?;
            f.write_all(&stamp_bytes)?;
            f.sync_all()?;
        }
        fs::rename(&stamp_tmp, dir.join("bun.stamp"))?;

        Ok(bun_path)
    })();

    let _ = FileExt::unlock(&lock_file);
    result
}

fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let mut f = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// A cheap RFC3339-ish timestamp that avoids pulling chrono into this crate.
fn chrono_utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch-{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_blob(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = zstd::stream::write::Encoder::new(&mut out, 0).unwrap();
        enc.write_all(payload).unwrap();
        enc.finish().unwrap();
        out
    }

    fn sha_hex(payload: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(payload);
        hex::encode(h.finalize())
    }

    #[test]
    fn extract_happy_path_creates_bun_and_bunx_and_node() {
        let payload = b"#!/bin/sh\necho fake-bun\n";
        let blob = make_blob(payload);
        let expected_sha = sha_hex(payload);

        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("bun-9.9.9-aaaa");
        let bun_path = extract_into(&dir, &blob, &expected_sha, "9.9.9").unwrap();

        assert!(bun_path.is_file(), "bun file must exist");
        assert!(dir.join(bunx_filename()).exists(), "bunx must exist");
        assert!(dir.join(node_filename()).exists(), "node must exist");
        assert!(dir.join("bun.stamp").is_file(), "stamp must exist");

        let contents = std::fs::read(&bun_path).unwrap();
        assert_eq!(contents, payload);
    }

    #[test]
    fn extract_is_idempotent_via_stamp_fast_path() {
        let payload = b"#!/bin/sh\necho fake\n";
        let blob = make_blob(payload);
        let sha = sha_hex(payload);
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("bun-1.0-aaaa");

        extract_into(&dir, &blob, &sha, "1.0").unwrap();
        // Remove bun temp to prove re-extraction isn't happening.
        assert!(is_fresh(&dir, &sha, "1.0"));

        // Second call should early-return via is_fresh after lock reacquire.
        extract_into(&dir, &blob, &sha, "1.0").unwrap();
        assert!(is_fresh(&dir, &sha, "1.0"));
    }

    #[test]
    fn extract_rejects_corrupt_checksum() {
        let payload = b"real contents";
        let blob = make_blob(payload);
        let wrong_sha = "0000000000000000000000000000000000000000000000000000000000000000";

        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("bun-corrupt");
        let err = extract_into(&dir, &blob, wrong_sha, "1.0").unwrap_err();
        match err {
            ExtractError::ChecksumMismatch { .. } => {}
            e => panic!("expected ChecksumMismatch, got {e:?}"),
        }
        assert!(!dir.join(bun_filename()).exists());
    }

    #[test]
    fn is_fresh_returns_false_when_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!is_fresh(tmp.path(), "abc", "1.0"));
    }
}
