use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use nomifun_common::AppError;
use same_file::Handle as SameFileHandle;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const WORKSPACE_OWNER_DIRECTORY: &str = ".nomifun";
pub const ARTIFACT_DIRECTORY: &str = "artifacts";
pub const ARTIFACT_RELATIVE_ROOT: &str = ".nomifun/artifacts";
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ARTIFACT_READ_BYTES: usize = 1024 * 1024;
static PUBLICATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn is_workspace_owner_component(component: &std::ffi::OsStr) -> bool {
    let Some(component) = component.to_str() else {
        return false;
    };
    #[cfg(windows)]
    {
        component.eq_ignore_ascii_case(WORKSPACE_OWNER_DIRECTORY)
    }
    #[cfg(not(windows))]
    {
        component == WORKSPACE_OWNER_DIRECTORY
    }
}

#[derive(Clone, Debug)]
/// Content-addressed artifacts owned by the workspace File domain.
///
/// Bytes live below the reserved `.nomifun/artifacts` directory so they stay
/// on the workspace filesystem for atomic publication without masquerading as
/// user source. Agent path resolution, File listings/search, file-watch
/// context and the App VCS owner all exclude the reserved `.nomifun` root.
pub struct WorkspaceArtifactStore {
    workspace_root: PathBuf,
    artifact_root: PathBuf,
}

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
            AppError::BadRequest(format!(
                "cannot resolve workspace artifact root '{}': {error}",
                workspace_root.as_ref().display()
            ))
        })?;
        if !workspace_root.is_dir() {
            return Err(AppError::BadRequest(
                "workspace artifact root is not a directory".to_owned(),
            ));
        }
        let artifact_root = workspace_root
            .join(WORKSPACE_OWNER_DIRECTORY)
            .join(ARTIFACT_DIRECTORY);
        Ok(Self {
            workspace_root,
            artifact_root,
        })
    }

    pub fn publish(
        &self,
        relative_source: &str,
        expected_sha256: Option<&str>,
    ) -> Result<PublishedWorkspaceArtifact, AppError> {
        let source = self.resolve_source(relative_source)?;
        let artifact_root = self.ensure_artifact_root()?;
        let staged = stage_source(&source, &artifact_root)?;
        let digest = staged.digest.clone();
        if expected_sha256.is_some_and(|expected| expected != digest.as_str()) {
            return Err(AppError::Conflict(format!(
                "workspace artifact source digest changed (expected {}, observed {digest})",
                expected_sha256.expect("checked Some")
            )));
        }

        let target = artifact_root.join(&digest);
        publish_content_addressed(&target, &staged)?;
        let target = fs::canonicalize(&target).map_err(|error| {
            AppError::Internal(format!("cannot resolve published workspace artifact: {error}"))
        })?;
        if !target.starts_with(&artifact_root) {
            return Err(AppError::Forbidden(
                "published workspace artifact escaped its owner directory".to_owned(),
            ));
        }

        Ok(PublishedWorkspaceArtifact {
            artifact_id: digest.clone(),
            source_path: relative_source.to_owned(),
            relative_path: format!("{ARTIFACT_RELATIVE_ROOT}/{digest}"),
            mime_type: mime_guess::from_path(&source)
                .first_or_octet_stream()
                .essence_str()
                .to_owned(),
            size_bytes: staged.size_bytes,
            sha256: digest,
        })
    }

    pub fn read(
        &self,
        artifact_id: &str,
        offset: u64,
        limit: usize,
    ) -> Result<WorkspaceArtifactRead, AppError> {
        validate_artifact_id(artifact_id)?;
        if limit == 0 || limit > MAX_ARTIFACT_READ_BYTES {
            return Err(AppError::BadRequest(format!(
                "workspace artifact read limit must be between 1 and {MAX_ARTIFACT_READ_BYTES} bytes"
            )));
        }
        let artifact_root = self.existing_artifact_root()?;
        let path = artifact_root.join(artifact_id);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::NotFound("workspace artifact was not found".to_owned())
            } else {
                AppError::BadRequest(format!("cannot inspect workspace artifact: {error}"))
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(AppError::Forbidden(
                "workspace artifact is not an owned regular file".to_owned(),
            ));
        }
        if metadata.len() > MAX_ARTIFACT_BYTES {
            return Err(AppError::BadRequest(format!(
                "workspace artifact exceeds the {MAX_ARTIFACT_BYTES}-byte limit"
            )));
        }
        if offset > metadata.len() {
            return Err(AppError::BadRequest(
                "workspace artifact offset is beyond the end of the file".to_owned(),
            ));
        }

        let (digest, bytes) = verified_chunk(&path, artifact_id, &metadata, offset, limit)?;
        let next_offset = offset + bytes.len() as u64;
        Ok(WorkspaceArtifactRead {
            artifact_id: artifact_id.to_owned(),
            offset,
            next_offset,
            size_bytes: metadata.len(),
            complete: next_offset == metadata.len(),
            sha256: digest,
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        })
    }

    fn resolve_source(&self, relative_source: &str) -> Result<PathBuf, AppError> {
        let relative = Path::new(relative_source);
        if relative_source.trim().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
            || relative_source.contains('\0')
            || relative_source.contains('\\')
            || relative.components().next().is_some_and(|component| {
                is_workspace_owner_component(component.as_os_str())
            })
        {
            return Err(AppError::BadRequest(
                "workspace artifact source must be a normalized workspace-relative path"
                    .to_owned(),
            ));
        }
        let candidate = self.workspace_root.join(relative);
        let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
            AppError::BadRequest(format!(
                "cannot inspect workspace artifact source '{relative_source}': {error}"
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(AppError::BadRequest(
                "workspace artifact source must be a regular non-symlink file".to_owned(),
            ));
        }
        let canonical = fs::canonicalize(&candidate).map_err(|error| {
            AppError::BadRequest(format!(
                "cannot resolve workspace artifact source '{relative_source}': {error}"
            ))
        })?;
        if !canonical.starts_with(&self.workspace_root) {
            return Err(AppError::Forbidden(
                "workspace artifact source is outside the publishable workspace".to_owned(),
            ));
        }
        Ok(canonical)
    }

    fn ensure_artifact_root(&self) -> Result<PathBuf, AppError> {
        let owner_root = self.workspace_root.join(WORKSPACE_OWNER_DIRECTORY);
        ensure_owned_directory(&owner_root, &self.workspace_root, "workspace owner")?;
        ensure_owned_directory(&self.artifact_root, &owner_root, "workspace artifact")?;
        self.existing_artifact_root()
    }

    fn existing_artifact_root(&self) -> Result<PathBuf, AppError> {
        let metadata = fs::symlink_metadata(&self.artifact_root).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::NotFound("workspace artifact store is empty".to_owned())
            } else {
                AppError::BadRequest(format!(
                    "cannot inspect workspace artifact owner directory: {error}"
                ))
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AppError::Forbidden(
                "workspace artifact owner path is not a regular directory".to_owned(),
            ));
        }
        let canonical = fs::canonicalize(&self.artifact_root).map_err(|error| {
            AppError::BadRequest(format!(
                "cannot resolve workspace artifact owner directory: {error}"
            ))
        })?;
        if !canonical.starts_with(&self.workspace_root) {
            return Err(AppError::Forbidden(
                "workspace artifact owner directory escaped the workspace".to_owned(),
            ));
        }
        Ok(canonical)
    }
}

