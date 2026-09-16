use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions as CapOpenOptions};
use nomifun_common::AppError;
use same_file::Handle as SameFileHandle;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const WORKSPACE_OWNER_DIRECTORY: &str = ".nomifun";
pub const ARTIFACT_DIRECTORY: &str = "artifacts";
pub const ARTIFACT_RELATIVE_ROOT: &str = ".nomifun/artifacts";
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ARTIFACT_READ_BYTES: usize = 1024 * 1024;
const ARTIFACT_CHUNK_BYTES: usize = 64 * 1024;
const MAX_STALE_PUBLICATION_CLEANUP: usize = 64;
const PUBLICATION_OUTCOME_UNKNOWN: &str = "artifact publication outcome is unknown";
static PUBLICATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn is_workspace_owner_component(component: &OsStr) -> bool {
    let Some(component) = component.to_str() else { return false };
    #[cfg(windows)]
    { component.eq_ignore_ascii_case(WORKSPACE_OWNER_DIRECTORY) }
    #[cfg(not(windows))]
    { component == WORKSPACE_OWNER_DIRECTORY }
}

pub(crate) fn normalized_workspace_relative(raw: &str, allow_empty: bool) -> Result<PathBuf, AppError> {
    if raw != raw.trim() || raw.contains(['\0', '\\']) {
        return Err(AppError::BadRequest("workspace path must use its exact portable relative form".into()));
    }
    if raw.is_empty() {
        return allow_empty.then(PathBuf::new)
            .ok_or_else(|| AppError::BadRequest("workspace path must not be empty".into()));
    }
    let mut normalized = PathBuf::new();
    let mut portable = Vec::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| AppError::BadRequest("workspace path must be UTF-8".into()))?;
                if portable.is_empty() && is_workspace_owner_component(component.as_os_str()) {
                    return Err(AppError::NotFound("workspace path was not found".into()));
                }
                portable.push(value);
                normalized.push(value);
            }
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::BadRequest("workspace path must be normalized and traversal-free".into()));
            }
        }
    }
    if portable.join("/") != raw {
        return Err(AppError::BadRequest("workspace path contains redundant separators or components".into()));
    }
    Ok(normalized)
}

/// Content-addressed artifacts owned through pinned, handle-relative paths.
#[derive(Clone)]
pub struct WorkspaceArtifactStore {
    workspace_path: PathBuf,
    workspace_identity: Arc<SameFileHandle>,
    workspace: Arc<Dir>,
    publication_lock: Arc<Mutex<()>>,
    cleanup_complete: Arc<AtomicBool>,
    verified: Arc<Mutex<HashMap<String, Arc<Mutex<VerifiedArtifact>>>>>,
    io_counters: Arc<ArtifactIoCounters>,
}

struct VerifiedArtifact {
    file: File,
    identity: SameFileHandle,
    size_bytes: u64,
    sha256: String,
    chunk_hashes: Vec<String>,
}

struct ArtifactNamespace { dir: Dir, identity: SameFileHandle }

