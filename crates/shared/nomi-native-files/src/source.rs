use std::{fs::File, io::Read, path::Path};
use sha2::{Digest, Sha256};
use crate::{MAX_SOURCE_BYTES, NativeFileError, NativeSource};

pub fn acquire_source_path(path: &Path, max_bytes: usize) -> Result<Option<NativeSource>, NativeFileError> {
    acquire_source_with_hooks(path, max_bytes, || {}, || {})
}

pub fn acquire_source_with_hooks(path: &Path, max_bytes: usize, before_open: impl FnOnce(), after_read: impl FnOnce()) -> Result<Option<NativeSource>, NativeFileError> {
    crate::validate_path(path)?;
    if max_bytes > MAX_SOURCE_BYTES { return Err(NativeFileError::InvalidInput("source byte budget exceeds 8 MiB".into())); }
    let canonical = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let (_, absent) = crate::resolve_missing(path, 4096, |path| std::fs::canonicalize(path).map_err(|error| NativeFileError::io("resolving native source ancestor", error)))?;
            if absent { return Ok(None); }
            return Err(NativeFileError::io("resolving native source", error));
        }
        Err(error) => return Err(NativeFileError::io("resolving native source", error)),
    };
    let open = || {
        #[cfg(windows)]
        { crate::windows::open_source(&canonical) }
        #[cfg(not(windows))]
        {
        let metadata = std::fs::symlink_metadata(&canonical).map_err(|error| NativeFileError::io("inspecting native source", error))?;
        if !metadata.is_file() || metadata.len() > max_bytes as u64 {
            return Err(NativeFileError::InvalidInput("source is not a bounded regular file".into()));
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        }
        options.open(&canonical).map_err(|error| NativeFileError::io("opening native source", error))
        }
    };
    before_open();
    let canonical_path = crate::path_string(&canonical)?;
    acquire_source_with_adapter(max_bytes, &mut 0, open, after_read,
        || std::fs::canonicalize(path).map(|current| current == canonical).map_err(|error| NativeFileError::io("verifying native source path", error)),
        |error| error).map(|(bytes, sha256)| Some(NativeSource { canonical_path, bytes, sha256 }))
}

/// Single bounded byte acquisition kernel. Authority-aware owners supply their
/// existing no-follow open and path-verification adapters; this code owns the
/// fd/content/identity/mtime checks and charges partial reads even on failure.
pub fn acquire_source_with_adapter<E>(
    max_bytes: usize, charged_bytes: &mut usize,
    open: impl Fn() -> Result<File, E>, after_read: impl FnOnce(),
    verify_path: impl FnOnce() -> Result<bool, E>, map_native: impl Fn(NativeFileError) -> E,
) -> Result<(Vec<u8>, String), E> {
    if max_bytes > MAX_SOURCE_BYTES { return Err(map_native(NativeFileError::InvalidInput("source byte budget exceeds 8 MiB".into()))); }
    let io = |error| map_native(NativeFileError::io("reading native source", error));
    let file = open()?;
    let opened = file.metadata().map_err(io)?;
    if !opened.is_file() || opened.len() > max_bytes as u64 {
        return Err(map_native(NativeFileError::InvalidInput("source is not a bounded regular file".into())));
    }
    let identity = same_file::Handle::from_file(file.try_clone().map_err(io)?).map_err(io)?;
    let mut bytes = Vec::with_capacity((opened.len() as usize).min(max_bytes));
    let mut bounded = file.take((max_bytes + 1) as u64);
    let read = bounded.read_to_end(&mut bytes);
    *charged_bytes = charged_bytes.saturating_add(bytes.len());
    read.map_err(io)?;
    after_read();
    let after = bounded.get_ref().metadata().map_err(io)?;
    let current = open().map_err(|_| map_native(NativeFileError::Changed("FILE_CHANGED_DURING_READ: named source can no longer be verified".into())))?;
    let current_identity = same_file::Handle::from_file(current).map_err(io)?;
    if bytes.len() > max_bytes || bytes.len() as u64 != opened.len() || opened.len() != after.len()
        || opened.modified().ok() != after.modified().ok() || identity != current_identity || !verify_path()? {
        return Err(map_native(NativeFileError::Changed("FILE_CHANGED_DURING_READ: discard this read and restart".into())));
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok((bytes, sha256))
}