fn validate_artifact_id(artifact_id: &str) -> Result<(), AppError> {
    if artifact_id.len() != 64
        || !artifact_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(AppError::BadRequest(
            "workspace artifact ID must be a lowercase SHA-256 digest".to_owned(),
        ));
    }
    Ok(())
}

struct StagedArtifact {
    path: PathBuf,
    identity: Option<SameFileHandle>,
    digest: String,
    size_bytes: u64,
}

impl Drop for StagedArtifact {
    fn drop(&mut self) {
        drop(self.identity.take());
        let _ = fs::remove_file(&self.path);
    }
}

fn stage_source(source: &Path, artifact_root: &Path) -> Result<StagedArtifact, AppError> {
    let mut source_file = File::open(source).map_err(|error| {
        AppError::BadRequest(format!("cannot open workspace artifact source: {error}"))
    })?;
    let before = source_file.metadata().map_err(|error| {
        AppError::BadRequest(format!("cannot inspect workspace artifact source: {error}"))
    })?;
    if !before.is_file() || before.len() == 0 || before.len() > MAX_ARTIFACT_BYTES {
        return Err(AppError::BadRequest(format!(
            "workspace artifact source must contain 1..={MAX_ARTIFACT_BYTES} bytes"
        )));
    }
    let before_modified = before.modified().map_err(|error| {
        AppError::BadRequest(format!(
            "cannot read workspace artifact source timestamp: {error}"
        ))
    })?;
    let source_identity = SameFileHandle::from_file(source_file.try_clone().map_err(|error| {
        AppError::BadRequest(format!("cannot clone workspace artifact source: {error}"))
    })?)
    .map_err(|error| {
        AppError::BadRequest(format!("cannot identify workspace artifact source: {error}"))
    })?;
    if SameFileHandle::from_path(source).map_err(|error| {
        AppError::BadRequest(format!("cannot identify workspace artifact source path: {error}"))
    })? != source_identity
    {
        return Err(AppError::Conflict(
            "workspace artifact source changed before publication".to_owned(),
        ));
    }

    let (temporary, mut staged) = create_staging_file(artifact_root)?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    let copy_result = (|| -> std::io::Result<()> {
        loop {
            let read = source_file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            total = total.saturating_add(read as u64);
            if total > MAX_ARTIFACT_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "workspace artifact source exceeds its byte limit",
                ));
            }
            hasher.update(&buffer[..read]);
            staged.write_all(&buffer[..read])?;
        }
        staged.sync_all()
    })();
    let staged_identity = if copy_result.is_ok() {
        staged
            .try_clone()
            .and_then(SameFileHandle::from_file)
            .map(Some)
    } else {
        Ok(None)
    };
    drop(staged);
    if let Err(error) = copy_result {
        let _ = fs::remove_file(&temporary);
        return Err(AppError::BadRequest(format!(
            "cannot stream workspace artifact source: {error}"
        )));
    }
    let staged_identity = staged_identity.map_err(|error| {
        let _ = fs::remove_file(&temporary);
        AppError::Internal(format!(
            "cannot identify workspace artifact staging file: {error}"
        ))
    })?;
    let staged = StagedArtifact {
        path: temporary,
        identity: staged_identity,
        digest: format!("{:x}", hasher.finalize()),
        size_bytes: total,
    };
    if staged.identity.as_ref().is_none_or(|identity| {
        SameFileHandle::from_path(&staged.path).map_or(true, |current| &current != identity)
    }) {
        return Err(AppError::Conflict(
            "workspace artifact staging identity changed during publication".to_owned(),
        ));
    }

    let after = source_file.metadata().map_err(|error| {
        AppError::Conflict(format!("workspace artifact source changed while publishing: {error}"))
    })?;
    let path_after = fs::metadata(source).map_err(|error| {
        AppError::Conflict(format!("workspace artifact source changed while publishing: {error}"))
    })?;
    let path_identity = SameFileHandle::from_path(source).map_err(|error| {
        AppError::Conflict(format!("workspace artifact source identity changed: {error}"))
    })?;
    if path_identity != source_identity
        || after.len() != before.len()
        || path_after.len() != before.len()
        || total != before.len()
        || after.modified().ok() != Some(before_modified)
        || path_after.modified().ok() != Some(before_modified)
    {
        return Err(AppError::Conflict(
            "workspace artifact source changed while publishing".to_owned(),
        ));
    }

    Ok(staged)
}

