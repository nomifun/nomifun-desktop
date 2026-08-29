use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use nomifun_agent_contracts::{FRESH_V4_PARENT_MARKER_FILE, FRESH_V4_READY_MARKER_FILE};

use crate::{
    FreshV4AccessAudit, FreshV4AccessKind, FreshV4RootError,
    coordinator::{FRESH_V4_DATABASE_FILE, FRESH_V4_INITIALIZING_MARKER_FILE},
};

const MAX_MARKER_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryKind {
    Missing,
    File,
    Directory,
    LinkOrReparse,
    Other,
}

#[derive(Clone, Debug)]
pub(crate) struct RootPaths {
    pub parent: PathBuf,
    pub canonical_root: PathBuf,
    pub canonical_basename: String,
    pub parent_marker: PathBuf,
    pub initializing_marker: PathBuf,
    pub ready_marker: PathBuf,
    pub database: PathBuf,
}

pub(crate) fn normalize_root(
    requested: &Path,
    protected_roots: &[PathBuf],
    audit: &dyn FreshV4AccessAudit,
) -> Result<RootPaths, FreshV4RootError> {
    if requested.as_os_str().is_empty() {
        return Err(FreshV4RootError::InvalidRoot(
            "canonical data root must not be empty".into(),
        ));
    }
    let absolute = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| FreshV4RootError::io("resolve current directory", ".", error))?
            .join(requested)
    };
    let basename = absolute
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            FreshV4RootError::InvalidRoot(
                "canonical data root must have one UTF-8 relative basename".into(),
            )
        })?
        .to_owned();
    let parent = absolute.parent().ok_or_else(|| {
        FreshV4RootError::InvalidRoot(
            "canonical data root must not be a filesystem root".into(),
        )
    })?;
    match entry_kind(parent, audit)? {
        EntryKind::Directory => {}
        EntryKind::Missing => {
            return Err(FreshV4RootError::InvalidRoot(format!(
                "canonical data-root parent does not exist: {}",
                parent.display()
            )));
        }
        _ => {
            return Err(FreshV4RootError::InvalidRoot(format!(
                "canonical data-root parent must be a real directory: {}",
                parent.display()
            )));
        }
    }
    audit.record(FreshV4AccessKind::Metadata, parent)?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|error| FreshV4RootError::io("canonicalize parent", parent, error))?;
    let canonical_root = parent.join(&basename);
    let canonical_root_identity =
        canonicalize_if_present(&canonical_root).unwrap_or_else(|| canonical_root.clone());

    let marker_probe = nomifun_agent_contracts::FreshV4ParentOperationMarker {
        operation_id: "path-validation".into(),
        operation_kind: nomifun_agent_contracts::FreshV4OperationKind::Fresh,
        canonical_normalized_relative_basename: basename.clone(),
        cutover_archive_sibling_relative_basename: None,
        target_data_generation: nomifun_agent_contracts::FRESH_V4_DATA_GENERATION,
        canonical_schema_manifest_digest:
            "0000000000000000000000000000000000000000000000000000000000000000"
                .into(),
    };
    marker_probe
        .validate()
        .map_err(FreshV4RootError::InvalidRoot)?;

    if home_directory()
        .as_deref()
        .and_then(canonicalize_if_present)
        .is_some_and(|home| home == canonical_root_identity)
    {
        return Err(FreshV4RootError::InvalidRoot(
            "canonical data root must not be the user home directory".into(),
        ));
    }
    for protected in protected_roots {
        let protected = absolute_normalized(protected)?;
        if protected == canonical_root_identity {
            return Err(FreshV4RootError::InvalidRoot(format!(
                "canonical data root must not equal protected workspace/repository root {}",
                protected.display()
            )));
        }
    }

    Ok(RootPaths {
        parent_marker: parent.join(FRESH_V4_PARENT_MARKER_FILE),
        initializing_marker: canonical_root.join(FRESH_V4_INITIALIZING_MARKER_FILE),
        ready_marker: canonical_root.join(FRESH_V4_READY_MARKER_FILE),
        database: canonical_root.join(FRESH_V4_DATABASE_FILE),
        parent,
        canonical_root,
        canonical_basename: basename,
    })
}

fn absolute_normalized(path: &Path) -> Result<PathBuf, FreshV4RootError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| FreshV4RootError::io("resolve current directory", ".", error))?
            .join(path)
    };
    Ok(canonicalize_if_present(&absolute).unwrap_or(absolute))
}