#[derive(Default)]
struct ArtifactIoCounters { full_scan_bytes: AtomicU64, page_read_bytes: AtomicU64 }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWorkspaceArtifact {
    pub artifact_id: String,
    pub source_path: String,
    pub relative_path: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceArtifactRead {
    pub artifact_id: String,
    pub offset: u64,
    pub next_offset: u64,
    pub size_bytes: u64,
    pub complete: bool,
    pub sha256: String,
    pub data_base64: String,
}

impl WorkspaceArtifactStore {
    pub fn new(workspace_root: impl AsRef<Path>) -> Result<Self, AppError> {
        let workspace_root = fs::canonicalize(workspace_root.as_ref()).map_err(|error| {
            AppError::BadRequest(format!("cannot resolve workspace artifact root '{}': {error}", workspace_root.as_ref().display()))
        })?;
        if !workspace_root.is_dir() { return Err(AppError::BadRequest("workspace artifact root is not a directory".into())); }
        let workspace = Dir::open_ambient_dir(&workspace_root, ambient_authority())
            .map_err(|error| AppError::BadRequest(format!("cannot pin workspace artifact root '{}': {error}", workspace_root.display())))?;
        let workspace_identity = dir_identity(&workspace)?;
        let canonical_after = fs::canonicalize(&workspace_root).map_err(|error| {
            AppError::Conflict(format!("workspace artifact root changed while opening: {error}"))
        })?;
        let reopened = Dir::open_ambient_dir(&workspace_root, ambient_authority()).map_err(|error| {
            AppError::Conflict(format!("workspace artifact root changed while opening: {error}"))
        })?;
        if canonical_after != workspace_root || dir_identity(&reopened)? != workspace_identity {
            return Err(AppError::Conflict("workspace artifact root changed while opening".into()));
        }
        let store = Self {
            workspace_path: workspace_root,
            workspace_identity: Arc::new(workspace_identity),
            workspace: Arc::new(workspace),
            publication_lock: Arc::new(Mutex::new(())),
            cleanup_complete: Arc::new(AtomicBool::new(false)),
            verified: Arc::new(Mutex::new(HashMap::new())),
            io_counters: Arc::new(ArtifactIoCounters::default()),
        };
        match open_artifact_namespace(&store.workspace, false) {
            Ok(namespace) => {
                cleanup_stale_publications(&namespace.dir)?;
                store.cleanup_complete.store(true, Ordering::Release);
            }
            Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
        Ok(store)
    }

    pub fn publish(&self, source: &str, expected: Option<&str>) -> Result<PublishedWorkspaceArtifact, AppError> {
        self.publish_with_hooks(source, expected, || {}, || {})
    }

    fn publish_with_hooks<F: FnOnce(), G: FnOnce()>(&self, source: &str, expected: Option<&str>, after_source: F, after_namespace: G) -> Result<PublishedWorkspaceArtifact, AppError> {
        let _publication = self.publication_lock.lock().map_err(|_| {
            AppError::Internal("workspace artifact publication lock is poisoned".into())
        })?;
        self.verify_workspace()?;
        let relative = normalized_workspace_relative(source, false)?;
        let namespace = open_artifact_namespace(&self.workspace, true)?;
        let cleaned_now = !self.cleanup_complete.load(Ordering::Acquire);
        if cleaned_now {
            cleanup_stale_publications(&namespace.dir)?;
        }
        after_namespace();
        verify_namespace(&self.workspace, &namespace)?;
        if cleaned_now {
            self.cleanup_complete.store(true, Ordering::Release);
        }
        let staged = stage_source(
            &self.workspace,
            &relative,
            &namespace,
            &self.io_counters,
            after_source,
        )?;
        self.verify_workspace()?;
        let digest = staged.digest.clone();
        if expected.is_some_and(|value| value != digest.as_str()) {
            return Err(AppError::Conflict(format!("workspace artifact source digest changed (expected {}, observed {digest})", expected.unwrap())));
        }
        let (created, verified) = publish_content_addressed(&namespace, &digest, &staged, &self.io_counters)?;
        if let Err(error) = verify_namespace(&self.workspace, &namespace) {
            drop(verified);
            if created {
                rollback_published(&namespace.dir, &digest).map_err(|cleanup| {
                    AppError::Conflict(format!(
                        "{PUBLICATION_OUTCOME_UNKNOWN}: namespace changed and rollback failed: {cleanup}"
                    ))
                })?;
            }
            return Err(error);
        }
        if let Err(error) = self.verify_workspace() {
            drop(verified);
            if created {
                rollback_published(&namespace.dir, &digest).map_err(|cleanup| {
                    AppError::Conflict(format!(
                        "{PUBLICATION_OUTCOME_UNKNOWN}: workspace changed and rollback failed: {cleanup}"
                    ))
                })?;
            }
            return Err(error);
        }
        self.verified.lock().map_err(|_| AppError::Internal("workspace artifact cache is poisoned".into()))?
            .insert(digest.clone(), Arc::new(Mutex::new(verified)));
        Ok(PublishedWorkspaceArtifact {
            artifact_id: digest.clone(), source_path: source.into(), relative_path: format!("{ARTIFACT_RELATIVE_ROOT}/{digest}"),
            mime_type: mime_guess::from_path(source).first_or_octet_stream().essence_str().into(), size_bytes: staged.size_bytes, sha256: digest,
        })
    }

    pub fn read(&self, artifact_id: &str, offset: u64, limit: usize) -> Result<WorkspaceArtifactRead, AppError> {
        self.verify_workspace()?;
        validate_artifact_id(artifact_id)?;
        if limit == 0 || limit > MAX_ARTIFACT_READ_BYTES { return Err(AppError::BadRequest(format!("workspace artifact read limit must be between 1 and {MAX_ARTIFACT_READ_BYTES} bytes"))); }
        let namespace = open_artifact_namespace(&self.workspace, false)?;
        verify_namespace(&self.workspace, &namespace)?;
        self.verify_workspace()?;
        let verified = self.verified_artifact(&namespace, artifact_id)?;
        let mut verified = verified.lock().map_err(|_| AppError::Internal("workspace artifact handle is poisoned".into()))?;
        if offset > verified.size_bytes { return Err(AppError::BadRequest("workspace artifact offset is beyond the end of the file".into())); }
        let bytes = read_verified_page(&namespace, artifact_id, &mut verified, offset, limit, &self.io_counters)?;
        verify_namespace(&self.workspace, &namespace)?;
        let next_offset = offset + bytes.len() as u64;
        Ok(WorkspaceArtifactRead { artifact_id: artifact_id.into(), offset, next_offset, size_bytes: verified.size_bytes, complete: next_offset == verified.size_bytes, sha256: verified.sha256.clone(), data_base64: base64::engine::general_purpose::STANDARD.encode(bytes) })
    }

    fn verified_artifact(&self, namespace: &ArtifactNamespace, id: &str) -> Result<Arc<Mutex<VerifiedArtifact>>, AppError> {
        let mut cache = self.verified.lock().map_err(|_| AppError::Internal("workspace artifact cache is poisoned".into()))?;
        if let Some(value) = cache.get(id) { return Ok(Arc::clone(value)); }
        let value = Arc::new(Mutex::new(load_verified_artifact(namespace, id, &self.io_counters)?));
        cache.insert(id.into(), Arc::clone(&value));
        Ok(value)
    }

    fn verify_workspace(&self) -> Result<(), AppError> {
        let canonical = fs::canonicalize(&self.workspace_path).map_err(|error| {
            AppError::Conflict(format!("workspace artifact root changed: {error}"))
        })?;
        let reopened = Dir::open_ambient_dir(&self.workspace_path, ambient_authority())
            .map_err(|error| AppError::Conflict(format!("workspace artifact root changed: {error}")))?;
        let reopened_identity = dir_identity(&reopened)?;
        if canonical != self.workspace_path
            || &reopened_identity != self.workspace_identity.as_ref()
        {
            return Err(AppError::Conflict(
                "workspace artifact root identity changed during the operation".into(),
            ));
        }
        Ok(())
    }

    #[cfg(test)] fn io_counts(&self) -> (u64, u64) { (self.io_counters.full_scan_bytes.load(Ordering::Acquire), self.io_counters.page_read_bytes.load(Ordering::Acquire)) }
}

fn validate_artifact_id(id: &str) -> Result<(), AppError> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f')) { return Err(AppError::BadRequest("workspace artifact ID must be a lowercase SHA-256 digest".into())); }
    Ok(())
}

struct StagedArtifact { dir: Dir, name: String, identity: Option<SameFileHandle>, digest: String, size_bytes: u64, chunk_hashes: Vec<String> }
impl Drop for StagedArtifact { fn drop(&mut self) { drop(self.identity.take()); let _ = self.dir.remove_file(&self.name); } }

fn open_artifact_namespace(workspace: &Dir, create: bool) -> Result<ArtifactNamespace, AppError> {
    let owner = open_owned_dir(workspace, WORKSPACE_OWNER_DIRECTORY, create, "workspace owner")?;
    let dir = open_owned_dir(&owner, ARTIFACT_DIRECTORY, create, "workspace artifact")?;
    let identity = dir_identity(&dir)?;
    Ok(ArtifactNamespace { dir, identity })
}

fn open_owned_dir(parent: &Dir, name: &str, create: bool, label: &str) -> Result<Dir, AppError> {
    match parent.open_dir_nofollow(name) {
        Ok(dir) => Ok(dir),
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
            if let Err(error) = parent.create_dir(name) && error.kind() != std::io::ErrorKind::AlreadyExists { return Err(AppError::Internal(format!("cannot create {label} directory: {error}"))); }
            parent.open_dir_nofollow(name).map_err(|error| AppError::Forbidden(format!("cannot pin {label} directory: {error}")))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(AppError::NotFound("workspace artifact store is empty".into())),
        Err(error) => Err(AppError::Forbidden(format!("cannot pin {label} directory without following links: {error}"))),
    }
}

fn dir_identity(dir: &Dir) -> Result<SameFileHandle, AppError> {
    SameFileHandle::from_file(dir.try_clone().map_err(|error| AppError::Internal(error.to_string()))?.into_std_file())
        .map_err(|error| AppError::Internal(format!("cannot identify owned directory: {error}")))
}

fn verify_namespace(workspace: &Dir, expected: &ArtifactNamespace) -> Result<(), AppError> {
    if open_artifact_namespace(workspace, false)?.identity != expected.identity { return Err(AppError::Conflict("workspace artifact namespace changed during the operation".into())); }
    Ok(())
}

fn owner_publication_temp_process(name: &str) -> Option<u32> {
    let Some(body) = name
        .strip_prefix(".publish-")
        .and_then(|value| value.strip_suffix(".tmp"))
    else {
        return None;
    };
    let Some((process, sequence)) = body.split_once('-') else {
        return None;
    };
    let valid = !process.is_empty()
        && !sequence.is_empty()
        && !sequence.contains('-')
        && process.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.bytes().all(|byte| byte.is_ascii_digit());
    valid.then(|| process.parse::<u32>().ok()).flatten()
}

#[cfg(unix)]
fn sync_directory(directory: &Dir) -> Result<(), AppError> {
    directory
        .try_clone()
        .map_err(|error| AppError::Internal(error.to_string()))?
        .into_std_file()
        .sync_all()
        .map_err(|error| AppError::Internal(format!("cannot sync artifact directory: {error}")))
}

#[cfg(windows)]
fn sync_directory(directory: &Dir) -> Result<(), AppError> {
    let result = directory
        .try_clone()
        .map_err(|error| AppError::Internal(error.to_string()))?
        .into_std_file()
        .sync_all();
    match result {
        Ok(()) => Ok(()),
        // Windows rejects FlushFileBuffers for directory handles opened with
        // backup semantics. The staged file itself was flushed before the
        // handle-relative hard-link, and NTFS exposes no directory-fsync
        // equivalent to strengthen that metadata boundary.
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
        Err(error) => Err(AppError::Internal(format!(
            "cannot sync artifact directory: {error}"
        ))),
    }
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_directory: &Dir) -> Result<(), AppError> {
    Ok(())
}

fn rollback_published(directory: &Dir, target: &str) -> Result<(), AppError> {
    directory.remove_file(target).map_err(|error| {
        AppError::Conflict(format!("cannot remove unconfirmed artifact link: {error}"))
    })?;
    sync_directory(directory)
}

fn cleanup_stale_publications(directory: &Dir) -> Result<(), AppError> {
    let mut stale = Vec::new();
    for entry in directory.entries().map_err(|error| {
        AppError::Internal(format!("cannot inspect artifact staging namespace: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            AppError::Internal(format!("cannot inspect artifact staging entry: {error}"))
        })?;
        let name = entry.file_name();
        if name.to_str().and_then(owner_publication_temp_process)
            .is_some_and(|process| process != std::process::id()) {
            if stale.len() >= MAX_STALE_PUBLICATION_CLEANUP {
                return Err(AppError::Conflict(format!(
                    "artifact staging cleanup exceeds {MAX_STALE_PUBLICATION_CLEANUP} owner temporary files"
                )));
            }
            stale.push(name);
        }
    }
    for name in &stale {
        directory.remove_file(name).map_err(|error| {
            AppError::Conflict(format!("cannot remove stale artifact staging file: {error}"))
        })?;
    }
    if !stale.is_empty() {
        sync_directory(directory)?;
    }
    Ok(())
}

fn read_options() -> CapOpenOptions { let mut value = CapOpenOptions::new(); value.read(true).follow(FollowSymlinks::No); value }
fn create_options() -> CapOpenOptions { let mut value = CapOpenOptions::new(); value.write(true).create_new(true).follow(FollowSymlinks::No); value }

fn validate_open_source(workspace: &Dir, path: &Path, expected: &SameFileHandle) -> Result<(), AppError> {
    let canonical = workspace.canonicalize(path).map_err(|error| AppError::Forbidden(format!("workspace artifact source escaped its root: {error}")))?;
    let mut components = canonical.components();
    let first = components.next().ok_or_else(|| AppError::BadRequest("workspace artifact source is empty".into()))?;
    if !matches!(first, Component::Normal(_))
        || is_workspace_owner_component(first.as_os_str())
        || components.any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppError::Forbidden("workspace artifact source escaped its allowed namespace".into()));
    }
    let reopened = workspace.open_with(path, &read_options()).map_err(|error| AppError::Conflict(format!("workspace artifact source changed: {error}")))?.into_std();
    let identity = SameFileHandle::from_file(reopened).map_err(|error| AppError::Conflict(format!("cannot identify workspace artifact source: {error}")))?;
    if &identity != expected { return Err(AppError::Conflict("workspace artifact source identity changed".into())); }
    Ok(())
}