fn create_staging_file(artifact_root: &Path) -> Result<(PathBuf, File), AppError> {
    for _ in 0..16 {
        let sequence = PUBLICATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = artifact_root.join(format!(
            ".publish-{}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "cannot create workspace artifact staging file: {error}"
                )));
            }
        }
    }
    Err(AppError::Conflict(
        "workspace artifact staging namespace is exhausted".to_owned(),
    ))
}

fn publish_content_addressed(target: &Path, staged: &StagedArtifact) -> Result<(), AppError> {
    let current_staging_identity = SameFileHandle::from_path(&staged.path).map_err(|error| {
        AppError::Conflict(format!("cannot identify workspace artifact staging path: {error}"))
    })?;
    if staged
        .identity
        .as_ref()
        .is_none_or(|expected| expected != &current_staging_identity)
    {
        return Err(AppError::Conflict(
            "workspace artifact staging identity changed before publication".to_owned(),
        ));
    }
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(AppError::Conflict(
                "workspace artifact identity is occupied by a non-file entry".to_owned(),
            ));
        }
        Ok(_) => return verify_existing(target, &staged.digest, staged.size_bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::BadRequest(format!(
                "cannot inspect workspace artifact publication target: {error}"
            )));
        }
    }

    // The staging file is in the same owned directory, so a hard link is an
    // atomic create-if-absent publication. Unlike rename on Unix it never
    // replaces an existing digest identity during a concurrent publish.
    let publication = fs::hard_link(&staged.path, target);
    if let Err(error) = publication {
        if target.exists() {
            return verify_existing(target, &staged.digest, staged.size_bytes);
        }
        return Err(AppError::Internal(format!(
            "cannot atomically publish workspace artifact without replacement: {error}"
        )));
    }
    verify_existing(target, &staged.digest, staged.size_bytes)
}

