//! Pre-boot directory configuration: persist the user's chosen working
//! directory so it survives a restart and is readable *before* the in-process
//! backend resolves `work_dir`.
//!
//! Why a file (not the database): under Tauri the backend is linked in-process,
//! and `work_dir` is resolved early in `bootstrap::init_environment` — before
//! the SQLite pool is opened. A config written by the running backend therefore
//! has to land somewhere that the *next* boot can read before any service
//! exists. A small JSON file under `data_dir` fits: `data_dir` is fixed for the
//! lifetime of the install (it does not change when `work_dir` does) and is
//! resolved at the very start of boot.
//!
//! Unlike [`crate::factory_reset`]'s one-shot reset request,
//! this config is *persistent*: it is kept until the user changes the directory
//! again, so every subsequent boot honors the choice.
//!
//! Flow for a changed work root:
//!   1. `POST /api/system/work-dir` writes a bound one-shot reset request.
//!   2. Frontend relaunches the desktop shell.
//!   3. Locked pre-boot reset preparation creates a fresh generation and calls
//!      [`set_work_dir`] only after the target is accepted.
//!   4. Later boots call [`checked_persisted_work_dir`] and reuse that path.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Config file under the data dir holding pre-boot directory overrides.
pub const DIR_CONFIG_FILE: &str = "dir-config.json";
const MAX_DIR_CONFIG_BYTES: u64 = 64 * 1024;

/// Persisted pre-boot directory overrides. Optional fields so an absent value
/// means "fall back to the normal resolution"; the struct leaves room to add
/// more pre-boot dirs later without breaking older files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirConfig {
    /// User-chosen conversation workspace root. `None` ⇒ no override.
    #[serde(default)]
    pub work_dir: Option<PathBuf>,
}

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_CONFIG_FILE)
}

/// Compatibility reader for callers that intentionally tolerate missing or
/// malformed config by using [`DirConfig::default`].
///
/// Startup must use [`checked_persisted_work_dir`] instead so an existing
/// untrusted config cannot silently change the resolved work root.
pub fn read(data_dir: &Path) -> DirConfig {
    match std::fs::read(config_path(data_dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => DirConfig::default(),
    }
}

/// The persisted working directory, if any and usable. Filters out empty and
/// non-absolute paths (a relative override is meaningless this early in boot).
pub fn persisted_work_dir(data_dir: &Path) -> Option<PathBuf> {
    let work_dir = read(data_dir).work_dir?;
    if work_dir.as_os_str().is_empty() || !work_dir.is_absolute() {
        return None;
    }
    Some(work_dir)
}

/// Strictly read the persisted working directory for startup.
///
/// Unlike [`read`] and [`persisted_work_dir`], an existing but malformed or
/// unsafe config fails closed. Startup must not silently fall back to another
/// work root when the persisted binding cannot be trusted.
pub fn checked_persisted_work_dir(
    data_dir: &Path,
) -> Result<Option<PathBuf>, AppError> {
    let path = config_path(data_dir);
    let bytes = match read_bounded_regular_file(&path, MAX_DIR_CONFIG_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "read dir-config safely {}: {error}",
                path.display()
            )));
        }
    };
    let config: DirConfig = serde_json::from_slice(&bytes).map_err(|error| {
        AppError::Internal(format!(
            "parse dir-config {}: {error}",
            path.display()
        ))
    })?;
    let Some(work_dir) = config.work_dir else {
        return Ok(None);
    };
    if work_dir.as_os_str().is_empty()
        || !work_dir.is_absolute()
        || crate::workspace_path_has_edge_whitespace_segment(&work_dir)
    {
        return Err(AppError::Internal(format!(
            "dir-config {} contains an unsafe, empty, or non-absolute work_dir: {}",
            path.display(),
            work_dir.display()
        )));
    }
    Ok(Some(work_dir))
}

fn read_bounded_regular_file(
    path: &Path,
    max_bytes: u64,
) -> std::io::Result<Vec<u8>> {
    let path_metadata = std::fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&path_metadata) || !path_metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("dir-config is not a regular file: {}", path.display()),
        ));
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("dir-config is not a regular file: {}", path.display()),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "dir-config exceeds the {max_bytes}-byte limit: {}",
                path.display()
            ),
        ));
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    use std::io::Read;
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "dir-config grew beyond the {max_bytes}-byte limit: {}",
                path.display()
            ),
        ));
    }
    Ok(bytes)
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// Persist `work_dir` as the pre-boot working-directory override. Read-modify-
/// write so future fields on [`DirConfig`] are preserved.
pub fn set_work_dir(data_dir: &Path, work_dir: &Path) -> Result<(), AppError> {
    let mut config = read(data_dir);
    config.work_dir = Some(work_dir.to_path_buf());
    let json = serde_json::to_vec_pretty(&config)
        .map_err(|e| AppError::Internal(format!("serialize dir-config: {e}")))?;
    write_atomic_replace(&config_path(data_dir), &json)
        .map_err(|e| AppError::Internal(format!("write dir-config: {e}")))
}