fn stage_source<F: FnOnce()>(workspace: &Dir, source: &Path, namespace: &ArtifactNamespace, counters: &ArtifactIoCounters, after_source_open: F) -> Result<StagedArtifact, AppError> {
    let mut source_file = workspace.open_with(source, &read_options()).map_err(|error| AppError::BadRequest(format!("cannot open workspace artifact source: {error}")))?.into_std();
    let source_identity = SameFileHandle::from_file(source_file.try_clone().map_err(|error| AppError::BadRequest(error.to_string()))?).map_err(|error| AppError::BadRequest(error.to_string()))?;
    validate_open_source(workspace, source, &source_identity)?;
    let before = source_file.metadata().map_err(|error| AppError::BadRequest(error.to_string()))?;
    if !before.is_file() || before.len() == 0 || before.len() > MAX_ARTIFACT_BYTES { return Err(AppError::BadRequest(format!("workspace artifact source must contain 1..={MAX_ARTIFACT_BYTES} bytes"))); }
    let modified = before.modified().ok();
    let (name, mut staged_file, staged_identity) = create_staging_file(namespace)?;
    after_source_open();
    let mut whole = Sha256::new(); let mut chunks = Vec::new(); let mut total = 0_u64; let mut buffer = vec![0_u8; ARTIFACT_CHUNK_BYTES];
    loop { let read = source_file.read(&mut buffer).map_err(|error| AppError::BadRequest(error.to_string()))?; if read == 0 { break; } total += read as u64; if total > MAX_ARTIFACT_BYTES { return Err(AppError::BadRequest("workspace artifact source exceeds its byte limit".into())); } counters.full_scan_bytes.fetch_add(read as u64, Ordering::Relaxed); whole.update(&buffer[..read]); chunks.push(format!("{:x}", Sha256::digest(&buffer[..read]))); staged_file.write_all(&buffer[..read]).map_err(|error| AppError::Internal(error.to_string()))?; }
    staged_file.sync_all().map_err(|error| AppError::Internal(error.to_string()))?; drop(staged_file);
    validate_open_source(workspace, source, &source_identity)?;
    let after = source_file.metadata().map_err(|error| AppError::Conflict(error.to_string()))?;
    if total != before.len() || after.len() != before.len() || after.modified().ok() != modified { return Err(AppError::Conflict("workspace artifact source changed while publishing".into())); }
    let staged = StagedArtifact { dir: namespace.dir.try_clone().map_err(|error| AppError::Internal(error.to_string()))?, name, identity: Some(staged_identity), digest: format!("{:x}", whole.finalize()), size_bytes: total, chunk_hashes: chunks };
    verify_staged_identity(&staged)?; Ok(staged)
}