fn verify_existing(path: &Path, expected_digest: &str, expected_size: u64) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AppError::Conflict(format!("cannot verify published workspace artifact: {error}"))
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() != expected_size
    {
        return Err(AppError::Conflict(
            "published workspace artifact does not match its receipt".to_owned(),
        ));
    }
    let _ = verified_chunk(path, expected_digest, &metadata, 0, 1)?;
    Ok(())
}

fn verified_chunk(
    path: &Path,
    expected_digest: &str,
    expected_metadata: &fs::Metadata,
    offset: u64,
    limit: usize,
) -> Result<(String, Vec<u8>), AppError> {
    let mut reader = File::open(path).map_err(|error| {
        AppError::BadRequest(format!("cannot open workspace artifact: {error}"))
    })?;
    let identity = SameFileHandle::from_file(reader.try_clone().map_err(|error| {
        AppError::BadRequest(format!("cannot clone workspace artifact handle: {error}"))
    })?)
    .map_err(|error| AppError::BadRequest(format!("cannot identify workspace artifact: {error}")))?;
    if SameFileHandle::from_path(path).map_err(|error| {
        AppError::BadRequest(format!("cannot identify workspace artifact path: {error}"))
    })? != identity
    {
        return Err(AppError::Conflict(
            "workspace artifact identity changed before reading".to_owned(),
        ));
    }
    let before = reader.metadata().map_err(|error| {
        AppError::BadRequest(format!("cannot inspect workspace artifact: {error}"))
    })?;
    let before_modified = before.modified().map_err(|error| {
        AppError::BadRequest(format!("cannot read workspace artifact timestamp: {error}"))
    })?;
    if before.len() != expected_metadata.len()
        || expected_metadata.modified().ok() != Some(before_modified)
    {
        return Err(AppError::Conflict(
            "workspace artifact changed before reading".to_owned(),
        ));
    }

    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let requested_end = offset.saturating_add(limit as u64).min(before.len());
    let mut position = 0_u64;
    let mut chunk = Vec::with_capacity((requested_end.saturating_sub(offset)) as usize);
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            AppError::BadRequest(format!("cannot read workspace artifact: {error}"))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        let block_start = position;
        let block_end = position + read as u64;
        let copy_start = block_start.max(offset);
        let copy_end = block_end.min(requested_end);
        if copy_start < copy_end {
            let local_start = (copy_start - block_start) as usize;
            let local_end = (copy_end - block_start) as usize;
            chunk.extend_from_slice(&buffer[local_start..local_end]);
        }
        position = block_end;
    }
    let digest = format!("{:x}", hasher.finalize());
    let after = reader.metadata().map_err(|error| {
        AppError::Conflict(format!("workspace artifact changed while reading: {error}"))
    })?;
    let path_after = fs::metadata(path).map_err(|error| {
        AppError::Conflict(format!("workspace artifact changed while reading: {error}"))
    })?;
    if SameFileHandle::from_path(path).map_err(|error| {
        AppError::Conflict(format!("workspace artifact identity changed: {error}"))
    })? != identity
        || position != before.len()
        || after.len() != before.len()
        || path_after.len() != before.len()
        || after.modified().ok() != Some(before_modified)
        || path_after.modified().ok() != Some(before_modified)
        || digest != expected_digest
    {
        return Err(AppError::Conflict(
            "workspace artifact changed or no longer matches its receipt".to_owned(),
        ));
    }
    Ok((digest, chunk))
}