/// Replace only a bounded, regular config whose JSON is malformed.
///
/// Callers must first prove the authoritative dataset/work-root binding (or a
/// fresh post-reset bootstrap). Valid-but-unsafe paths, symlinks/reparse
/// points, oversized files, and I/O failures are never repaired implicitly.
pub fn replace_malformed_work_dir_after_lifecycle_proof(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<bool, AppError> {
    let path = config_path(data_dir);
    let bytes = match read_bounded_regular_file(&path, MAX_DIR_CONFIG_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "refusing to repair unsafe dir-config {}: {error}",
                path.display()
            )));
        }
    };
    if serde_json::from_slice::<DirConfig>(&bytes).is_ok() {
        return Ok(false);
    }
    let json = serde_json::to_vec_pretty(&DirConfig {
        work_dir: Some(work_dir.to_path_buf()),
    })
    .map_err(|error| {
        AppError::Internal(format!(
            "serialize lifecycle-proven dir-config repair: {error}"
        ))
    })?;
    write_atomic_replace(&path, &json).map_err(|error| {
        AppError::Internal(format!(
            "atomically repair malformed dir-config {}: {error}",
            path.display()
        ))
    })?;
    Ok(true)
}

/// Return whether startup may temporarily ignore the config while it obtains
/// an authoritative lifecycle proof. Unsafe file types and oversized/I/O
/// failures remain errors; only malformed JSON is repairable.
pub fn repairable_malformed_work_dir_exists(
    data_dir: &Path,
) -> Result<bool, AppError> {
    let path = config_path(data_dir);
    let bytes = match read_bounded_regular_file(&path, MAX_DIR_CONFIG_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => {
            return Err(AppError::Internal(format!(
                "inspect dir-config repair eligibility {}: {error}",
                path.display()
            )));
        }
    };
    Ok(serde_json::from_slice::<DirConfig>(&bytes).is_err())
}

fn write_atomic_replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("dir-config"),
        uuid::Uuid::now_v7()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temp, path)?;
        sync_parent_directory(parent)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(source, target)
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Atomically publish a minimal work-dir config only when no active config
/// exists.
///
/// This is intentionally narrower than [`set_work_dir`]. It is used by the
/// one-time legacy-v1 reset repair and must never overwrite a config that
/// appeared concurrently. Publication is atomic and no-clobber without
/// exposing a partially written JSON file.
pub fn install_work_dir_if_absent(
    data_dir: &Path,
    work_dir: &Path,
) -> Result<(), AppError> {
    let path = config_path(data_dir);
    let json = serde_json::to_vec_pretty(&DirConfig {
        work_dir: Some(work_dir.to_path_buf()),
    })
    .map_err(|error| {
        AppError::Internal(format!(
            "serialize recovered dir-config: {error}"
        ))
    })?;
    let temp = data_dir.join(format!(
        ".{DIR_CONFIG_FILE}.repair-{}",
        uuid::Uuid::now_v7()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        use std::io::Write;
        file.write_all(&json)?;
        file.sync_all()?;
        drop(file);
        publish_new_file(&temp, &path)?;
        sync_parent_directory(data_dir)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|error| {
        AppError::Internal(format!(
            "atomically install recovered dir-config {}: {error}",
            path.display()
        ))
    })
}

#[cfg(target_os = "macos")]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source path contains a NUL byte",
            )
        },
    )?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "target path contains a NUL byte",
            )
        },
    )?;
    if unsafe {
        libc::renamex_np(
            source.as_ptr(),
            target.as_ptr(),
            libc::RENAME_EXCL,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source path contains a NUL byte",
            )
        },
    )?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(
        |_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "target path contains a NUL byte",
            )
        },
    )?;
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos"))
))]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::hard_link(source, target)?;
    std::fs::remove_file(source)
}

#[cfg(not(any(unix, windows)))]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::hard_link(source, target)?;
    std::fs::remove_file(source)
}

