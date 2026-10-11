//! Hash-verified, atomic component publication under an inter-process lock.
use crate::notices::PublicationFile;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
struct Stamp {
    sha256: String,
    version: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("component I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("component checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("component stamp is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub(crate) fn is_fresh(
    dir: &Path,
    name: &str,
    digest: &str,
    version: &str,
    bun_alias: bool,
    notices: &[PublicationFile],
) -> bool {
    let Ok(bytes) = fs::read(dir.join("component.stamp")) else {
        return false;
    };
    let Ok(stamp) = serde_json::from_slice::<Stamp>(&bytes) else {
        return false;
    };
    stamp.sha256 == digest && stamp.version == version
        && sha256_file(&dir.join(name)).is_ok_and(|actual| actual == digest)
        && executable_permissions(&dir.join(name))
        && (!bun_alias || alias_is_fresh(dir, name, digest))
        // A cache from the retired Bun-as-Node design must never be reusable.
        && (!bun_alias || !dir.join(node_filename()).exists())
        && notices.iter().all(|notice| sha256_file(&dir.join(notice.filename)).is_ok_and(|actual| actual == hex::encode(Sha256::digest(notice.contents))))
}

pub(crate) fn extract_into(
    dir: &Path,
    name: &str,
    blob: &[u8],
    digest: &str,
    version: &str,
    bun_alias: bool,
    files: &[PublicationFile<'_>],
) -> Result<PathBuf, ExtractError> {
    extract_with(dir, name, digest, version, bun_alias, files, |output| {
        let mut decoder =
            zstd::stream::read::Decoder::new(BufReader::new(std::io::Cursor::new(blob)))?;
        std::io::copy(&mut decoder, output)?;
        Ok(())
    })
}

pub(crate) fn extract_file_into(
    dir: &Path,
    name: &str,
    input: &Path,
    digest: &str,
    version: &str,
    bun_alias: bool,
    files: &[PublicationFile<'_>],
) -> Result<PathBuf, ExtractError> {
    extract_with(dir, name, digest, version, bun_alias, files, |output| {
        std::io::copy(&mut BufReader::new(File::open(input)?), output)?;
        Ok(())
    })
}

fn extract_with(
    dir: &Path,
    name: &str,
    digest: &str,
    version: &str,
    bun_alias: bool,
    notices: &[PublicationFile<'_>],
    decode: impl FnOnce(&mut File) -> Result<(), std::io::Error>,
) -> Result<PathBuf, ExtractError> {
    fs::create_dir_all(dir)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("component.lock"))?;
    lock.lock_exclusive()?;
    if is_fresh(dir, name, digest, version, bun_alias, notices) {
        FileExt::unlock(&lock)?;
        return Ok(dir.join(name));
    }
    let binary_fresh = sha256_file(&dir.join(name)).is_ok_and(|actual| actual == digest);
    let staging = dir.join("component.tmp");
    let stamp_staging = dir.join("component.stamp.tmp");
    let result = (|| {
        if !binary_fresh {
            remove_if_present(&staging)?;
            let mut output = File::options()
                .write(true)
                .create_new(true)
                .open(&staging)?;
            decode(&mut output)?;
            output.sync_all()?;
            drop(output);
            let actual = sha256_file(&staging)?;
            if actual != digest {
                return Err(ExtractError::ChecksumMismatch {
                    expected: digest.into(),
                    actual,
                });
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&staging, fs::Permissions::from_mode(0o555))?;
            }
        }
        #[cfg(unix)]
        if binary_fresh && !executable_permissions(&dir.join(name)) {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o555))?;
        }
        remove_if_present(&dir.join("component.stamp"))?;
        let executable = dir.join(name);
        // Windows cannot atomically replace an open executable. A corrupt cache
        // is not an admitted running version, so remove it only after validation.
        if !binary_fresh {
            #[cfg(windows)]
            remove_if_present(&executable)?;
            fs::rename(&staging, &executable)?;
        }
        if bun_alias {
            remove_if_present(&dir.join(node_filename()))?;
        }
        if bun_alias && !alias_is_fresh(dir, name, digest) {
            let alias = dir.join(bunx_filename());
            remove_if_present(&alias)?;
            remove_if_present(&dir.join(node_filename()))?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(name, &alias)?;
            #[cfg(windows)]
            fs::copy(&executable, &alias)?;
        }
        for notice in notices {
            publish_notice(dir, notice)?;
        }
        remove_if_present(&stamp_staging)?;
        let mut stamp = File::options()
            .write(true)
            .create_new(true)
            .open(&stamp_staging)?;
        stamp.write_all(&serde_json::to_vec(&Stamp {
            sha256: digest.into(),
            version: version.into(),
        })?)?;
        stamp.sync_all()?;
        drop(stamp);
        fs::rename(&stamp_staging, dir.join("component.stamp"))?;
        Ok(executable)
    })();
    if result.is_err() {
        let _ = fs::remove_file(staging);
        let _ = fs::remove_file(stamp_staging);
        for notice in notices {
            let _ = fs::remove_file(dir.join(format!("{}.tmp", notice.filename)));
        }
    }
    FileExt::unlock(&lock)?;
    result
}

fn executable_permissions(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn alias_is_fresh(dir: &Path, name: &str, _digest: &str) -> bool {
    #[cfg(unix)]
    {
        fs::read_link(dir.join(bunx_filename())).is_ok_and(|target| target == Path::new(name))
    }
    #[cfg(windows)]
    {
        let _ = name;
        sha256_file(&dir.join(bunx_filename())).is_ok_and(|actual| actual == _digest)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (dir, name, _digest);
        false
    }
}

fn publish_notice(dir: &Path, notice: &PublicationFile) -> Result<(), std::io::Error> {
    let staging = dir.join(format!("{}.tmp", notice.filename));
    remove_if_present(&staging)?;
    let mut file = File::options()
        .create_new(true)
        .write(true)
        .open(&staging)?;
    file.write_all(notice.contents)?;
    file.sync_all()?;
    drop(file);
    fs::rename(staging, dir.join(notice.filename))
}

fn remove_if_present(path: &Path) -> Result<(), std::io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn bunx_filename() -> &'static str {
    if cfg!(windows) { "bunx.exe" } else { "bunx" }
}
fn node_filename() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn blob(bytes: &[u8]) -> Vec<u8> {
        zstd::stream::encode_all(bytes, 0).unwrap()
    }
    fn digest(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }
    #[test]
    fn corrupt_executable_and_alias_are_repaired_without_trusting_stamp() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let bytes = b"approved runtime";
        let sha = digest(bytes);
        extract_into(dir, "bun", &blob(bytes), &sha, "1.3.13", true, &[]).unwrap();
        assert!(!dir.join(node_filename()).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.join("bun"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(dir.join("bun"), b"corrupt").unwrap();
        assert!(!is_fresh(dir, "bun", &sha, "1.3.13", true, &[]));
        extract_into(dir, "bun", &blob(bytes), &sha, "1.3.13", true, &[]).unwrap();
        assert!(is_fresh(dir, "bun", &sha, "1.3.13", true, &[]));
        fs::remove_file(dir.join(bunx_filename())).unwrap();
        extract_into(dir, "bun", &blob(bytes), &sha, "1.3.13", true, &[]).unwrap();
        assert_eq!(fs::read(dir.join(bunx_filename())).unwrap(), bytes);
    }
    #[test]
    fn invalid_payload_preserves_previous_valid_version() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = b"approved";
        let sha = digest(bytes);
        extract_into(tmp.path(), "officecli", &blob(bytes), &sha, "1", false, &[]).unwrap();
        assert!(
            extract_into(
                tmp.path(),
                "officecli",
                &blob(b"wrong"),
                &digest(b"approved new generation"),
                "2",
                false,
                &[]
            )
            .is_err()
        );
        assert!(is_fresh(tmp.path(), "officecli", &sha, "1", false, &[]));
        assert!(!tmp.path().join("component.tmp").exists());
    }