fn canonicalize_if_present(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

pub(crate) fn entry_kind(
    path: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<EntryKind, FreshV4RootError> {
    audit.record(FreshV4AccessKind::Metadata, path)?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EntryKind::Missing);
        }
        Err(error) => {
            return Err(FreshV4RootError::io(
                "inspect filesystem entry",
                path,
                error,
            ));
        }
    };
    if metadata_is_link_or_reparse(&metadata) {
        Ok(EntryKind::LinkOrReparse)
    } else if metadata.is_file() {
        Ok(EntryKind::File)
    } else if metadata.is_dir() {
        Ok(EntryKind::Directory)
    } else {
        Ok(EntryKind::Other)
    }
}

pub(crate) fn require_real_directory(
    path: &Path,
    label: &str,
    audit: &dyn FreshV4AccessAudit,
) -> Result<(), FreshV4RootError> {
    match entry_kind(path, audit)? {
        EntryKind::Directory => Ok(()),
        kind => Err(FreshV4RootError::State(format!(
            "{label} must be a real directory, found {kind:?}: {}",
            path.display()
        ))),
    }
}

pub(crate) fn same_filesystem(
    parent: &Path,
    root: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<bool, FreshV4RootError> {
    audit.record(FreshV4AccessKind::Metadata, parent)?;
    audit.record(FreshV4AccessKind::Metadata, root)?;
    let parent_metadata = std::fs::metadata(parent)
        .map_err(|error| FreshV4RootError::io("inspect parent filesystem", parent, error))?;
    let root_metadata = std::fs::metadata(root)
        .map_err(|error| FreshV4RootError::io("inspect root filesystem", root, error))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(parent_metadata.dev() == root_metadata.dev())
    }
    #[cfg(windows)]
    {
        // The target is an exact sibling under `parent`; a different Windows
        // volume would require a reparse/mount-point root, which preflight
        // rejects before this check.
        let _ = (parent_metadata, root_metadata);
        Ok(true)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (parent_metadata, root_metadata);
        Ok(true)
    }
}