fn ensure_owned_directory(path: &Path, parent: &Path, label: &str) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(AppError::Forbidden(format!(
                "{label} path is not a regular directory"
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Err(error) = fs::create_dir(path)
                && error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(AppError::Internal(format!(
                    "cannot create {label} directory: {error}"
                )));
            }
        }
        Err(error) => {
            return Err(AppError::BadRequest(format!(
                "cannot inspect {label} directory: {error}"
            )));
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AppError::BadRequest(format!("cannot verify {label} directory: {error}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Forbidden(format!(
            "{label} path is not a regular directory"
        )));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| AppError::BadRequest(format!("cannot resolve {label} directory: {error}")))?;
    if !canonical.starts_with(parent) {
        return Err(AppError::Forbidden(format!(
            "{label} directory escaped its owner"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_and_bounded_read_are_content_addressed() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("result.txt"), "artifact payload").unwrap();
        let store = WorkspaceArtifactStore::new(workspace.path()).unwrap();
        let published = store.publish("result.txt", None).unwrap();
        assert_eq!(
            published.relative_path,
            format!("{ARTIFACT_RELATIVE_ROOT}/{}", published.artifact_id)
        );
        assert!(workspace.path().join(&published.relative_path).is_file());
        let replay = store
            .publish("result.txt", Some(&published.sha256))
            .unwrap();
        assert_eq!(replay, published);

        let first = store.read(&published.artifact_id, 0, 8).unwrap();
        assert!(!first.complete);
        let second = store
            .read(
                &published.artifact_id,
                first.next_offset,
                MAX_ARTIFACT_READ_BYTES,
            )
            .unwrap();
        assert!(second.complete);
        let bytes = [first.data_base64, second.data_base64]
            .into_iter()
            .flat_map(|encoded| {
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(bytes, b"artifact payload");
    }

    #[test]
    fn publication_rejects_escape_symlink_and_stale_digest() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("result.txt"), "current").unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        let store = WorkspaceArtifactStore::new(workspace.path()).unwrap();
        assert!(store.publish("../secret.txt", None).is_err());
        assert!(store.publish("result.txt", Some(&"0".repeat(64))).is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("secret.txt"),
                workspace.path().join("escape.txt"),
            )
            .unwrap();
            assert!(store.publish("escape.txt", None).is_err());
        }
    }

    #[test]
    fn read_rejects_same_size_content_replacement() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("result.txt"), "original").unwrap();
        let store = WorkspaceArtifactStore::new(workspace.path()).unwrap();
        let published = store.publish("result.txt", None).unwrap();
        let artifact = workspace.path().join(&published.relative_path);
        let mut permissions = fs::metadata(&artifact).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&artifact, permissions).unwrap();
        fs::write(&artifact, "tampered").unwrap();

        assert!(matches!(
            store.read(&published.artifact_id, 0, MAX_ARTIFACT_READ_BYTES),
            Err(AppError::Conflict(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn publication_rejects_a_symlinked_owner_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("result.txt"), "artifact").unwrap();
        std::os::unix::fs::symlink(outside.path(), workspace.path().join(".nomifun"))
            .unwrap();
        let store = WorkspaceArtifactStore::new(workspace.path()).unwrap();
        assert!(matches!(
            store.publish("result.txt", None),
            Err(AppError::Forbidden(_))
        ));
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    }
}