    #[test]
    fn missing_or_changed_notices_are_repaired_without_republishing_valid_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = b"approved runtime";
        let sha = digest(bytes);
        let notices = [PublicationFile {
            filename: "LICENSE.txt",
            contents: b"approved license text",
        }];
        let path = extract_into(
            tmp.path(),
            "node",
            &blob(bytes),
            &sha,
            "22",
            false,
            &notices,
        )
        .unwrap();
        let original_modified = fs::metadata(&path).unwrap().modified().unwrap();
        #[cfg(unix)]
        let original_inode = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&path).unwrap().ino()
        };
        for replacement in [None, Some(b"corrupt notice".as_slice())] {
            fs::remove_file(tmp.path().join("LICENSE.txt")).unwrap();
            if let Some(bytes) = replacement {
                fs::write(tmp.path().join("LICENSE.txt"), bytes).unwrap();
            }
            assert!(!is_fresh(tmp.path(), "node", &sha, "22", false, &notices));
            extract_into(
                tmp.path(),
                "node",
                b"never decoded for an already verified binary",
                &sha,
                "22",
                false,
                &notices,
            )
            .unwrap();
            assert!(is_fresh(tmp.path(), "node", &sha, "22", false, &notices));
            assert_eq!(
                fs::metadata(&path).unwrap().modified().unwrap(),
                original_modified
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(fs::metadata(&path).unwrap().ino(), original_inode);
            }
        }
    }
    #[test]
    fn concurrent_publication_has_one_complete_verified_result() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let bytes = b"approved";
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    extract_into(&dir, "node", &blob(bytes), &digest(bytes), "22", false, &[])
                        .unwrap()
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), dir.join("node"));
        }
        assert!(is_fresh(&dir, "node", &digest(bytes), "22", false, &[]));
    }
}