pub(crate) fn create_directory(
    path: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<(), FreshV4RootError> {
    audit.record(FreshV4AccessKind::Write, path)?;
    std::fs::create_dir(path)
        .map_err(|error| FreshV4RootError::io("create canonical root", path, error))?;
    sync_parent(path)
}

pub(crate) fn remove_empty_directory(
    path: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<(), FreshV4RootError> {
    audit.record(FreshV4AccessKind::Remove, path)?;
    std::fs::remove_dir(path)
        .map_err(|error| FreshV4RootError::io("remove empty incomplete root", path, error))?;
    sync_parent(path)
}

pub(crate) fn rename_directory(
    source: &Path,
    target: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<(), FreshV4RootError> {
    if source.parent() != target.parent() {
        return Err(FreshV4RootError::InvalidRoot(format!(
            "whole-root cutover rename must stay within one parent: {} -> {}",
            source.display(),
            target.display()
        )));
    }
    audit.record(FreshV4AccessKind::RenameSource, source)?;
    audit.record(FreshV4AccessKind::RenameTarget, target)?;
    std::fs::rename(source, target)
        .map_err(|error| FreshV4RootError::io("rename whole canonical root", source, error))?;
    sync_parent(source)
}

pub(crate) fn read_bounded(
    path: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<Vec<u8>, FreshV4RootError> {
    audit.record(FreshV4AccessKind::Read, path)?;
    let file = File::open(path)
        .map_err(|error| FreshV4RootError::io("open marker", path, error))?;
    let mut bytes = Vec::new();
    file.take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| FreshV4RootError::io("read marker", path, error))?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err(FreshV4RootError::State(format!(
            "marker exceeds {MAX_MARKER_BYTES} bytes: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

pub(crate) fn open_regular_read_write(
    path: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<File, FreshV4RootError> {
    if entry_kind(path, audit)? != EntryKind::File {
        return Err(FreshV4RootError::State(format!(
            "marker must be a regular file: {}",
            path.display()
        )));
    }
    audit.record(FreshV4AccessKind::Read, path)?;
    let mut options = shared_marker_open_options();
    options.read(true).write(true);
    options
        .open(path)
        .map_err(|error| FreshV4RootError::io("open immutable marker", path, error))
}

pub(crate) fn read_bounded_file(
    file: &mut File,
    path: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<Vec<u8>, FreshV4RootError> {
    audit.record(FreshV4AccessKind::Read, path)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| FreshV4RootError::io("seek immutable marker", path, error))?;
    let mut bytes = Vec::new();
    file.take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| FreshV4RootError::io("read immutable marker", path, error))?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err(FreshV4RootError::State(format!(
            "marker exceeds {MAX_MARKER_BYTES} bytes: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

pub(crate) fn write_create_new_durable(
    path: &Path,
    bytes: &[u8],
    audit: &dyn FreshV4AccessAudit,
) -> Result<File, FreshV4RootError> {
    audit.record(FreshV4AccessKind::Write, path)?;
    let mut options = shared_marker_open_options();
    let mut file = options
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| FreshV4RootError::io("create immutable marker", path, error))?;
    let durable = file
        .write_all(bytes)
        .map_err(|error| FreshV4RootError::io("write immutable marker", path, error))
        .and_then(|()| {
            file.sync_all()
                .map_err(|error| FreshV4RootError::io("sync immutable marker", path, error))
        })
        .and_then(|()| sync_parent(path));
    if let Err(error) = durable {
        drop(file);
        let _ = std::fs::remove_file(path);
        let _ = sync_parent(path);
        return Err(error);
    }
    Ok(file)
}

fn shared_marker_open_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        const FILE_SHARE_DELETE: u32 = 0x0000_0004;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    }
    options
}

pub(crate) fn write_atomic_durable(
    path: &Path,
    bytes: &[u8],
    audit: &dyn FreshV4AccessAudit,
) -> Result<(), FreshV4RootError> {
    let parent = path.parent().ok_or_else(|| {
        FreshV4RootError::InvalidRoot(format!("marker has no parent: {}", path.display()))
    })?;
    let temporary = atomic_staging_path(path)?;
    match entry_kind(&temporary, audit)? {
        EntryKind::Missing => {}
        EntryKind::File => remove_file_durable(&temporary, audit)?,
        kind => {
            return Err(FreshV4RootError::State(format!(
                "marker staging path has invalid kind {kind:?}: {}",
                temporary.display()
            )));
        }
    }
    audit.record(FreshV4AccessKind::Write, &temporary)?;
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| {
                FreshV4RootError::io("create marker staging file", &temporary, error)
            })?;
        file.write_all(bytes).map_err(|error| {
            FreshV4RootError::io("write marker staging file", &temporary, error)
        })?;
        file.sync_all().map_err(|error| {
            FreshV4RootError::io("sync marker staging file", &temporary, error)
        })?;
        drop(file);
        audit.record(FreshV4AccessKind::Write, path)?;
        std::fs::rename(&temporary, path)
            .map_err(|error| FreshV4RootError::io("publish marker", path, error))?;
        sync_directory(parent)
    })();
    if write_result.is_err() {
        let _ = audit.record(FreshV4AccessKind::Remove, &temporary);
        let _ = std::fs::remove_file(&temporary);
        let _ = sync_parent(&temporary);
    }
    write_result
}

pub(crate) fn atomic_staging_path(path: &Path) -> Result<PathBuf, FreshV4RootError> {
    let parent = path.parent().ok_or_else(|| {
        FreshV4RootError::InvalidRoot(format!("marker has no parent: {}", path.display()))
    })?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            FreshV4RootError::InvalidRoot(format!(
                "marker filename is not UTF-8: {}",
                path.display()
            ))
        })?;
    Ok(parent.join(format!("{name}.staging")))
}

pub(crate) fn remove_file_durable(
    path: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<(), FreshV4RootError> {
    audit.record(FreshV4AccessKind::Remove, path)?;
    match std::fs::remove_file(path) {
        Ok(()) => sync_parent(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(FreshV4RootError::io("remove marker", path, error)),
    }
}

pub(crate) fn sync_regular_file(
    path: &Path,
    audit: &dyn FreshV4AccessAudit,
) -> Result<(), FreshV4RootError> {
    if entry_kind(path, audit)? != EntryKind::File {
        return Err(FreshV4RootError::State(format!(
            "database must be a regular file before ready publication: {}",
            path.display()
        )));
    }
    audit.record(FreshV4AccessKind::Database, path)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| FreshV4RootError::io("sync fresh-v4 database", path, error))
}

pub(crate) fn sync_parent(path: &Path) -> Result<(), FreshV4RootError> {
    let parent = path.parent().ok_or_else(|| {
        FreshV4RootError::InvalidRoot(format!("path has no parent: {}", path.display()))
    })?;
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<(), FreshV4RootError> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| FreshV4RootError::io("sync directory", directory, error))
}

#[cfg(windows)]
fn sync_directory(directory: &Path) -> Result<(), FreshV4RootError> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| FreshV4RootError::io("sync directory", directory, error))
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(directory: &Path) -> Result<(), FreshV4RootError> {
    Err(FreshV4RootError::InvalidRoot(format!(
        "durable directory synchronization is unavailable on this platform: {}",
        directory.display()
    )))
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