fn create_staging_file(namespace: &ArtifactNamespace) -> Result<(String, File, SameFileHandle), AppError> {
    for _ in 0..16 { let name = format!(".publish-{}-{}.tmp", std::process::id(), PUBLICATION_SEQUENCE.fetch_add(1, Ordering::Relaxed)); match namespace.dir.open_with(&name, &create_options()) { Ok(file) => { let file = file.into_std(); let identity = SameFileHandle::from_file(file.try_clone().map_err(|error| AppError::Internal(error.to_string()))?).map_err(|error| AppError::Internal(error.to_string()))?; return Ok((name, file, identity)); }, Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue, Err(error) => return Err(AppError::Internal(error.to_string())) } }
    Err(AppError::Conflict("workspace artifact staging namespace is exhausted".into()))
}

fn verify_staged_identity(staged: &StagedArtifact) -> Result<(), AppError> {
    let current = staged.dir.open_with(&staged.name, &read_options()).map_err(|error| AppError::Conflict(error.to_string()))?.into_std();
    let current = SameFileHandle::from_file(current).map_err(|error| AppError::Conflict(error.to_string()))?;
    if staged.identity.as_ref().is_none_or(|expected| expected != &current) { return Err(AppError::Conflict("artifact staging identity changed before publication".into())); }
    Ok(())
}