#[cfg(windows)]
fn publish_new_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } != 0
    {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(80 | 183)) {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            error,
        ))
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    OpenOptions::new().read(true).open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timestamp::now_ms;

    fn temp_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nomifun-dircfg-{tag}-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn set_then_read_roundtrips_the_work_dir() {
        let data_dir = temp_data_dir("roundtrip");
        let work_dir = data_dir.join("my-workspace"); // absolute (under temp dir)

        set_work_dir(&data_dir, &work_dir).unwrap();

        assert_eq!(read(&data_dir).work_dir.as_deref(), Some(work_dir.as_path()));
        assert_eq!(persisted_work_dir(&data_dir).as_deref(), Some(work_dir.as_path()));
        assert_eq!(
            checked_persisted_work_dir(&data_dir)
                .unwrap()
                .as_deref(),
            Some(work_dir.as_path())
        );

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn missing_file_is_default_and_no_override() {
        let data_dir = temp_data_dir("missing");
        assert!(read(&data_dir).work_dir.is_none());
        assert!(persisted_work_dir(&data_dir).is_none());
        assert!(checked_persisted_work_dir(&data_dir).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn malformed_file_falls_back_to_default() {
        let data_dir = temp_data_dir("malformed");
        std::fs::write(config_path(&data_dir), b"not json at all").unwrap();
        assert!(read(&data_dir).work_dir.is_none());
        assert!(persisted_work_dir(&data_dir).is_none());
        let error = checked_persisted_work_dir(&data_dir).unwrap_err();
        assert!(error.to_string().contains("parse dir-config"));
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn lifecycle_proof_can_atomically_repair_only_malformed_json() {
        let data_dir = temp_data_dir("lifecycle-repair");
        let work_dir = data_dir.join("proven-work");
        std::fs::write(config_path(&data_dir), b"{\"work_dir\":").unwrap();

        assert!(
            replace_malformed_work_dir_after_lifecycle_proof(
                &data_dir,
                &work_dir,
            )
            .unwrap()
        );
        assert_eq!(
            checked_persisted_work_dir(&data_dir)
                .unwrap()
                .as_deref(),
            Some(work_dir.as_path())
        );

        assert!(
            !replace_malformed_work_dir_after_lifecycle_proof(
                &data_dir,
                &work_dir,
            )
            .unwrap(),
            "a valid config is never rewritten by the narrow repair"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn checked_reader_rejects_oversized_file() {
        let data_dir = temp_data_dir("oversized");
        std::fs::write(
            config_path(&data_dir),
            vec![b'x'; MAX_DIR_CONFIG_BYTES as usize + 1],
        )
        .unwrap();

        let error = checked_persisted_work_dir(&data_dir).unwrap_err();
        assert!(error.to_string().contains("65536-byte limit"));

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[cfg(unix)]
    #[test]
    fn checked_reader_rejects_symlink() {
        use std::os::unix::fs::symlink;

        let data_dir = temp_data_dir("symlink");
        let work_dir = data_dir.join("chosen-ws");
        let target = data_dir.join("target-dir-config.json");
        std::fs::write(
            &target,
            serde_json::to_vec(&DirConfig {
                work_dir: Some(work_dir),
            })
            .unwrap(),
        )
        .unwrap();
        symlink(&target, config_path(&data_dir)).unwrap();

        let error = checked_persisted_work_dir(&data_dir).unwrap_err();
        assert!(error.to_string().contains("read dir-config safely"));

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn persisted_work_dir_rejects_relative_path() {
        let data_dir = temp_data_dir("relative");
        std::fs::write(config_path(&data_dir), br#"{"work_dir":"relative/ws"}"#).unwrap();
        // read() surfaces the raw stored value, persisted_work_dir() filters it out.
        assert_eq!(read(&data_dir).work_dir, Some(PathBuf::from("relative/ws")));
        assert!(persisted_work_dir(&data_dir).is_none());
        let error = checked_persisted_work_dir(&data_dir).unwrap_err();
        assert!(error.to_string().contains("non-absolute work_dir"));
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn persisted_work_dir_rejects_empty_path() {
        let data_dir = temp_data_dir("empty");
        std::fs::write(config_path(&data_dir), br#"{"work_dir":""}"#).unwrap();
        assert!(persisted_work_dir(&data_dir).is_none());
        let error = checked_persisted_work_dir(&data_dir).unwrap_err();
        assert!(error.to_string().contains("unsafe, empty"));
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn checked_reader_rejects_edge_whitespace_path_segment() {
        let data_dir = temp_data_dir("edge-whitespace");
        let unsafe_work_dir = data_dir.join(" trailing ");
        std::fs::write(
            config_path(&data_dir),
            serde_json::to_vec(&DirConfig {
                work_dir: Some(unsafe_work_dir),
            })
            .unwrap(),
        )
        .unwrap();

        let error = checked_persisted_work_dir(&data_dir).unwrap_err();
        assert!(error.to_string().contains("unsafe"));
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn set_work_dir_overwrites_previous_value() {
        let data_dir = temp_data_dir("overwrite");
        let first = data_dir.join("ws-a");
        let second = data_dir.join("ws-b");

        set_work_dir(&data_dir, &first).unwrap();
        set_work_dir(&data_dir, &second).unwrap();

        assert_eq!(read(&data_dir).work_dir.as_deref(), Some(second.as_path()));
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn recovery_install_is_atomic_and_never_overwrites_existing_config() {
        let data_dir = temp_data_dir("recover-no-clobber");
        let first = data_dir.join("ws-a");
        let second = data_dir.join("ws-b");

        install_work_dir_if_absent(&data_dir, &first).unwrap();
        let error = install_work_dir_if_absent(&data_dir, &second).unwrap_err();

        assert_eq!(persisted_work_dir(&data_dir).as_deref(), Some(first.as_path()));
        assert!(error.to_string().contains("atomically install"));
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