fn publish_content_addressed(namespace: &ArtifactNamespace, target: &str, staged: &StagedArtifact, counters: &ArtifactIoCounters) -> Result<(bool, VerifiedArtifact), AppError> {
    verify_staged_identity(staged)?;
    match namespace.dir.symlink_metadata(target) { Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => return Ok((false, load_verified_artifact(namespace, target, counters)?)), Ok(_) => return Err(AppError::Conflict("workspace artifact identity is occupied by a non-file entry".into())), Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}, Err(error) => return Err(AppError::BadRequest(error.to_string())) }
    if let Err(error) = staged.dir.hard_link(&staged.name, &namespace.dir, target) { if namespace.dir.metadata(target).is_ok() { return Ok((false, load_verified_artifact(namespace, target, counters)?)); } return Err(AppError::Internal(format!("cannot atomically publish artifact: {error}"))); }
    if let Err(sync_error) = sync_directory(&namespace.dir) {
        if let Err(cleanup_error) = rollback_published(&namespace.dir, target) {
            return Err(AppError::Conflict(format!(
                "{PUBLICATION_OUTCOME_UNKNOWN}: directory sync failed ({sync_error}) and rollback failed ({cleanup_error})"
            )));
        }
        return Err(sync_error);
    }
    let file = namespace.dir.open_with(target, &read_options()).map_err(|error| AppError::Conflict(error.to_string()))?.into_std();
    let identity = SameFileHandle::from_file(file.try_clone().map_err(|error| AppError::Conflict(error.to_string()))?).map_err(|error| AppError::Conflict(error.to_string()))?;
    if staged.identity.as_ref().is_none_or(|expected| expected != &identity) || file.metadata().map_err(|error| AppError::Conflict(error.to_string()))?.len() != staged.size_bytes { return Err(AppError::Conflict("published artifact differs from staged bytes".into())); }
    Ok((true, VerifiedArtifact { file, identity, size_bytes: staged.size_bytes, sha256: staged.digest.clone(), chunk_hashes: staged.chunk_hashes.clone() }))
}

fn load_verified_artifact(namespace: &ArtifactNamespace, id: &str, counters: &ArtifactIoCounters) -> Result<VerifiedArtifact, AppError> {
    let mut file = namespace.dir.open_with(id, &read_options()).map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { AppError::NotFound("workspace artifact was not found".into()) } else { AppError::BadRequest(error.to_string()) })?.into_std();
    let identity = SameFileHandle::from_file(file.try_clone().map_err(|error| AppError::BadRequest(error.to_string()))?).map_err(|error| AppError::BadRequest(error.to_string()))?;
    let metadata = file.metadata().map_err(|error| AppError::BadRequest(error.to_string()))?; if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ARTIFACT_BYTES { return Err(AppError::Conflict("workspace artifact has an invalid stored size".into())); }
    let mut whole = Sha256::new(); let mut chunks = Vec::new(); let mut total = 0_u64; let mut buffer = vec![0_u8; ARTIFACT_CHUNK_BYTES];
    loop { let read = file.read(&mut buffer).map_err(|error| AppError::BadRequest(error.to_string()))?; if read == 0 { break; } counters.full_scan_bytes.fetch_add(read as u64, Ordering::Relaxed); total += read as u64; whole.update(&buffer[..read]); chunks.push(format!("{:x}", Sha256::digest(&buffer[..read]))); }
    let digest = format!("{:x}", whole.finalize()); if total != metadata.len() || digest != id { return Err(AppError::Conflict("workspace artifact no longer matches its content identity".into())); }
    verify_target_identity(namespace, id, &identity, total)?; file.seek(SeekFrom::Start(0)).map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(VerifiedArtifact { file, identity, size_bytes: total, sha256: digest, chunk_hashes: chunks })
}

fn verify_target_identity(namespace: &ArtifactNamespace, id: &str, expected: &SameFileHandle, size: u64) -> Result<(), AppError> {
    let file = namespace.dir.open_with(id, &read_options()).map_err(|error| AppError::Conflict(error.to_string()))?.into_std();
    let identity = SameFileHandle::from_file(file.try_clone().map_err(|error| AppError::Conflict(error.to_string()))?).map_err(|error| AppError::Conflict(error.to_string()))?;
    if &identity != expected || file.metadata().map_err(|error| AppError::Conflict(error.to_string()))?.len() != size { return Err(AppError::Conflict("workspace artifact path identity changed".into())); }
    Ok(())
}

fn read_verified_page(namespace: &ArtifactNamespace, id: &str, verified: &mut VerifiedArtifact, offset: u64, limit: usize, counters: &ArtifactIoCounters) -> Result<Vec<u8>, AppError> {
    verify_target_identity(namespace, id, &verified.identity, verified.size_bytes)?;
    let end = offset.saturating_add(limit as u64).min(verified.size_bytes); let first = (offset / ARTIFACT_CHUNK_BYTES as u64) as usize; let last = if end == offset { first } else { ((end - 1) / ARTIFACT_CHUNK_BYTES as u64) as usize }; let mut output = Vec::with_capacity((end - offset) as usize);
    for index in first..=last { let start = index as u64 * ARTIFACT_CHUNK_BYTES as u64; if start >= verified.size_bytes { break; } let len = (verified.size_bytes - start).min(ARTIFACT_CHUNK_BYTES as u64) as usize; let mut chunk = vec![0_u8; len]; verified.file.seek(SeekFrom::Start(start)).map_err(|error| AppError::BadRequest(error.to_string()))?; verified.file.read_exact(&mut chunk).map_err(|error| AppError::Conflict(error.to_string()))?; counters.page_read_bytes.fetch_add(len as u64, Ordering::Relaxed); let digest = format!("{:x}", Sha256::digest(&chunk)); if verified.chunk_hashes.get(index).map(String::as_str) != Some(digest.as_str()) { return Err(AppError::Conflict("workspace artifact chunk no longer matches verified content".into())); } let copy_start = offset.max(start); let copy_end = end.min(start + len as u64); if copy_start < copy_end { output.extend_from_slice(&chunk[(copy_start-start) as usize..(copy_end-start) as usize]); } }
    verify_target_identity(namespace, id, &verified.identity, verified.size_bytes)?; Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn publish_and_read_round_trip() { let workspace=tempfile::tempdir().unwrap(); fs::write(workspace.path().join("result.txt"),"artifact payload").unwrap(); let store=WorkspaceArtifactStore::new(workspace.path()).unwrap(); let published=store.publish("result.txt",None).unwrap(); let first=store.read(&published.artifact_id,0,8).unwrap(); let second=store.read(&published.artifact_id,first.next_offset,MAX_ARTIFACT_READ_BYTES).unwrap(); let bytes=[first.data_base64,second.data_base64].into_iter().flat_map(|v|base64::engine::general_purpose::STANDARD.decode(v).unwrap()).collect::<Vec<_>>(); assert_eq!(bytes,b"artifact payload"); }
    #[test] fn rejects_noncanonical_owner_paths() { let workspace=tempfile::tempdir().unwrap(); fs::write(workspace.path().join("result.txt"),"x").unwrap(); let store=WorkspaceArtifactStore::new(workspace.path()).unwrap(); for path in ["../x","./result.txt",".nomifun/artifacts/x","nested//x"] { assert!(store.publish(path,None).is_err(),"{path}"); } #[cfg(windows)] assert!(store.publish(".NOMIFUN/artifacts/x",None).is_err()); }
    #[test] fn tampered_chunk_is_rejected() { let workspace=tempfile::tempdir().unwrap(); fs::write(workspace.path().join("result.txt"),"original").unwrap(); let store=WorkspaceArtifactStore::new(workspace.path()).unwrap(); let artifact=store.publish("result.txt",None).unwrap(); fs::write(workspace.path().join(&artifact.relative_path),"tampered").unwrap(); assert!(store.read(&artifact.artifact_id,0,MAX_ARTIFACT_READ_BYTES).is_err()); }
    #[test] fn pages_reuse_verified_index() { let workspace=tempfile::tempdir().unwrap(); fs::write(workspace.path().join("large.bin"),vec![b'x';ARTIFACT_CHUNK_BYTES*8]).unwrap(); let store=WorkspaceArtifactStore::new(workspace.path()).unwrap(); let artifact=store.publish("large.bin",None).unwrap(); let before=store.io_counts(); for offset in [0,17_000,131_000,260_000] { store.read(&artifact.artifact_id,offset,16_384).unwrap(); } let after=store.io_counts(); assert_eq!(after.0,before.0); assert!(after.1-before.1<=4*2*ARTIFACT_CHUNK_BYTES as u64); }
    #[test] fn startup_cleanup_removes_only_strict_owner_publication_temps() { let workspace=tempfile::tempdir().unwrap(); let artifacts=workspace.path().join(ARTIFACT_RELATIVE_ROOT); fs::create_dir_all(&artifacts).unwrap(); let live=format!(".publish-{}-99.tmp",std::process::id()); fs::write(artifacts.join(".publish-4294967295-34.tmp"),"stale").unwrap(); fs::write(artifacts.join(&live),"possibly live").unwrap(); fs::write(artifacts.join(".publish-owner.tmp"),"keep").unwrap(); fs::write(artifacts.join(".publish-12-34-extra.tmp"),"keep").unwrap(); fs::write(artifacts.join("ordinary.tmp"),"keep").unwrap(); WorkspaceArtifactStore::new(workspace.path()).unwrap(); assert!(!artifacts.join(".publish-4294967295-34.tmp").exists()); assert!(artifacts.join(live).exists()); assert!(artifacts.join(".publish-owner.tmp").exists()); assert!(artifacts.join(".publish-12-34-extra.tmp").exists()); assert!(artifacts.join("ordinary.tmp").exists()); }
    #[test] fn concurrent_publications_share_one_temp_and_link_owner_gate() { let workspace=tempfile::tempdir().unwrap(); fs::write(workspace.path().join("first"),"first").unwrap(); fs::write(workspace.path().join("second"),"second").unwrap(); let store=Arc::new(WorkspaceArtifactStore::new(workspace.path()).unwrap()); let (entered_tx,entered_rx)=std::sync::mpsc::channel(); let (release_tx,release_rx)=std::sync::mpsc::channel(); let first_store=Arc::clone(&store); let first=std::thread::spawn(move|| first_store.publish_with_hooks("first",None,move||{entered_tx.send(()).unwrap();release_rx.recv().unwrap();},||{})); entered_rx.recv().unwrap(); let (done_tx,done_rx)=std::sync::mpsc::channel(); let second_store=Arc::clone(&store); let second=std::thread::spawn(move||{let result=second_store.publish("second",None);done_tx.send(result.is_ok()).unwrap();result}); assert!(done_rx.recv_timeout(std::time::Duration::from_millis(100)).is_err()); release_tx.send(()).unwrap(); let first=first.join().unwrap().unwrap(); let second=second.join().unwrap().unwrap(); assert_ne!(first.artifact_id,second.artifact_id); assert!(store.read(&first.artifact_id,0,MAX_ARTIFACT_READ_BYTES).is_ok()); assert!(store.read(&second.artifact_id,0,MAX_ARTIFACT_READ_BYTES).is_ok()); }
    #[cfg(unix)] #[test] fn final_component_swap_is_rejected() { let workspace=tempfile::tempdir().unwrap(); let outside=tempfile::tempdir().unwrap(); fs::write(workspace.path().join("result.txt"),"inside").unwrap(); fs::write(outside.path().join("secret"),"outside").unwrap(); let store=WorkspaceArtifactStore::new(workspace.path()).unwrap(); let result=store.publish_with_hooks("result.txt",None,||{fs::remove_file(workspace.path().join("result.txt")).unwrap();std::os::unix::fs::symlink(outside.path().join("secret"),workspace.path().join("result.txt")).unwrap();},||{}); assert!(result.is_err()); }
    #[cfg(unix)] #[test] fn ancestor_swap_is_rejected() { let workspace=tempfile::tempdir().unwrap(); let outside=tempfile::tempdir().unwrap(); fs::create_dir(workspace.path().join("nested")).unwrap(); fs::write(workspace.path().join("nested/result"),"inside").unwrap(); fs::write(outside.path().join("result"),"outside").unwrap(); let store=WorkspaceArtifactStore::new(workspace.path()).unwrap(); let result=store.publish_with_hooks("nested/result",None,||{fs::rename(workspace.path().join("nested"),workspace.path().join("owned")).unwrap();std::os::unix::fs::symlink(outside.path(),workspace.path().join("nested")).unwrap();},||{}); assert!(result.is_err()); }
    #[cfg(unix)] #[test] fn owner_namespace_swap_writes_nothing_outside() { let workspace=tempfile::tempdir().unwrap(); let outside=tempfile::tempdir().unwrap(); fs::write(workspace.path().join("result"),"inside").unwrap(); let store=WorkspaceArtifactStore::new(workspace.path()).unwrap(); let result=store.publish_with_hooks("result",None,||{},||{fs::rename(workspace.path().join(".nomifun"),workspace.path().join("owned")).unwrap();std::os::unix::fs::symlink(outside.path(),workspace.path().join(".nomifun")).unwrap();}); assert!(result.is_err()); assert!(fs::read_dir(outside.path()).unwrap().next().is_none()); }
    #[cfg(windows)]
    #[test]
    fn ancestor_and_owner_junction_swaps_are_rejected() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("result"), "outside").unwrap();

        let ancestor_workspace = tempfile::tempdir().unwrap();
        fs::create_dir(ancestor_workspace.path().join("nested")).unwrap();
        fs::write(
            ancestor_workspace.path().join("nested/result"),
            "inside",
        )
        .unwrap();
        let ancestor_store =
            WorkspaceArtifactStore::new(ancestor_workspace.path()).unwrap();
        let ancestor_swapped = std::sync::atomic::AtomicBool::new(false);
        let ancestor = ancestor_store.publish_with_hooks(
            "nested/result",
            None,
            || {
                if fs::rename(
                    ancestor_workspace.path().join("nested"),
                    ancestor_workspace.path().join("owned"),
                )
                .is_ok()
                {
                    junction::create(
                        outside.path(),
                        ancestor_workspace.path().join("nested"),
                    )
                    .unwrap();
                    ancestor_swapped.store(true, Ordering::Release);
                }
            },
            || {},
        );
        if ancestor_swapped.load(Ordering::Acquire) {
            assert!(ancestor.is_err());
            junction::delete(ancestor_workspace.path().join("nested")).unwrap();
        } else {
            // Windows sharing rules denied the raced directory replacement
            // while the pinned source handle was live.
            assert!(ancestor.is_ok());
        }

        let owner_workspace = tempfile::tempdir().unwrap();
        fs::write(owner_workspace.path().join("result"), "inside").unwrap();
        let owner_store = WorkspaceArtifactStore::new(owner_workspace.path()).unwrap();
        let owner_swapped = std::sync::atomic::AtomicBool::new(false);
        let owner = owner_store.publish_with_hooks(
            "result",
            None,
            || {},
            || {
                if fs::rename(
                    owner_workspace.path().join(".nomifun"),
                    owner_workspace.path().join(".nomifun-owned"),
                )
                .is_ok()
                {
                    junction::create(
                        outside.path(),
                        owner_workspace.path().join(".nomifun"),
                    )
                    .unwrap();
                    owner_swapped.store(true, Ordering::Release);
                }
            },
        );
        if owner_swapped.load(Ordering::Acquire) {
            assert!(owner.is_err());
            junction::delete(owner_workspace.path().join(".nomifun")).unwrap();
        } else {
            assert!(owner.is_ok());
        }
        assert!(
            fs::read_dir(outside.path())
                .unwrap()
                .all(|entry| entry.unwrap().file_name() == "result")
        );
    }
}
