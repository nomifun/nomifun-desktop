use std::collections::{BTreeSet, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Instant, UNIX_EPOCH};

use base64::Engine;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

use nomifun_api_types::WebSocketMessage;
use nomifun_common::AppError;
use nomifun_realtime::UserEventSink;

use crate::path_safety::{
    PathAuthority, has_traversal, is_unsafe_path_segment, validate_path, validate_path_authority,
    validate_path_for_write, validate_path_for_write_authority, validate_path_with_extra_root,
};
use crate::resource::AgentSessionWorkspaceBinding;
use crate::agent_patch_outcome::{AgentPatchFailureObservation, AgentSessionPatchFailure, PatchPublicationFailure};
use crate::types::{
    ContentUpdateEvent, ContentUpdateOperation, CopyResult, DirOrFile, FileMetadata, WorkspaceFlatFile, ZipEntry,
};

/// Maximum number of files returned by `list_workspace_files`.
const MAX_WORKSPACE_FILES: usize = 20_000;

/// Maximum file size for read operations (256 MB).
const MAX_FILE_SIZE: u64 = 256 * 1024 * 1024;

/// Maximum remote image size (5 MB).
const MAX_REMOTE_IMAGE_SIZE: usize = 5 * 1024 * 1024;

/// Maximum number of HTTP redirects for remote image fetching.
const MAX_REDIRECTS: usize = 5;

/// Maximum number of files accepted by one agent patch request.
pub const MAX_AGENT_PATCH_FILES: usize = 64;

/// Maximum number of hunks accepted for one file in an agent patch request.
pub const MAX_AGENT_PATCH_HUNKS_PER_FILE: usize = 256;

/// Maximum number of patch lines accepted for one hunk.
pub const MAX_AGENT_PATCH_LINES_PER_HUNK: usize = 16_384;

/// Maximum number of source/output lines accepted for one patched file.
pub const MAX_AGENT_PATCH_LINES_PER_FILE: usize = 131_072;

/// Maximum bytes read from or written to one file by the agent patch API.
pub const MAX_AGENT_PATCH_FILE_BYTES: usize = 8 * 1024 * 1024;

/// Maximum bytes read from or written to all files in one agent patch.
pub const MAX_AGENT_PATCH_TOTAL_BYTES: usize = 32 * 1024 * 1024;

/// Maximum bytes in one workspace-relative patch path.
const MAX_AGENT_PATCH_PATH_BYTES: usize = 4 * 1024;

const FILE_WRITE_OUTCOME_UNKNOWN: &str = "workspace file publication outcome is unknown";
pub(crate) const FILE_DELETE_OUTCOME_UNKNOWN: &str = "workspace entry deletion outcome is unknown";

pub fn file_write_outcome_unknown(error: &AppError) -> bool {
    matches!(error, AppError::Internal(message) if message.starts_with(FILE_WRITE_OUTCOME_UNKNOWN))
}

pub fn file_delete_outcome_unknown(error: &AppError) -> bool {
    matches!(error, AppError::Internal(message) if message.starts_with(FILE_DELETE_OUTCOME_UNKNOWN))
}

fn file_write_publication_error(failure: PatchPublicationFailure) -> AppError {
    if file_write_outcome_unknown(&failure.error) {
        failure.error
    } else if failure.published || failure.temporary_cleanup_unconfirmed {
        AppError::Internal(format!(
            "{FILE_WRITE_OUTCOME_UNKNOWN}; published={}, temporary_cleanup_unconfirmed={}; re-read the target before retry",
            failure.published, failure.temporary_cleanup_unconfirmed,
        ))
    } else {
        failure.error
    }
}

fn finish_agent_patch_failure(error: AppError, observation: AgentPatchFailureObservation) -> AgentSessionPatchFailure {
    let uncertain = !observation.unverified_publications.is_empty()
        || !observation.restore_published_unconfirmed.is_empty()
        || !observation.temporary_cleanup_unconfirmed.is_empty();
    let error = if uncertain && !file_write_outcome_unknown(&error) {
        AppError::Internal(format!("{FILE_WRITE_OUTCOME_UNKNOWN}; patch recovery or cleanup is unconfirmed; re-read every target before retry"))
    } else { error };
    AgentSessionPatchFailure { error, observation }
}

/// Maximum bytes in one patch line's text.
const MAX_AGENT_PATCH_LINE_BYTES: usize = 1024 * 1024;

/// Request timeout for remote image fetching (30 seconds).
const REMOTE_IMAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Allowed hosts for remote image fetching.
const ALLOWED_IMAGE_HOSTS: &[&str] = &[
    "github.com",
    "raw.githubusercontent.com",
    "avatars.githubusercontent.com",
    "user-images.githubusercontent.com",
    "camo.githubusercontent.com",
    "objects.githubusercontent.com",
    "repository-images.githubusercontent.com",
];

/// Placeholder SVG returned when remote image fetching fails.
const PLACEHOLDER_SVG: &str = concat!(
    "<svg xmlns=\"http://www.w3.org/2000/svg\" ",
    "width=\"200\" height=\"200\" viewBox=\"0 0 200 200\">",
    "<rect fill=\"#f0f0f0\" width=\"200\" height=\"200\"/>",
    "<text x=\"100\" y=\"96\" text-anchor=\"middle\" ",
    "fill=\"#999\" font-family=\"sans-serif\" font-size=\"14\">",
    "Image Unavailable",
    "</text>",
    "</svg>",
);

/// A bounded, typed patch request for one AgentSession workspace.
///
/// The request deliberately models patch lines instead of accepting an
/// arbitrary unified-diff string. This keeps the wire shape explicit and
/// allows serde to reject fields that are not part of this contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionPatchRequest {
    pub files: Vec<AgentSessionFilePatch>,
}

/// A patch for one workspace-relative file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionFilePatch {
    pub path: String,
    /// Bind to the full previously observed bytes or explicit absence. Omit
    /// only for legacy line-only matching; this is not a filesystem lock.
    #[serde(default, skip_serializing_if = "crate::AgentSessionPatchSource::is_any")]
    pub expected_source: crate::AgentSessionPatchSource,
    pub hunks: Vec<AgentSessionPatchHunk>,
}

/// A line-addressed patch hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionPatchHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<AgentSessionPatchLine>,
}

/// One context, addition, or removal line in a patch hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentSessionPatchLine {
    Context { text: String },
    Add { text: String },
    Remove { text: String },
}

/// Bounded metadata returned after an agent patch is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionPatchResult {
    pub files: Vec<AgentSessionPatchFileResult>,
    pub file_count: usize,
    pub total_bytes_before: u64,
    pub total_bytes_after: u64,
}

/// Bounded metadata for one successfully patched file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionPatchFileResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<crate::WorkspacePathObservation>,
    /// Normalized workspace-relative path; no native absolute path is
    /// returned to the agent.
    pub path: String,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub hunks_applied: usize,
    pub created: bool,
    /// Historic source version; None for creation or legacy receipts.
    #[serde(default)]
    pub source_sha256: Option<String>,
    /// Bytes supplied to successful publication, not a lock on current bytes.
    /// Re-read with expected_sha256 before relying on this version later.
    #[serde(default)]
    pub written_sha256: Option<String>,
}

#[derive(Debug)]
pub struct AgentSessionWriteResult {
    pub created: bool,
    pub workspace_path: Option<crate::WorkspacePathObservation>,
}

#[derive(Default)]
struct WorkspaceInventory {
    files: OnceLock<Vec<WorkspaceFlatFile>>,
    readers: AtomicUsize,
}

/// The slot identity is the publication token. Removing it revokes all scans
/// holding that token without keeping an unbounded per-root generation ledger.
struct WorkspaceInventoryRead<'a> {
    cache: &'a DashMap<String, Arc<WorkspaceInventory>>,
    key: &'a str,
    slot: Arc<WorkspaceInventory>,
}

impl Drop for WorkspaceInventoryRead<'_> {
    fn drop(&mut self) {
        if self.slot.readers.fetch_sub(1, Ordering::AcqRel) == 1 && self.slot.files.get().is_none() {
            self.cache.remove_if(self.key, |_, current| Arc::ptr_eq(current, &self.slot)
                && current.readers.load(Ordering::Acquire) == 0 && current.files.get().is_none());
        }
    }
}

/// A concrete implementation of [`crate::traits::IFileService`].
pub struct FileService {
    user_events: Arc<dyn UserEventSink>,
    /// Allowed root directories for path safety validation.
    allowed_roots: Vec<std::path::PathBuf>,
    /// In-memory cache for `list_workspace_files`, keyed by canonical root.
    workspace_files_cache: DashMap<String, Arc<WorkspaceInventory>>,
    /// Cancellation flags for in-progress ZIP operations, keyed by request_id.
    zip_cancellations: DashMap<String, Arc<AtomicBool>>,
    /// Serializes multi-file AgentSession patch commits within this service.
    /// Individual file operations remain independently usable by the UI, but
    /// one patch must not interleave with another patch's prepare/commit
    /// sequence.
    agent_patch_lock: Arc<tokio::sync::Mutex<()>>,
}

impl FileService {
    pub fn new(user_events: Arc<dyn UserEventSink>, allowed_roots: Vec<std::path::PathBuf>) -> Self {
        Self {
            user_events,
            allowed_roots,
            workspace_files_cache: DashMap::new(),
            zip_cancellations: DashMap::new(),
            agent_patch_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Read a workspace-relative file through an explicit AgentSession
    /// resource binding. The host supplies the resolved native workspace root;
    /// Use the same confined, 8 MiB reader as the Agent text/image tool paths.
    pub async fn read_file_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
    ) -> Result<Option<String>, AppError> {
        scope.require_operation(crate::resource::READ_OPERATION)?;
        let path = scope.resolve_relative_path(relative_path)?;
        let authority = scope.authority();
        tokio::task::spawn_blocking(move || {
            crate::agent_text_read::read_source(&path,&authority,MAX_AGENT_PATCH_FILE_BYTES,&mut 0)
                .map(|source|source.map(|(text,_)|text))
        }).await.map_err(|error|AppError::Internal(format!("workspace text read task failed: {error}")))?
    }

    pub async fn list_workspace_files_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
    ) -> Result<Vec<WorkspaceFlatFile>, AppError> {
        scope.require_operation(crate::resource::READ_OPERATION)?;
        self.list_workspace_files_impl(&scope.workspace_root().to_string_lossy(), &scope.authority())
            .await
    }

    pub async fn get_file_metadata_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
    ) -> Result<FileMetadata, AppError> {
        scope.require_operation(crate::resource::READ_OPERATION)?;
        let path = scope.resolve_relative_path(relative_path)?;
        self.get_file_metadata_impl(&path.to_string_lossy(), &scope.authority())
            .await
    }

    /// Publish complete bytes, returning whether a new file was created.
    /// A failed settlement may require reconciliation; never truncate in place.
    pub async fn write_file_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
        data: &[u8],
    ) -> Result<bool, AppError> {
        self.write_file_with_observation_for_agent_session(scope, relative_path, data)
            .await.map(|result| result.created)
    }

    pub async fn write_file_with_observation_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
        data: &[u8],
    ) -> Result<AgentSessionWriteResult, AppError> {
        scope.require_operation(crate::resource::WRITE_OPERATION)?;
        if data.len() > MAX_AGENT_PATCH_FILE_BYTES {
            return Err(AppError::BadRequest("workspace write exceeds the 8 MiB byte limit".into()));
        }
        let path = scope.resolve_relative_path(relative_path)?;
        let prepared_parent = crate::workspace_write::prepare_parent(&path, scope.workspace_root())?;
        let path = prepared_parent.as_path().to_path_buf();
        let path_owned = path.clone();
        let data_owned = data.to_vec();
        let result = tokio::task::spawn_blocking(move || {
            // Keep the directory handles alive if this future is cancelled;
            // the blocking owner still must finish publication and cleanup.
            let _parent_guard = prepared_parent;
            let existed = match std::fs::symlink_metadata(&path_owned) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(AppError::Internal(format!("cannot inspect workspace write target: {error}")).into()),
            };
            let source = if existed { PublicationSource::Existing } else { PublicationSource::Absent };
            write_file_with_source_sync_atomic(&path_owned, &data_owned, source)?;
            Ok(!existed)
        }).await.map_err(|_| AppError::Internal(format!(
            "{FILE_WRITE_OUTCOME_UNKNOWN}; publication task stopped; re-read the target before retry"
        )))?;
        self.observe_file_publication(scope.owner_id(), &path, data, &scope.workspace_root().to_string_lossy(), &result);
        result.map(|created| AgentSessionWriteResult {
            created,
            workspace_path: crate::WorkspacePathObservation::from_canonical(scope.workspace_root(), &path),
        }).map_err(file_write_publication_error)
    }

    pub async fn remove_entry_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
    ) -> Result<(), AppError> {
        self.remove_entry_with_observation_for_agent_session(scope, relative_path).await.map(|_| ())
    }

    pub async fn remove_entry_with_observation_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
    ) -> Result<Option<crate::WorkspacePathObservation>, AppError> {
        scope.require_operation(crate::resource::DELETE_OPERATION)?;
        if relative_path.is_empty() {
            return Err(AppError::BadRequest("cannot delete the bound workspace root".into()));
        }
        let path = scope.resolve_relative_path(relative_path)?;
        // Inspect the requested entry before canonicalization. Resolving a
        // final symlink/junction and deleting its target changes the operation.
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            let message = format!("cannot inspect workspace deletion target: {error}");
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::NotFound(message)
            } else {
                AppError::Internal(message)
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Forbidden("workspace deletion does not follow symbolic links or junctions".into()));
        }
        let canonical = validate_path_authority(&path.to_string_lossy(), &scope.authority())?;
        let observation = crate::WorkspacePathObservation::from_canonical(scope.workspace_root(), &canonical);
        let workspace = scope.workspace_root().to_string_lossy();
        self.remove_entry_impl(
            scope.owner_id(),
            &canonical.to_string_lossy(),
            &workspace,
            &scope.authority(),
        )
        .await?;
        Ok(observation)
    }

    pub async fn rename_entry_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
        new_name: &str,
    ) -> Result<String, AppError> {
        scope.require_operation(crate::resource::WRITE_OPERATION)?;
        let path = scope.resolve_relative_path(relative_path)?;
        self.rename_entry_impl(&path.to_string_lossy(), new_name, &scope.authority())
            .await
    }

    /// Apply a bounded, typed patch under an AgentSession workspace binding.
    ///
    /// Every target is resolved and authority-checked first. All source files
    /// are read and all hunks are applied in memory before the first write is
    /// attempted, so malformed paths, limits, or hunks cannot partially
    /// modify the workspace. The actual writes reuse the existing
    /// authority-aware read/write/remove paths rather than a legacy gateway or
    /// conversation implementation.
    pub async fn apply_patch_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        request: AgentSessionPatchRequest,
    ) -> Result<AgentSessionPatchResult, AppError> {
        self.apply_patch_with_observation_for_agent_session(scope, request)
            .await.map_err(|failure| failure.error)
    }

    /// Production engines must retain failure observations: Err is not proof
    /// that no files changed. The legacy wrapper above preserves its API.
    pub async fn apply_patch_with_observation_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        request: AgentSessionPatchRequest,
    ) -> Result<AgentSessionPatchResult, AgentSessionPatchFailure> {
        let _patch_guard = self.agent_patch_lock.lock().await;
        scope.require_operation(crate::resource::WRITE_OPERATION)?;
        validate_agent_patch_request_shape(&request)?;

        let authority = scope.authority();
        let workspace_root = std::fs::canonicalize(scope.workspace_root()).map_err(|error| {
            AppError::BadRequest(format!(
                "cannot resolve bound workspace '{}': {error}",
                scope.workspace_root().display()
            ))
        })?;
        let mut prepared = Vec::with_capacity(request.files.len());
        let mut seen_paths: HashSet<PathBuf> = HashSet::with_capacity(request.files.len());
        let mut total_before = 0_u64;
        let mut total_after = 0_u64;

        for (index, file_patch) in request.files.iter().enumerate() {
            let (path, existed) = validate_agent_patch_target(scope, &file_patch.path, &authority)?;
            for previous in &seen_paths {
                if crate::path_safety::patch_targets_overlap(&workspace_root, previous, &path)? {
                    return Err(AppError::BadRequest(format!(
                        "agent patch contains duplicate or nested file target '{}'",
                        file_patch.path
                    )).into());
                }
            }
            seen_paths.insert(path.clone());

            let before = if existed {
                let metadata = std::fs::metadata(&path).map_err(|error| {
                    AppError::Internal(format!(
                        "cannot inspect patch target '{}': {error}",
                        path.display()
                    ))
                })?;
                if metadata.len() > MAX_AGENT_PATCH_FILE_BYTES as u64 {
                    return Err(AppError::BadRequest(format!(
                        "patch target '{}' exceeds the {} byte per-file limit",
                        file_patch.path, MAX_AGENT_PATCH_FILE_BYTES
                    )).into());
                }

                let read_path = path.clone();
                let read_authority = authority.clone();
                let read_limit = MAX_AGENT_PATCH_FILE_BYTES.min(
                    MAX_AGENT_PATCH_TOTAL_BYTES.saturating_sub(total_before as usize),
                );
                tokio::task::spawn_blocking(move || {
                    let mut charged = 0;
                    crate::agent_text_read::read_source(&read_path, &read_authority, read_limit, &mut charged)
                }).await.map_err(|error| AppError::Internal(format!("patch source read failed: {error}")))??
                    .ok_or_else(|| {
                        AppError::Conflict(format!(
                            "patch target '{}' disappeared while it was being read",
                            file_patch.path
                        ))
                    })?
                    .0
                    .into_bytes()
            } else {
                Vec::new()
            };

            file_patch.expected_source.check(existed, &before).map_err(|error| {
                AgentSessionPatchFailure {
                    error: AppError::Conflict(format!("patch target {:?}: {error}", file_patch.path)),
                    observation: AgentPatchFailureObservation {
                        failed_file: Some(index),
                        ..Default::default()
                    },
                }
            })?;
            let before_text = std::str::from_utf8(&before).map_err(|_| {
                AppError::BadRequest(format!(
                    "patch target '{}' is not valid UTF-8 text",
                    file_patch.path
                ))
            })?;
            let after_text = apply_agent_patch_hunks(before_text, &file_patch.hunks)
                .map_err(|error| AppError::BadRequest(format!("patch target {:?}: {error}", file_patch.path)))?;
            let after = after_text.into_bytes();

            if after.len() > MAX_AGENT_PATCH_FILE_BYTES {
                return Err(AppError::BadRequest(format!(
                    "patched file '{}' exceeds the {} byte per-file limit",
                    file_patch.path, MAX_AGENT_PATCH_FILE_BYTES
                )).into());
            }
            total_before = total_before
                .checked_add(before.len() as u64)
                .ok_or_else(|| AppError::BadRequest("patch byte count overflow".to_owned()))?;
            total_after = total_after
                .checked_add(after.len() as u64)
                .ok_or_else(|| AppError::BadRequest("patch byte count overflow".to_owned()))?;
            if total_before > MAX_AGENT_PATCH_TOTAL_BYTES as u64
                || total_after > MAX_AGENT_PATCH_TOTAL_BYTES as u64
            {
                return Err(AppError::BadRequest(format!(
                    "agent patch exceeds the {} byte total limit",
                    MAX_AGENT_PATCH_TOTAL_BYTES
                )).into());
            }

            let relative_path = rel_to_api_string(path.strip_prefix(&workspace_root).map_err(|_| {
                AppError::Forbidden(format!(
                    "patch target '{}' is outside the bound workspace",
                    file_patch.path
                ))
            })?);

            prepared.push(PreparedAgentPatchFile {
                path,
                relative_path,
                before,
                after,
                existed,
                hunks_applied: file_patch.hunks.len(),
            });
        }

        // No write occurs above this point. If an I/O failure happens during
        // the commit phase, attempt to restore existing touched files and
        // retain new creations. Return every restoration/retention observation.
        let workspace = scope.workspace_root().to_string_lossy().into_owned();
        let mut applied = Vec::with_capacity(prepared.len());
        for (index, file) in prepared.iter().enumerate() {
            if let Err(error) = self
                .verify_agent_patch_precondition(file, &authority)
                .await
            {
                let mut observation = AgentPatchFailureObservation {
                    failed_file: Some(index), published: applied.clone(), ..Default::default()
                };
                self.rollback_agent_patch_files(
                    scope,
                    &authority,
                    &workspace,
                    &prepared,
                    &applied,
                    &mut observation,
                )
                .await;
                return Err(finish_agent_patch_failure(error, observation));
            }
            let write_result = self
                .write_agent_patch_file(
                    scope.owner_id(),
                    file,
                    &file.after,
                    file.existed.then_some(file.before.as_slice()),
                    &workspace,
                    &authority,
                )
                .await;

            if let Err(failure) = write_result {
                let mut observation = AgentPatchFailureObservation::failed_publication(index, &mut applied, &failure);
                self.rollback_agent_patch_files(scope, &authority, &workspace, &prepared, &applied, &mut observation)
                    .await;
                return Err(finish_agent_patch_failure(failure.error, observation));
            }
            applied.push(index);
        }

        Ok(AgentSessionPatchResult {
            file_count: prepared.len(),
            files: prepared
                .into_iter()
                .map(|file| AgentSessionPatchFileResult {
                    workspace_path: crate::WorkspacePathObservation::from_canonical(&workspace_root, &file.path),
                    path: file.relative_path,
                    bytes_before: file.before.len() as u64,
                    bytes_after: file.after.len() as u64,
                    hunks_applied: file.hunks_applied,
                    created: !file.existed,
                    source_sha256: file.existed.then(|| format!("{:x}", Sha256::digest(&file.before))),
                    written_sha256: Some(format!("{:x}", Sha256::digest(&file.after))),
                })
                .collect(),
            total_bytes_before: total_before,
            total_bytes_after: total_after,
        })
    }

    async fn verify_agent_patch_precondition(
        &self,
        file: &PreparedAgentPatchFile,
        authority: &PathAuthority,
    ) -> Result<(), AppError> {
        self.verify_agent_patch_precondition_with_hooks(file, authority, || {}, || {}).await
    }

    async fn verify_agent_patch_precondition_with_hooks(
        &self,
        file: &PreparedAgentPatchFile,
        authority: &PathAuthority,
        before_open: impl FnOnce(),
        after_read: impl FnOnce(),
    ) -> Result<(), AppError> {
        let path = file.path.to_string_lossy();
        match std::fs::symlink_metadata(file.path.as_path()) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(AppError::Conflict(format!(
                        "patch target '{}' became a symbolic link; retry from a fresh read",
                        file.relative_path
                    )));
                }
                if !metadata.is_file() {
                    return Err(AppError::Conflict(format!(
                        "patch target '{}' is no longer a regular file",
                        file.relative_path
                    )));
                }
                let canonical = validate_path_authority(&path, authority)?;
                if canonical != file.path {
                    return Err(AppError::Conflict(format!(
                        "patch target '{}' changed identity; retry from a fresh read",
                        file.relative_path
                    )));
                }
                let current = crate::agent_text_read::read_source_bytes_with_hooks(
                    &file.path, authority, MAX_AGENT_PATCH_FILE_BYTES, &mut 0, before_open, after_read,
                )?;
                if current.is_none_or(|(bytes, _, canonical)| bytes != file.before || canonical != file.path) {
                    return Err(AppError::Conflict(format!(
                        "patch target '{}' changed while the patch was being prepared",
                        file.relative_path
                    )));
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !file.existed => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(AppError::Conflict(format!(
                    "patch target '{}' disappeared while the patch was being prepared",
                    file.relative_path
                )))
            }
            Err(error) => Err(AppError::Internal(format!(
                "cannot inspect patch target '{}': {error}",
                file.relative_path
            ))),
        }
    }

    async fn rollback_agent_patch_files(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        authority: &PathAuthority,
        workspace: &str,
        files: &[PreparedAgentPatchFile],
        applied: &[usize],
        observation: &mut AgentPatchFailureObservation,
    ) {
        for index in applied.iter().rev().copied() {
            let file = &files[index];
            if file.existed {
                if !current_file_matches(&file.path, &file.after, authority) {
                    observation.skipped_changed_or_unreadable.push(index);
                    continue;
                }
                let result = self
                    .write_agent_patch_file(
                        scope.owner_id(),
                        file,
                        &file.before,
                        Some(&file.after),
                        workspace,
                        authority,
                    )
                    .await;
                match result {
                    Ok(_) => observation.restored.push(index),
                    Err(failure) => {
                        if failure.published {
                            observation.restore_published_unconfirmed.push(index);
                            if !failure.content_verified && !observation.unverified_publications.contains(&index) {
                                observation.unverified_publications.push(index);
                            }
                        } else {
                            observation.rollback_failed.push(index);
                        }
                        if failure.temporary_cleanup_unconfirmed
                            && !observation.temporary_cleanup_unconfirmed.contains(&index)
                        {
                            observation.temporary_cleanup_unconfirmed.push(index);
                        }
                    }
                }
            } else {
                // No portable compare-and-unlink primitive. A prior content
                // check does not justify deleting a concurrent replacement.
                // Retain and report; this is deliberately not an atomic undo.
                observation.retained_created.push(index);
            }
        }
    }

    async fn write_agent_patch_file(
        &self,
        owner_id: &str,
        file: &PreparedAgentPatchFile,
        data: &[u8],
        expected: Option<&[u8]>,
        workspace: &str,
        authority: &PathAuthority,
    ) -> Result<bool, PatchPublicationFailure> {
        let path = file.path.to_string_lossy();
        if has_traversal(&path) {
            return Err(AppError::BadRequest(format!(
                "path '{}' contains invalid traversal patterns",
                path
            )).into());
        }
        let prepared_parent = match authority {
            PathAuthority::Workspace(root) => Some(crate::workspace_write::prepare_parent(&file.path, root)?),
            _ => None,
        };
        let canonical = match &prepared_parent {
            Some(parent) => parent.as_path().to_path_buf(),
            None => validate_path_for_write_authority(&path, authority)?,
        };
        if canonical != file.path {
            return Err(AppError::Conflict(format!(
                "patch target '{}' changed identity before publication",
                file.relative_path
            )).into());
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&canonical)
            && metadata.file_type().is_symlink()
        {
            return Err(AppError::Conflict(format!(
                "patch target '{}' is a symbolic link",
                file.relative_path
            )).into());
        }
        let result = write_file_sync_atomic(&canonical, data, expected);
        self.observe_file_publication(owner_id, &canonical, data, workspace, &result);
        result?;
        Ok(true)
    }

    fn observe_file_publication<T>(
        &self, owner_id: &str, path: &Path, data: &[u8], workspace: &str,
        result: &Result<T, PatchPublicationFailure>,
    ) {
        if result.as_ref().map_or_else(|failure| failure.published && failure.content_verified, |_| true) {
            self.emit_content_update(owner_id, path, data, workspace);
        } else if result.as_ref().is_err_and(|failure| failure.published || failure.temporary_cleanup_unconfirmed || file_write_outcome_unknown(&failure.error)) {
            // The expected buffer is not evidence of what reached disk. Revoke
            // inventories without advertising unverified bytes as new content.
            self.invalidate_caches_for_path(path);
        }
    }

    fn emit_content_update(
        &self,
        owner_id: &str,
        canonical: &Path,
        data: &[u8],
        workspace: &str,
    ) {
        let workspace_path = Path::new(workspace);
        let relative_path = rel_to_api_string(
            canonical
                .strip_prefix(
                    std::fs::canonicalize(workspace_path)
                        .unwrap_or_else(|_| workspace_path.to_path_buf()),
                )
                .unwrap_or(canonical),
        );
        let content = String::from_utf8(data.to_vec()).ok();
        let event = ContentUpdateEvent {
            file_path: canonical.to_string_lossy().into_owned(),
            content,
            workspace: workspace.to_owned(),
            relative_path,
            operation: ContentUpdateOperation::Write,
        };
        let payload = serde_json::to_value(&event).unwrap_or_default();
        if let Ok(canonical_ws) = std::fs::canonicalize(workspace_path) {
            self.invalidate_cache(&canonical_ws.to_string_lossy());
        }
        self.user_events
            .send_to_user(owner_id, WebSocketMessage::new("fileStream.contentUpdate", payload));
    }

    /// Invalidate the workspace files cache for a given root.
    /// Called when file changes are detected.
    pub fn invalidate_cache(&self, root: &str) {
        self.workspace_files_cache.remove(root);
    }

    /// A native change can affect inventories above or below a watched path.
    /// Use the captured native path: deleted/renamed entries cannot be resolved
    /// again, and resolving a replacement would invalidate a different target.
    pub(crate) fn invalidate_caches_for_path(&self, changed: &Path) {
        self.workspace_files_cache.retain(|root, _| {
            let root = Path::new(root);
            !changed.starts_with(root) && !root.starts_with(changed)
        });
    }

    /// Get the allowed root references for path validation.
    fn allowed_roots_refs(&self) -> Vec<&Path> {
        self.allowed_roots.iter().map(|p| p.as_path()).collect()
    }

    /// The default [`PathAuthority`] for the non-scoped trait methods: confine
    /// to the service's construction-time `allowed_roots`, optionally widened
    /// by a request-scoped `extra` root. This reproduces the historical
    /// `allowed_roots ∪ extra_root` behaviour exactly, so the non-scoped
    /// methods (UI file routes, internal callers) are byte-for-byte unchanged.
    fn base_authority(&self, extra: Option<&Path>) -> PathAuthority {
        let mut roots = self.allowed_roots.clone();
        if let Some(extra) = extra {
            roots.push(extra.to_path_buf());
        }
        PathAuthority::Confined(roots)
    }

    /// Whether a (possibly non-existent) `path` textually falls under the given
    /// authority — used by the read fallback to distinguish "allowed but not
    /// found" (→ `Ok(None)`) from "forbidden" (→ error). `Unrestricted` always
    /// qualifies; `Confined` checks whether the path textually starts with one
    /// of the (canonicalized) confining roots.
    fn path_uses_authority(&self, path: &Path, authority: &PathAuthority) -> bool {
        match authority {
            PathAuthority::Unrestricted => true,
            PathAuthority::Confined(roots) => {
                let candidate = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    match std::env::current_dir() {
                        Ok(current_dir) => current_dir.join(path),
                        Err(_) => path.to_path_buf(),
                    }
                };
                roots
                    .iter()
                    .filter_map(|root| std::fs::canonicalize(root).ok())
                    .any(|root| candidate.starts_with(root))
            }
            PathAuthority::Workspace(root) => {
                let candidate = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    match std::env::current_dir() {
                        Ok(current_dir) => current_dir.join(path),
                        Err(_) => path.to_path_buf(),
                    }
                };
                crate::path_safety::validate_workspace_candidate(&candidate, root).is_ok()
            }
        }
    }

    // -- Authority-aware cores (shared by the non-scoped + `*_scoped` trait
    //    methods). The only difference between the two is the [`PathAuthority`]
    //    passed in; the I/O below is identical, so it lives here once. --

    async fn get_files_by_dir_impl(
        &self,
        dir: &str,
        root: &str,
        authority: &PathAuthority,
    ) -> Result<Vec<DirOrFile>, AppError> {
        self.get_files_by_dir_with_hooks(dir, root, authority, || {}, |_| {}).await
    }

    async fn get_files_by_dir_with_hooks(
        &self, dir: &str, root: &str, authority: &PathAuthority,
        before_tree: impl FnOnce() + Send + 'static,
        before_children: impl FnMut(&Path) + Send + 'static,
    ) -> Result<Vec<DirOrFile>, AppError> {
        let canonical_dir = validate_path_authority(dir, authority)?;
        let canonical_root = validate_path_authority(root, authority)?;
        let authority = authority.clone();
        tokio::task::spawn_blocking(move || {
            before_tree();
            build_dir_tree_with_hook(&canonical_dir, &canonical_root, &authority, before_children)
        }).await.map_err(|error| AppError::Internal(format!("directory listing task failed: {error}")))?
    }

    async fn list_workspace_files_impl(
        &self,
        root: &str,
        authority: &PathAuthority,
    ) -> Result<Vec<WorkspaceFlatFile>, AppError> {
        self.list_workspace_files_with_hook(root, authority, || async {}).await
    }

    async fn list_workspace_files_with_hook<Fut: std::future::Future<Output = ()>>(
        &self, root: &str, authority: &PathAuthority, mut after_scan: impl FnMut() -> Fut,
    ) -> Result<Vec<WorkspaceFlatFile>, AppError> {
        let canonical_root = validate_path_authority(root, authority)?;
        let cache_key = canonical_root.to_string_lossy().into_owned();

        // One re-read is allowed only after an observed invalidation. A root
        // that keeps changing must return a conflict, never a stale cache hit.
        for _ in 0..2 {
            let slot = {
                let entry = self.workspace_files_cache.entry(cache_key.clone())
                    .or_insert_with(|| Arc::new(WorkspaceInventory::default()));
                entry.readers.fetch_add(1, Ordering::Relaxed);
                entry.value().clone()
            };
            let _read = WorkspaceInventoryRead { cache: &self.workspace_files_cache, key: &cache_key, slot: slot.clone() };
            if let Some(files) = slot.files.get() { return Ok(files.clone()); }
            let root_owned = canonical_root.clone();
            let files = tokio::task::spawn_blocking(move || list_workspace_files_sync(&root_owned))
                .await
                .map_err(|e| AppError::Internal(format!("workspace file listing task failed: {e}")))??;
            after_scan().await;
            // Hold the map guard through publication so invalidation cannot
            // fall between the identity check and committing the cached data.
            if let Some(current) = self.workspace_files_cache.get(&cache_key)
                && Arc::ptr_eq(current.value(), &slot)
            {
                return Ok(slot.files.get_or_init(|| files).clone());
            }
        }
        Err(AppError::Conflict("workspace changed repeatedly during file listing; request a fresh inventory".into()))
    }

    async fn get_file_metadata_impl(
        &self,
        path: &str,
        authority: &PathAuthority,
    ) -> Result<FileMetadata, AppError> {
        self.get_file_metadata_with_hooks(path, authority, || {}, || {}).await
    }

    async fn get_file_metadata_with_hooks(
        &self, path: &str, authority: &PathAuthority,
        before_read: impl FnOnce() + Send + 'static,
        after_read: impl FnOnce() + Send + 'static,
    ) -> Result<FileMetadata, AppError> {
        let canonical = validate_path_authority(path, authority)?;
        let authority = authority.clone();
        let result = tokio::task::spawn_blocking(move || {
            before_read();
            let result = crate::workspace_read_dir::metadata(&canonical, &authority);
            after_read();
            let observed = result?.ok_or_else(|| AppError::NotFound("workspace metadata target disappeared".into()))?;
            if observed.metadata.file_type().is_symlink() {
                return Err(AppError::Conflict("workspace metadata target changed to a symbolic link".into()));
            }
            Ok(file_metadata_from_observation(&observed.canonical, &observed.metadata))
        })
            .await
            .map_err(|e| AppError::Internal(format!("file metadata task failed: {e}")))??;
        Ok(result)
    }

    async fn read_file_impl(
        &self,
        path: &str,
        authority: &PathAuthority,
    ) -> Result<Option<String>, AppError> {
        if has_traversal(path) {
            return Err(AppError::BadRequest(format!(
                "path '{}' contains invalid traversal patterns",
                path
            )));
        }

        let canonical = match validate_path_authority(path, authority) {
            Ok(c) => c,
            Err(err) => {
                // Path does not exist yet but WOULD be within authority → "not
                // found" rather than "forbidden" (matches the historical
                // read fallback semantics).
                if matches!(err, AppError::BadRequest(_))
                    && validate_path_for_write_authority(path, authority).is_ok()
                {
                    return Ok(None);
                }
                if matches!(err, AppError::BadRequest(_)) && self.path_uses_authority(Path::new(path), authority) {
                    return Ok(None);
                }
                return Err(err);
            }
        };

        tokio::task::spawn_blocking(move || read_file_sync(&canonical))
            .await
            .map_err(|e| AppError::Internal(format!("read file task failed: {e}")))?
    }

    async fn write_file_impl(
        &self,
        owner_id: &str,
        path: &str,
        data: &[u8],
        workspace: &str,
        authority: &PathAuthority,
    ) -> Result<bool, AppError> {
        if has_traversal(path) {
            return Err(AppError::BadRequest(format!(
                "path '{}' contains invalid traversal patterns",
                path
            )));
        }

        let canonical = validate_path_for_write_authority(path, authority)?;

        let path_owned = canonical.clone();
        let data_owned = data.to_vec();
        tokio::task::spawn_blocking(move || write_file_sync(&path_owned, &data_owned))
            .await
            .map_err(|e| AppError::Internal(format!("write file task failed: {e}")))??;

        self.emit_content_update(owner_id, &canonical, data, workspace);
        Ok(true)
    }

    async fn remove_entry_impl(
        &self,
        owner_id: &str,
        path: &str,
        workspace: &str,
        authority: &PathAuthority,
    ) -> Result<(), AppError> {
        if has_traversal(path) {
            return Err(AppError::BadRequest(format!(
                "path '{}' contains invalid traversal patterns",
                path
            )));
        }

        let canonical = validate_path_authority(path, authority)?;

        let path_owned = canonical.clone();
        let delete_authority = authority.clone();
        let removed = tokio::task::spawn_blocking(move || remove_entry_scoped_sync_with_hooks(
            &path_owned, &delete_authority, || {}, || {},
        ))
            .await
            .map_err(|e| AppError::Internal(format!(
                "{FILE_DELETE_OUTCOME_UNKNOWN}; removal task stopped: {e}; inspect the remaining tree before retry"
            )))
            .and_then(|result| result);
        if let Err(error) = removed {
            // Recursive removal may already have changed descendants. Diagnostic
            // reads must not return a listing cached before that attempt.
            if let Ok(root) = std::fs::canonicalize(workspace) {
                self.invalidate_cache(&root.to_string_lossy());
            }
            return Err(error);
        }

        let workspace_path = Path::new(workspace);
        let relative_path = rel_to_api_string(
            canonical
                .strip_prefix(std::fs::canonicalize(workspace_path).unwrap_or_else(|_| workspace_path.to_path_buf()))
                .unwrap_or(&canonical),
        );

        let event = ContentUpdateEvent {
            file_path: canonical.to_string_lossy().into_owned(),
            content: None,
            workspace: workspace.to_owned(),
            relative_path,
            operation: ContentUpdateOperation::Delete,
        };
        let payload = serde_json::to_value(&event).unwrap_or_default();
        let msg = WebSocketMessage::new("fileStream.contentUpdate", payload);
        if let Ok(canonical_ws) = std::fs::canonicalize(workspace_path) {
            self.invalidate_cache(&canonical_ws.to_string_lossy());
        }
        self.user_events.send_to_user(owner_id, msg);

        Ok(())
    }

    async fn rename_entry_impl(
        &self,
        path: &str,
        new_name: &str,
        authority: &PathAuthority,
    ) -> Result<String, AppError> {
        if has_traversal(path) {
            return Err(AppError::BadRequest(format!(
                "path '{}' contains invalid traversal patterns",
                path
            )));
        }

        if new_name.contains('/') || new_name.contains('\\') {
            return Err(AppError::BadRequest(format!(
                "new name '{}' must not contain path separators",
                new_name
            )));
        }

        if is_unsafe_path_segment(new_name) {
            return Err(AppError::BadRequest(format!(
                "new name '{}' is not a valid file name",
                new_name
            )));
        }

        let canonical = validate_path_authority(path, authority)?;

        let new_name_owned = new_name.to_owned();
        let path_owned = canonical;
        let new_path: PathBuf = tokio::task::spawn_blocking(move || rename_entry_sync(&path_owned, &new_name_owned))
            .await
            .map_err(|e| AppError::Internal(format!("rename entry task failed: {e}")))??;

        Ok(new_path.to_string_lossy().into_owned())
    }

}

struct PreparedAgentPatchFile {
    path: PathBuf,
    relative_path: String,
    before: Vec<u8>,
    after: Vec<u8>,
    existed: bool,
    hunks_applied: usize,
}

fn validate_agent_patch_request_shape(request: &AgentSessionPatchRequest) -> Result<(), AppError> {
    if request.files.is_empty() {
        return Err(AppError::BadRequest(
            "agent patch must contain at least one file".to_owned(),
        ));
    }
    if request.files.len() > MAX_AGENT_PATCH_FILES {
        return Err(AppError::BadRequest(format!(
            "agent patch contains {} files; maximum is {}",
            request.files.len(),
            MAX_AGENT_PATCH_FILES
        )));
    }

    let mut total_patch_text = 0_usize;
    for file in &request.files {
        file.expected_source.validate()?;
        if file.path.trim().is_empty() {
            return Err(AppError::BadRequest(
                "agent patch file path must not be empty".to_owned(),
            ));
        }
        if file.path.as_bytes().len() > MAX_AGENT_PATCH_PATH_BYTES {
            return Err(AppError::BadRequest(format!(
                "agent patch file path exceeds the {} byte limit",
                MAX_AGENT_PATCH_PATH_BYTES
            )));
        }
        if file.hunks.is_empty() {
            return Err(AppError::BadRequest(format!(
                "agent patch file '{}' must contain at least one hunk",
                file.path
            )));
        }
        if file.hunks.len() > MAX_AGENT_PATCH_HUNKS_PER_FILE {
            return Err(AppError::BadRequest(format!(
                "agent patch file '{}' contains too many hunks; maximum is {}",
                file.path, MAX_AGENT_PATCH_HUNKS_PER_FILE
            )));
        }

        for hunk in &file.hunks {
            if hunk.lines.is_empty() {
                return Err(AppError::BadRequest(format!(
                    "agent patch file '{}' contains an empty hunk",
                    file.path
                )));
            }
            if hunk.lines.len() > MAX_AGENT_PATCH_LINES_PER_HUNK {
                return Err(AppError::BadRequest(format!(
                    "agent patch file '{}' contains a hunk with too many lines; maximum is {}",
                    file.path, MAX_AGENT_PATCH_LINES_PER_HUNK
                )));
            }
            if hunk.old_lines > MAX_AGENT_PATCH_LINES_PER_FILE
                || hunk.new_lines > MAX_AGENT_PATCH_LINES_PER_FILE
                || hunk.old_start > MAX_AGENT_PATCH_LINES_PER_FILE
                || hunk.new_start > MAX_AGENT_PATCH_LINES_PER_FILE
            {
                return Err(AppError::BadRequest(format!(
                    "agent patch file '{}' has a line count or offset beyond the bounded limit",
                    file.path
                )));
            }

            for line in &hunk.lines {
                let text = match line {
                    AgentSessionPatchLine::Context { text }
                    | AgentSessionPatchLine::Add { text }
                    | AgentSessionPatchLine::Remove { text } => text,
                };
                if text.contains(['\n', '\r', '\0']) {
                    return Err(AppError::BadRequest(format!(
                        "agent patch file '{}' contains a line terminator or NUL; supply logical text without CR/LF",
                        file.path
                    )));
                }
                if text.len() > MAX_AGENT_PATCH_LINE_BYTES {
                    return Err(AppError::BadRequest(format!(
                        "agent patch file '{}' contains a line exceeding the {} byte limit",
                        file.path, MAX_AGENT_PATCH_LINE_BYTES
                    )));
                }
                total_patch_text = total_patch_text
                    .checked_add(text.len())
                    .ok_or_else(|| AppError::BadRequest("patch byte count overflow".to_owned()))?;
                if total_patch_text > MAX_AGENT_PATCH_TOTAL_BYTES {
                    return Err(AppError::BadRequest(format!(
                        "agent patch text exceeds the {} byte total limit",
                        MAX_AGENT_PATCH_TOTAL_BYTES
                    )));
                }
            }
        }
    }

    Ok(())
}

fn validate_agent_patch_target(
    scope: &AgentSessionWorkspaceBinding,
    relative_path: &str,
    authority: &PathAuthority,
) -> Result<(PathBuf, bool), AppError> {
    let candidate = scope.resolve_relative_path(relative_path)?;
    let candidate_string = candidate.to_string_lossy();
    let write_candidate = crate::workspace_write::validate_target(&candidate, scope.workspace_root())?;

    // A final symlink is rejected instead of being followed. The parent was
    // canonicalized by the write validator, but following a final symlink
    // would otherwise let a bound write land outside the workspace.
    match std::fs::symlink_metadata(&write_candidate) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(AppError::Forbidden(format!(
                    "patch target '{}' is a symbolic link",
                    relative_path
                )));
            }
            if !metadata.is_file() {
                return Err(AppError::BadRequest(format!(
                    "patch target '{}' is not a regular file",
                    relative_path
                )));
            }
            let canonical = validate_path_authority(&candidate_string, authority)?;
            Ok((canonical, true))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((write_candidate, false)),
        Err(error) => Err(AppError::BadRequest(format!(
            "cannot inspect patch target '{}': {error}",
            relative_path
        ))),
    }
}

fn apply_agent_patch_hunks(
    original: &str,
    hunks: &[AgentSessionPatchHunk],
) -> Result<String, AppError> {
    use crate::agent_patch_lines::{PatchLine, PatchText};
    let source_text = PatchText::parse(original, MAX_AGENT_PATCH_LINES_PER_FILE)?;
    let source = &source_text.lines;

    let mut output = Vec::with_capacity(source.len());
    let mut source_cursor = 0_usize;

    for hunk in hunks {
        let hunk_start = if hunk.old_lines == 0 {
            // A zero-length old range is an insertion AFTER old_start source
            // lines (0 = BOF, source.len() = EOF), as in unified diff. The old
            // before-next-line alias is accepted only when new_start selects
            // the current cursor unambiguously; no text/whitespace guessing.
            if hunk.old_start == source_cursor.saturating_add(1)
                && hunk.new_start == output.len().saturating_add(1)
            { source_cursor } else { hunk.old_start }
        } else {
            hunk.old_start.checked_sub(1).ok_or_else(|| {
                invalid_agent_hunk(hunk, "old_start must be at least 1 for a non-empty hunk")
            })?
        };
        if hunk_start < source_cursor || hunk_start > source.len() {
            return Err(invalid_agent_hunk(hunk, "old_start is outside the source file"));
        }

        output.extend(source[source_cursor..hunk_start].iter().cloned());
        let output_start = output.len();
        let expected_new_start = if hunk.new_lines == 0 {
            // A deletion's zero-length new range is AFTER the surviving
            // prefix, including a deletion in the middle or at EOF.
            // Retain the prior before-next-line spelling for existing clients.
            if hunk.new_start == output_start.saturating_add(1) {
                hunk.new_start
            } else { output_start }
        } else {
            output_start.checked_add(1).ok_or_else(|| {
                AppError::BadRequest("patch output line offset overflow".to_owned())
            })?
        };
        if hunk.new_start != expected_new_start {
            return Err(invalid_agent_hunk(
                hunk,
                "hunks must be ordered and new_start must match the output position",
            ));
        }

        let mut source_position = hunk_start;
        let mut old_consumed = 0_usize;
        let mut new_produced = 0_usize;
        for line in &hunk.lines {
            match line {
                AgentSessionPatchLine::Context { text } => {
                    if source.get(source_position).map(|line| line.text) != Some(text.as_str()) {
                        return Err(agent_patch_line_mismatch(hunk, source_position, "context"));
                    }
                    output.push(source[source_position]);
                    source_position += 1;
                    old_consumed += 1;
                    new_produced += 1;
                }
                AgentSessionPatchLine::Remove { text } => {
                    if source.get(source_position).map(|line| line.text) != Some(text.as_str()) {
                        return Err(agent_patch_line_mismatch(hunk, source_position, "removed"));
                    }
                    source_position += 1;
                    old_consumed += 1;
                }
                AgentSessionPatchLine::Add { text } => {
                    output.push(PatchLine { text: text.as_str(), ending: source_text.preferred_ending });
                    new_produced += 1;
                }
            }

            if old_consumed > hunk.old_lines || new_produced > hunk.new_lines {
                return Err(invalid_agent_hunk(
                    hunk,
                    "hunk line counts are smaller than the supplied lines",
                ));
            }
        }

        if old_consumed != hunk.old_lines || new_produced != hunk.new_lines {
            return Err(invalid_agent_hunk(
                hunk,
                "hunk line counts do not match the supplied context/add/remove lines",
            ));
        }
        source_cursor = source_position;
        if output.len() > MAX_AGENT_PATCH_LINES_PER_FILE {
            return Err(AppError::BadRequest(format!(
                "patched file has too many lines; maximum is {}",
                MAX_AGENT_PATCH_LINES_PER_FILE
            )));
        }
    }

    output.extend(source[source_cursor..].iter().cloned());
    if output.len() > MAX_AGENT_PATCH_LINES_PER_FILE {
        return Err(AppError::BadRequest(format!(
            "patched file has too many lines; maximum is {}",
            MAX_AGENT_PATCH_LINES_PER_FILE
        )));
    }
    source_text.render(&output, MAX_AGENT_PATCH_FILE_BYTES)
}

fn agent_patch_line_mismatch(hunk: &AgentSessionPatchHunk, source_position: usize, kind: &str) -> AppError {
    // Report the exact coordinate without leaking a potentially secret line
    // into durable error logs. Never fuzz-match repeated code or whitespace.
    invalid_agent_hunk(hunk, &format!("{kind} line does not match source line {}; re-read and rebuild the hunk", source_position + 1))
}

fn invalid_agent_hunk(hunk: &AgentSessionPatchHunk, reason: &str) -> AppError {
    AppError::BadRequest(format!(
        "invalid patch hunk (old {}+{}, new {}+{}): {reason}",
        hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
    ))
}

/// Normalize a workspace-relative path to forward-slash separators for the
/// cross-platform JSON/WS API contract (frontend consumers expect '/').
///
/// Component-join never emits a backslash and handles multi-segment relatives
/// correctly across platforms (equivalent to a `\` -> `/` replace, but explicit).
fn rel_to_api_string(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Synchronous directory tree builder (runs in blocking thread pool).
#[cfg(test)]
fn build_dir_tree_sync(dir: &Path, root: &Path) -> Result<Vec<DirOrFile>, AppError> {
    let directory = std::fs::canonicalize(dir).map_err(|error| AppError::BadRequest(error.to_string()))?;
    let root = std::fs::canonicalize(root).map_err(|error| AppError::BadRequest(error.to_string()))?;
    build_dir_tree_with_hook(&directory, &root, &PathAuthority::Workspace(root.clone()), |_| {})
}

fn build_dir_tree_with_hook(dir: &Path, root: &Path, authority: &PathAuthority, mut before_children: impl FnMut(&Path)) -> Result<Vec<DirOrFile>, AppError> {
    let entries = crate::workspace_read_dir::read_directory(dir, authority)?;

    let mut result = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| AppError::Internal(format!("error reading directory entry: {e}")))?;

        let path = entry.path();
        if path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| relative.components().next())
            .is_some_and(|component| {
                crate::artifact_store::is_workspace_owner_component(component.as_os_str())
            })
        {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|e| AppError::Internal(format!("cannot read metadata for '{}': {e}", path.display())))?;

        let name = entry.file_name().to_string_lossy().into_owned();

        let full_path = path.to_string_lossy().into_owned();
        let relative_path = rel_to_api_string(path.strip_prefix(root).unwrap_or(&path));

        let is_dir = kind.is_dir();

        // For directories, also read their immediate children
        let children = if is_dir {
            before_children(&path);
            read_children_sync(&path, root, authority)?
        } else {
            Vec::new()
        };

        result.push(DirOrFile {
            name,
            full_path,
            relative_path,
            is_dir,
            children,
        });
    }

    // Sort: directories first, then alphabetical
    result.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

    Ok(result)
}

/// Read immediate children of a directory (one level, no grandchildren).
fn read_children_sync(dir: &Path, root: &Path, authority: &PathAuthority) -> Result<Vec<DirOrFile>, AppError> {
    let entries = crate::workspace_read_dir::read_directory(dir, authority)?;

    let mut children = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| AppError::Internal(format!("cannot read child directory entry: {error}")))?;

        let path = entry.path();
        if path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| relative.components().next())
            .is_some_and(|component| {
                crate::artifact_store::is_workspace_owner_component(component.as_os_str())
            })
        {
            continue;
        }
        let is_dir = entry.file_type().map_err(|error| AppError::Internal(format!("cannot classify child directory entry: {error}")))?.is_dir();

        let name = entry.file_name().to_string_lossy().into_owned();

        let full_path = path.to_string_lossy().into_owned();
        let relative_path = rel_to_api_string(path.strip_prefix(root).unwrap_or(&path));

        children.push(DirOrFile {
            name,
            full_path,
            relative_path,
            is_dir,
            children: Vec::new(),
        });
    }

    children.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

    Ok(children)
}

/// Recursively list files under the already validated canonical root. Rule
/// reads and directory enumeration use that root as their only authority.
fn list_workspace_files_sync(root: &Path) -> Result<Vec<WorkspaceFlatFile>, AppError> {
    list_workspace_files_sync_with_hook(root, || {})
}

fn list_workspace_files_sync_with_hook(
    root: &Path,
    before_walk: impl FnOnce(),
) -> Result<Vec<WorkspaceFlatFile>, AppError> {
    list_workspace_files_sync_with_hooks(root, before_walk, |_| {})
}

fn list_workspace_files_sync_with_hooks(
    root: &Path,
    before_walk: impl FnOnce(),
    before_directory: impl FnMut(&Path),
) -> Result<Vec<WorkspaceFlatFile>, AppError> {
    before_walk();
    let mut reasons = BTreeSet::new();
    let mut source_bytes = 0;
    let mut walker = crate::workspace_search_walk::SearchWalk::inventory(
        root, Instant::now(), &mut reasons, before_directory,
    )?;
    let mut files = Vec::new();

    while let Some(entry) = walker.next(&mut source_bytes, &mut reasons) {
        let entry = entry?;
        let Some(kind) = entry.file_type() else { continue; };
        if kind.is_dir() && entry.depth() >= 64 {
            return Err(AppError::Conflict("workspace inventory exceeds the directory depth budget".into()));
        }
        if kind.is_symlink() || !kind.is_file() { continue; }
        if files.len() >= MAX_WORKSPACE_FILES {
            return Err(AppError::Conflict("workspace inventory exceeds the file budget; narrow the directory".into()));
        }
        let path = entry.path();
        let full_path = path.to_str().ok_or_else(|| AppError::BadRequest("workspace inventory path is not valid UTF-8".into()))?.to_owned();
        let name = path.file_name().and_then(|name| name.to_str())
            .ok_or_else(|| AppError::BadRequest("workspace inventory entry has no representable name".into()))?.to_owned();
        let relative = path.strip_prefix(root)
            .map_err(|_| AppError::Forbidden("workspace inventory escaped its root".into()))?;
        let relative_path = rel_to_api_string(relative);

        files.push(WorkspaceFlatFile {
            name,
            full_path,
            relative_path,
        });

    }
    if !reasons.is_empty() {
        return Err(AppError::Conflict(format!("workspace inventory is incomplete: {}", reasons.into_iter().collect::<Vec<_>>().join(", "))));
    }
    Ok(files)
}

/// Validate that a file exists and is within the size limit.
/// Returns `Ok(None)` if the file does not exist.
/// Returns `Ok(Some(()))` if the file is valid for reading.
fn validate_file_for_read(path: &Path) -> Result<Option<()>, AppError> {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(e) => {
            return Err(AppError::Internal(format!(
                "cannot read metadata for '{}': {e}",
                path.display()
            )));
        }
    };

    if metadata.len() > MAX_FILE_SIZE {
        return Err(AppError::BadRequest(format!(
            "file '{}' exceeds 256 MB limit ({} bytes)",
            path.display(),
            metadata.len()
        )));
    }

    if metadata.is_dir() {
        return Err(AppError::BadRequest(format!(
            "path '{}' is a directory; expected a file",
            path.display()
        )));
    }

    Ok(Some(()))
}

/// Read a file as UTF-8 text. Returns `None` if the file does not exist.
/// Rejects files larger than 256 MB.
fn read_file_sync(path: &Path) -> Result<Option<String>, AppError> {
    if validate_file_for_read(path)?.is_none() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| AppError::Internal(format!("cannot read file '{}': {e}", path.display())))?;

    Ok(Some(content))
}

/// Write data to a file synchronously. Creates the file if it does not exist.
/// Returns `true` on success.
fn write_file_sync(path: &Path, data: &[u8]) -> Result<bool, AppError> {
    std::fs::write(path, data)
        .map_err(|e| AppError::Internal(format!("cannot write file '{}': {e}", path.display())))?;
    Ok(true)
}

fn current_file_matches(path: &Path, expected: &[u8], authority: &PathAuthority) -> bool {
    current_file_matches_with_hooks(path, expected, authority, || {}, || {})
}

fn current_file_matches_with_hooks(
    path: &Path, expected: &[u8], authority: &PathAuthority,
    before_open: impl FnOnce(), after_read: impl FnOnce(),
) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return false;
    }
    if metadata.len() > (MAX_AGENT_PATCH_FILE_BYTES as u64) {
        return false;
    }
    crate::agent_text_read::read_source_bytes_with_hooks(
        path, authority, MAX_AGENT_PATCH_FILE_BYTES, &mut 0, before_open, after_read,
    ).is_ok_and(|source| source.is_some_and(|(bytes, _, _)| bytes == expected))
}

/// Atomically publish one already-authorized AgentSession file.
///
/// The temporary file is created beside the target, fully written and synced,
/// and then replaced with a same-filesystem rename. A new file uses a
/// no-clobber publication (handle rename on Windows, hard link elsewhere) so a
/// concurrent creator cannot be overwritten. Existing files use the platform's
/// atomic replacement primitive.
fn write_file_sync_atomic(path: &Path, data: &[u8], expected: Option<&[u8]>) -> Result<(), PatchPublicationFailure> {
    write_file_with_source_sync_atomic(path, data,
        expected.map_or(PublicationSource::Absent, PublicationSource::Matching))
}

#[derive(Clone, Copy)]
enum PublicationSource<'a> {
    Absent,
    Existing,
    Matching(&'a [u8]),
}

#[cfg(windows)]
#[derive(Default)]
struct PublicationProgress {
    published: bool,
    temporary_consumed: bool,
    content_verified: bool,
}

#[cfg(windows)]
struct StagedPublication<'a> {
    owner: &'a crate::windows_cleanup::OwnedFile,
    bytes: &'a [u8],
}

fn write_file_with_source_sync_atomic(path: &Path, data: &[u8], source: PublicationSource<'_>) -> Result<(), PatchPublicationFailure> {
    let parent = path.parent().ok_or_else(|| {
        AppError::BadRequest(format!(
            "patch target '{}' has no parent directory",
            path.display()
        ))
    })?;
    path.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
        AppError::BadRequest(format!(
            "patch target '{}' has no valid file name",
            path.display()
        ))
    })?;
    static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".nomifun-patch-{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    publish_patch_file(path, data, &temporary, source)
}

fn publish_patch_file(path: &Path, data: &[u8], temporary: &Path, source: PublicationSource<'_>) -> Result<(), PatchPublicationFailure> {
    publish_patch_file_with_hooks(path, data, temporary, source, || {}, || Ok(()))
}

fn publish_patch_file_with_hooks(
    path: &Path, data: &[u8], temporary: &Path, source: PublicationSource<'_>,
    after_staging: impl FnOnce(),
    after_publication: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), PatchPublicationFailure> {
    publish_patch_file_with_prepublication_hook(path, data, temporary, source, after_staging, || {}, after_publication,
        #[cfg(windows)] replace_file_windows_native,
    )
}

fn publish_patch_file_with_prepublication_hook(
    path: &Path, data: &[u8], temporary: &Path, source: PublicationSource<'_>,
    after_staging: impl FnOnce(),
    before_replace: impl FnOnce(),
    after_publication: impl FnOnce() -> Result<(), AppError>,
    #[cfg(windows)] native_replace: impl FnOnce(&Path, &Path, &Path) -> std::io::Result<()>,
) -> Result<(), PatchPublicationFailure> {
    #[cfg(windows)]
    if matches!(source, PublicationSource::Absent) {
        return crate::windows_create::publish(path, data, temporary, after_staging, after_publication);
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_GENERIC_WRITE, FILE_SHARE_READ};
        // Retain our newly created object's name and bytes while staging.
        options.access_mode(FILE_GENERIC_WRITE | DELETE).share_mode(FILE_SHARE_READ);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Only a successful create_new gives this operation ownership to clean up.
    let mut file = options.open(temporary).map_err(|error| {
        AppError::Internal(format!(
            "cannot create temporary patch file '{}': {error}",
            temporary.display()
        ))
    })?;
    #[cfg(windows)]
    let mut temporary_owner = Some(crate::windows_cleanup::OwnedFile::capture(&file).map_err(|error| PatchPublicationFailure {
        error: AppError::Internal(format!("cannot retain temporary publication identity: {error}")),
        published: false,
        content_verified: false,
        temporary_cleanup_unconfirmed: true,
    })?);
    let mut published = false;
    let mut temporary_consumed = false;
    #[cfg(windows)]
    let mut content_verified = false;
    #[cfg(not(windows))]
    let content_verified = false;
    let result = (|| -> Result<(), AppError> {
        file.write_all(data).map_err(|error| {
            AppError::Internal(format!(
                "cannot write temporary patch file '{}': {error}",
                temporary.display()
            ))
        })?;
        file.sync_all().map_err(|error| {
            AppError::Internal(format!(
                "cannot sync temporary patch file '{}': {error}",
                temporary.display()
            ))
        })?;
        drop(file);
        after_staging();

        let target_metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(AppError::Conflict(format!(
                        "patch target '{}' changed to a non-regular file",
                        path.display()
                    )));
                }
                Some(metadata)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "cannot inspect patch target '{}': {error}",
                    path.display()
                )));
            }
        };
        if !matches!(source, PublicationSource::Absent) {
            let metadata = target_metadata.ok_or_else(|| AppError::Conflict(format!(
                "patch target '{}' disappeared before publication", path.display()
            )))?;
            #[cfg(not(windows))]
            {
                // Agent publication holds PreparedParent through this call.
                // Windows checks through a handle that denies write sharing.
                let source_authority = PathAuthority::Confined(vec![path.parent().expect("publication has a parent").to_path_buf()]);
                if let PublicationSource::Matching(expected) = source
                    && !current_file_matches(path, expected, &source_authority)
                {
                    return Err(AppError::Conflict(format!(
                        "patch target '{}' changed before publication; re-read before retry", path.display()
                    )));
                }
            }
            #[cfg(not(windows))]
            std::fs::set_permissions(&temporary, metadata.permissions()).map_err(|error| {
                AppError::Internal(format!(
                    "cannot preserve patch target permissions '{}': {error}",
                    path.display()
                ))
            })?;
            #[cfg(windows)]
            let _ = metadata;
            before_replace();
            let expected = match source { PublicationSource::Matching(bytes) => Some(bytes), _ => None };
            #[cfg(not(windows))]
            let replacement = replace_file_path(&temporary, path, &mut published, expected);
            #[cfg(not(windows))]
            { temporary_consumed = published; }
            #[cfg(windows)]
            let replacement = {
                let mut progress = PublicationProgress::default();
                let result = replace_file_path_windows_verified(temporary, path, &mut progress, expected,
                    StagedPublication { owner: temporary_owner.as_ref().expect("staged ownership is live"), bytes: data },
                    native_replace, |path, owner| owner.remove(path));
                published = progress.published;
                temporary_consumed = progress.temporary_consumed;
                content_verified = progress.content_verified;
                result
            };
            #[cfg(windows)]
            if temporary_consumed { temporary_owner = None; }
            replacement?;
        } else {
            // Creation intent is fixed during preparation. Never reinterpret
            // a newly appeared target as an existing-file replacement.
            if target_metadata.is_some() {
                return Err(AppError::Conflict(format!(
                    "patch target '{}' appeared before publication", path.display()
                )));
            }
            // hard_link is intentionally used for the create case: unlike
            // rename, it fails rather than replacing a target that appeared
            // after the precondition check.
            std::fs::hard_link(&temporary, path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    AppError::Conflict(format!(
                        "patch target '{}' appeared during publication",
                        path.display()
                    ))
                } else {
                    AppError::Internal(format!(
                        "cannot publish new patch target '{}': {error}",
                        path.display()
                    ))
                }
            })?;
            published = true;
            #[cfg(windows)]
            let cleanup = temporary_owner.as_ref().expect("staged file ownership is live").remove(temporary);
            #[cfg(not(windows))]
            let cleanup = std::fs::remove_file(temporary);
            cleanup.map_err(|error| {
                AppError::Internal(format!(
                    "cannot remove temporary patch file '{}': {error}",
                    temporary.display()
                ))
            })?;
            temporary_consumed = true;
            #[cfg(windows)]
            { temporary_owner = None; }
        }
        after_publication()?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            let directory = std::fs::File::open(parent).map_err(|error| {
                AppError::Internal(format!("cannot open patch target directory for sync: {error}"))
            })?;
            directory.sync_all().map_err(|error| {
                AppError::Internal(format!(
                    "cannot sync patch target directory '{}': {error}",
                    parent.display()
                ))
            })?;
        }
        Ok(())
    })();
    result.map_err(|error| {
        // Once the source was consumed, this filename is no longer ours.
        // A later failure must not unlink a concurrently created replacement.
        #[cfg(windows)]
        let cleanup = || temporary_owner.as_ref().expect("unconsumed file ownership is live").remove(temporary);
        #[cfg(not(windows))]
        let cleanup = || std::fs::remove_file(temporary);
        let temporary_cleanup_unconfirmed = !temporary_consumed && match cleanup() {
            Ok(()) => false,
            #[cfg(not(windows))]
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => true,
        };
        PatchPublicationFailure { error, published, content_verified, temporary_cleanup_unconfirmed }
    })
}

#[cfg(not(windows))]
fn replace_file_path(source: &Path, target: &Path, published: &mut bool, _expected: Option<&[u8]>) -> Result<(), AppError> {
    std::fs::rename(source, target).map_err(|error| {
        AppError::Internal(format!(
            "cannot atomically replace patch target '{}': {error}",
            target.display()
        ))
    })?;
    *published = true;
    Ok(())
}

#[cfg(windows)]
fn replace_file_windows_native(source: &Path, target: &Path, backup: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    let wide = |path: &Path| path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let source = wide(source);
    let target = wide(target);
    let backup = wide(backup);
    // SAFETY: buffers are NUL-terminated and live throughout the native call.
    // Do not ignore ACL merge failures.
    if unsafe {
        ReplaceFileW(target.as_ptr(), source.as_ptr(), backup.as_ptr(),
            0, std::ptr::null(), std::ptr::null())
    } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(windows, test))]
fn replace_file_path_windows_with(
    source: &Path, target: &Path, published: &mut bool,
    expected: Option<&[u8]>,
    replace: impl FnOnce(&Path, &Path, &Path) -> std::io::Result<()>,
    cleanup: impl FnOnce(&Path, &crate::windows_cleanup::OwnedFile) -> std::io::Result<()>,
) -> Result<(), AppError> {
    // Native fault-injection tests capture the source before their callback.
    // Production supplies the identity recorded at create_new plus caller bytes.
    let mut file = std::fs::File::open(source).map_err(|error| AppError::Internal(error.to_string()))?;
    let owner = crate::windows_cleanup::OwnedFile::capture(&file).map_err(|error| AppError::Internal(error.to_string()))?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file).take(MAX_AGENT_PATCH_FILE_BYTES as u64 + 1).read_to_end(&mut bytes)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    if bytes.len() > MAX_AGENT_PATCH_FILE_BYTES { return Err(AppError::BadRequest("staged test source exceeds byte limit".into())); }
    drop(file);
    let mut progress = PublicationProgress::default();
    let result = replace_file_path_windows_verified(source, target, &mut progress, expected,
        StagedPublication { owner: &owner, bytes: &bytes }, replace, cleanup);
    *published = progress.published;
    result
}

#[cfg(windows)]
fn replace_file_path_windows_verified(
    source: &Path, target: &Path, progress: &mut PublicationProgress,
    expected: Option<&[u8]>, staged: StagedPublication<'_>,
    replace: impl FnOnce(&Path, &Path, &Path) -> std::io::Result<()>,
    cleanup: impl FnOnce(&Path, &crate::windows_cleanup::OwnedFile) -> std::io::Result<()>,
) -> Result<(), AppError> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    // Keep ReplaceFile's native ACL/stream merge. Require write access even
    // though its target handle only needs read/delete access. Deny other
    // writers from the final check through the native call; existing readers
    // remain compatible when they grant write/delete sharing.
    let mut access = std::fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(target)
        .map_err(|error| AppError::Internal(format!("cannot open replacement target: {error}")))?;
    let metadata = access.metadata().map_err(|error| AppError::Internal(format!("cannot inspect replacement target: {error}")))?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(AppError::Conflict("replacement target changed to a link or non-regular file".into()));
    }
    if let Some(expected) = expected {
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut access).take(expected.len() as u64 + 1).read_to_end(&mut bytes)
            .map_err(|error| AppError::Internal(format!("cannot verify replacement target: {error}")))?;
        if bytes != expected {
            return Err(AppError::Conflict("patch target changed before publication; re-read before retry".into()));
        }
    }
    let original_owner = crate::windows_cleanup::OwnedFile::capture(&access)
        .map_err(|error| AppError::Internal(format!("cannot retain replacement target identity: {error}")))?;
    let mut staged_file = staged.owner.open_for_verification(source)
        .map_err(|_| AppError::Conflict("staged publication source changed identity or is unreadable".into()))?;
    if !publication_bytes_match(&mut staged_file, staged.bytes)
        .map_err(|_| AppError::Conflict("staged publication bytes could not be verified".into()))?
    {
        return Err(AppError::Conflict("staged publication bytes changed before replacement".into()));
    }
    staged_file.set_permissions(metadata.permissions())
        .map_err(|error| AppError::Internal(format!("cannot preserve replacement permissions: {error}")))?;
    let backup = source.with_extension(format!("{}.backup", nomifun_common::generate_id()));
    if backup.try_exists().map_err(|error| AppError::Internal(error.to_string()))? {
        return Err(AppError::Conflict("replacement backup already exists".into()));
    }
    // ReplaceFile opens the source with no sharing. Close the read guard only
    // for that call, then verify the recorded object and bytes before cleanup.
    drop(staged_file);
    let replaced = replace(source, target, &backup);
    let error = match replaced {
        Ok(()) => {
            progress.published = true;
            let unverified = |reason: &str| AppError::Internal(format!(
                "{FILE_WRITE_OUTCOME_UNKNOWN}; published source or bytes are unverified ({reason}); retain original backup and reconcile before retry"
            ));
            let mut published_file = staged.owner.open_for_verification(target).map_err(|error| unverified(&error.to_string()))?;
            progress.temporary_consumed = true;
            if !publication_bytes_match(&mut published_file, staged.bytes).map_err(|error| unverified(&error.to_string()))? {
                return Err(unverified("intended bytes do not match"));
            }
            progress.content_verified = true;
            drop(access);
            return cleanup(&backup, &original_owner).map_err(|_| AppError::Internal(format!(
                "{FILE_WRITE_OUTCOME_UNKNOWN}; original backup cleanup is unconfirmed; re-read the target before retry"
            )));
        }
        Err(error) => error,
    };
    drop(access);
    // ReplaceFile's partial failure 1177 can leave the original at backup.
    // Restore only into an absent target, without replacing a concurrent file.
    if matches!(error.raw_os_error(), Some(1176 | 1177)) {
        if target.try_exists().ok() == Some(false)
            && original_owner.restore(&backup, target).is_ok()
        {
            return Err(AppError::Internal(format!("replacement failed; original restored: {error}")));
        }
        return Err(AppError::Internal(format!(
            "{FILE_WRITE_OUTCOME_UNKNOWN}; replacement failed ({error}); retain recovery files and re-read before retry"
        )));
    }
    Err(AppError::Internal(format!("cannot replace workspace file: {error}")))
}

#[cfg(windows)]
fn publication_bytes_match(file: &mut std::fs::File, expected: &[u8]) -> std::io::Result<bool> {
    let mut bytes = Vec::new();
    std::io::Read::by_ref(file).take(expected.len() as u64 + 1).read_to_end(&mut bytes)?;
    Ok(bytes == expected)
}

/// Split a file name into `(base, ext)` where `ext` includes the leading dot.
///
/// Uses the **last** `.` as the extension boundary (matching macOS Finder and
/// Chrome download naming). If the file has no extension, or the only dot is at
/// the very start (hidden files like `.env`), the entire name is treated as the
/// base and `ext` is empty.
///
/// Examples:
/// - `"image.png"` -> `("image", ".png")`
/// - `"foo.tar.gz"` -> `("foo.tar", ".gz")`
/// - `"README"` -> `("README", "")`
/// - `".env"` -> `(".env", "")`
fn split_base_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(idx) if idx > 0 => name.split_at(idx),
        _ => (name, ""),
    }
}

/// Get file metadata synchronously.
#[cfg(test)]
fn get_file_metadata_sync(path: &Path) -> Result<FileMetadata, AppError> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| AppError::NotFound(format!("cannot read metadata for '{}': {e}", path.display())))?;
    Ok(file_metadata_from_observation(path, &metadata))
}

fn file_metadata_from_observation(path: &Path, metadata: &std::fs::Metadata) -> FileMetadata {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let size = metadata.len();
    let is_directory = metadata.is_dir();

    let mime_type = if is_directory {
        "inode/directory".to_owned()
    } else {
        mime_guess::from_path(path)
            .first()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_owned())
    };

    let last_modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    FileMetadata {
        name,
        path: path.to_string_lossy().into_owned(),
        size,
        mime_type,
        last_modified,
        is_directory,
    }
}

/// Remove a file or directory synchronously. Directories are removed recursively.
#[cfg(test)]
fn remove_entry_sync(path: &Path) -> Result<(), AppError> {
    remove_entry_sync_with_hook(path, || {})
}

fn remove_entry_scoped_sync_with_hooks(
    path: &Path, authority: &PathAuthority,
    before_open: impl FnOnce(), after_open: impl FnOnce(),
) -> Result<(), AppError> {
    #[cfg(windows)]
    let root = crate::windows_read::ReadRoot::for_target(path, authority)?;
    #[cfg(not(windows))]
    let _ = authority;
    before_open();
    #[cfg(windows)]
    if let Some(root) = root {
        return crate::windows_delete::remove_entry(path, Some(&root), after_open);
    }
    remove_entry_sync_with_hook(path, after_open)
}

fn remove_entry_sync_with_hook(path: &Path, after_open: impl FnOnce()) -> Result<(), AppError> {
    #[cfg(windows)]
    { crate::windows_delete::remove_entry(path, None, after_open) }
    #[cfg(not(windows))]
    {
        let metadata =
            std::fs::metadata(path).map_err(|e| AppError::NotFound(format!("cannot remove '{}': {e}", path.display())))?;
        after_open();
        if metadata.is_dir() {
            std::fs::remove_dir_all(path)
                .map_err(|e| AppError::Internal(format!(
                    "{FILE_DELETE_OUTCOME_UNKNOWN}; recursive removal of '{}' failed: {e}; inspect the remaining tree before retry",
                    path.display()
                )))
        } else {
            std::fs::remove_file(path)
                .map_err(|e| AppError::Internal(format!("cannot remove file '{}': {e}", path.display())))
        }
    }
}

/// Rename a file or directory synchronously. Returns the new absolute path.
fn rename_entry_sync(path: &Path, new_name: &str) -> Result<PathBuf, AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::BadRequest(format!("path '{}' has no parent", path.display())))?;

    let new_path = parent.join(new_name);

    if new_path.exists() {
        return Err(AppError::BadRequest(format!(
            "target '{}' already exists",
            new_path.display()
        )));
    }

    std::fs::rename(path, &new_path).map_err(|e| {
        AppError::Internal(format!(
            "cannot rename '{}' to '{}': {e}",
            path.display(),
            new_path.display()
        ))
    })?;

    Ok(new_path)
}

/// Copy a single file, creating parent directories as needed.
fn copy_single_file_sync(src: &Path, dest: &Path, workspace: &Path) -> Result<(), AppError> {
    // Validate the nearest existing ancestor before creating any directories.
    // A workspace subdirectory may already be a link to somewhere else.
    for ancestor in dest.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                validate_path(&ancestor.to_string_lossy(), &[workspace])?;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(AppError::Internal(format!("cannot inspect copy target: {error}"))),
        }
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Internal(format!("cannot create directory '{}': {e}", parent.display())))?;
    }

    let target = validate_path_for_write(&dest.to_string_lossy(), &[workspace])?;
    std::fs::copy(src, &target)
        .map_err(|e| AppError::Internal(format!("cannot copy '{}' to '{}': {e}", src.display(), dest.display())))?;

    Ok(())
}

/// Read a local image file and return a base64 Data URL.
fn get_image_base64_sync(path: &Path) -> Result<String, AppError> {
    let bytes =
        std::fs::read(path).map_err(|e| AppError::NotFound(format!("cannot read image '{}': {e}", path.display())))?;

    let mime = mime_guess::from_path(path)
        .first()
        .map(|m| m.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_owned());

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);

    Ok(format!("data:{mime};base64,{encoded}"))
}

/// Build a placeholder SVG Data URL for failed remote image fetches.
fn placeholder_svg_data_url() -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(PLACEHOLDER_SVG);
    format!("data:image/svg+xml;base64,{encoded}")
}

/// Check whether a URL host is in the allowed whitelist.
fn is_allowed_image_host(url: &reqwest::Url) -> bool {
    let host = match url.host_str() {
        Some(h) => h,
        None => return false,
    };
    ALLOWED_IMAGE_HOSTS.contains(&host)
}

/// Validate a remote image URL: protocol must be HTTP(S) and host must be
/// whitelisted.
fn validate_remote_image_url(raw_url: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw_url).map_err(|e| format!("invalid URL '{raw_url}': {e}"))?;

    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!("unsupported protocol '{scheme}', only HTTP/HTTPS allowed"));
        }
    }

    if !is_allowed_image_host(&url) {
        return Err(format!(
            "host '{}' is not in the allowed image host list",
            url.host_str().unwrap_or("unknown")
        ));
    }

    Ok(url)
}

/// Read at most the existing image budget, including responses without a
/// Content-Length or with transparent decompression. Never collect first.
async fn read_remote_image_body(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if chunk.len() > MAX_REMOTE_IMAGE_SIZE - bytes.len() {
            return Err("remote image body exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

struct ZipCancellationGuard<'a> {
    cancellations: &'a DashMap<String, Arc<AtomicBool>>,
    request_id: Option<&'a str>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for ZipCancellationGuard<'_> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(id) = self.request_id {
            self.cancellations.remove_if(id, |_, flag| Arc::ptr_eq(flag, &self.cancelled));
        }
    }
}

/// Synchronous ZIP creation (runs in blocking thread pool).
///
/// Writes entries into a ZIP archive at `output_path`. Checks the
/// `cancelled` flag between entries and aborts early if set.
/// On cancellation, the partial ZIP file is removed.
fn create_zip_sync(output_path: &Path, entries: &[ZipEntry], cancelled: &AtomicBool) -> Result<bool, AppError> {
    if cancelled.load(Ordering::Relaxed) {
        return Ok(false);
    }
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::Internal(format!(
                "cannot create parent directory for '{}': {e}",
                output_path.display()
            ))
        })?;
    }

    let file = std::fs::File::create(output_path)
        .map_err(|e| AppError::Internal(format!("cannot create ZIP file '{}': {e}", output_path.display())))?;

    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let result = write_zip_entries(&mut zip, entries, cancelled, options);

    if let Err(e) = result {
        drop(zip);
        let _ = std::fs::remove_file(output_path);
        return Err(e);
    }

    // write_zip_entries returned Ok(false) means cancelled
    if !result.unwrap() {
        drop(zip);
        let _ = std::fs::remove_file(output_path);
        return Ok(false);
    }

    zip.finish().map_err(|e| {
        let _ = std::fs::remove_file(output_path);
        AppError::Internal(format!("ZIP: failed to finalize '{}': {e}", output_path.display()))
    })?;

    Ok(true)
}

/// Write entries into a ZIP writer. Returns `Ok(true)` when all entries
/// are written, `Ok(false)` if cancelled, or `Err` on I/O failure.
fn write_zip_entries(
    zip: &mut zip::ZipWriter<std::fs::File>,
    entries: &[ZipEntry],
    cancelled: &AtomicBool,
    options: zip::write::SimpleFileOptions,
) -> Result<bool, AppError> {
    for entry in entries {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(false);
        }

        match entry {
            ZipEntry::Text { name, content } => {
                if !write_zip_entry(zip, name, content.as_bytes(), cancelled, options)? {
                    return Ok(false);
                }
            }
            ZipEntry::Disk { name, file_path } => {
                let source = std::fs::File::open(file_path)
                    .map_err(|e| AppError::Internal(format!("ZIP: cannot read source file '{file_path}': {e}")))?;
                let metadata = source.metadata()
                    .map_err(|e| AppError::Internal(format!("ZIP: cannot inspect source file '{file_path}': {e}")))?;
                if !metadata.is_file() {
                    return Err(AppError::BadRequest("ZIP source must be a regular file".into()));
                }
                // A growing source must not keep the export running forever.
                if !write_zip_entry(zip, name, source.take(metadata.len()), cancelled, options)? {
                    return Ok(false);
                }
            }
        }
    }

    // Final cancellation check before finishing
    if cancelled.load(Ordering::Relaxed) {
        return Ok(false);
    }

    Ok(true)
}

fn write_zip_entry(
    zip: &mut zip::ZipWriter<std::fs::File>,
    name: &str,
    mut source: impl Read,
    cancelled: &AtomicBool,
    options: zip::write::SimpleFileOptions,
) -> Result<bool, AppError> {
    zip.start_file(name, options)
        .map_err(|e| AppError::Internal(format!("ZIP: failed to start entry '{name}': {e}")))?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let count = match source.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.map_err(|e| AppError::Internal(format!("ZIP: failed to read entry '{name}': {e}")))?,
        };
        if count == 0 {
            return Ok(true);
        }
        zip.write_all(&buffer[..count])
            .map_err(|e| AppError::Internal(format!("ZIP: failed to write entry '{name}': {e}")))?;
    }
}

#[async_trait::async_trait]
impl crate::traits::IFileService for FileService {
    async fn get_files_by_dir(&self, dir: &str, root: &str) -> Result<Vec<DirOrFile>, AppError> {
        self.get_files_by_dir_impl(dir, root, &self.base_authority(None)).await
    }

    async fn get_files_by_dir_scoped(
        &self,
        dir: &str,
        root: &str,
        authority: &PathAuthority,
    ) -> Result<Vec<DirOrFile>, AppError> {
        self.get_files_by_dir_impl(dir, root, authority).await
    }

    async fn list_workspace_files(&self, root: &str) -> Result<Vec<WorkspaceFlatFile>, AppError> {
        self.list_workspace_files_impl(root, &self.base_authority(None)).await
    }

    async fn list_workspace_files_scoped(
        &self,
        root: &str,
        authority: &PathAuthority,
    ) -> Result<Vec<WorkspaceFlatFile>, AppError> {
        self.list_workspace_files_impl(root, authority).await
    }

    async fn get_file_metadata(&self, path: &str, extra_root: Option<&Path>) -> Result<FileMetadata, AppError> {
        self.get_file_metadata_impl(path, &self.base_authority(extra_root)).await
    }

    async fn get_file_metadata_scoped(
        &self,
        path: &str,
        authority: &PathAuthority,
    ) -> Result<FileMetadata, AppError> {
        self.get_file_metadata_impl(path, authority).await
    }

    // -- File read/write (task 7.4) --

    async fn read_file(&self, path: &str, extra_root: Option<&Path>) -> Result<Option<String>, AppError> {
        self.read_file_impl(path, &self.base_authority(extra_root)).await
    }

    async fn read_file_scoped(&self, path: &str, authority: &PathAuthority) -> Result<Option<String>, AppError> {
        self.read_file_impl(path, authority).await
    }

    async fn write_file(
        &self,
        owner_id: &str,
        path: &str,
        data: &[u8],
        workspace: &str,
    ) -> Result<bool, AppError> {
        self.write_file_impl(owner_id, path, data, workspace, &self.base_authority(None))
            .await
    }

    async fn write_file_scoped(
        &self,
        owner_id: &str,
        path: &str,
        data: &[u8],
        workspace: &str,
        authority: &PathAuthority,
    ) -> Result<bool, AppError> {
        self.write_file_impl(owner_id, path, data, workspace, authority).await
    }

    async fn copy_files_to_workspace(
        &self,
        file_paths: &[String],
        workspace: &str,
        source_root: Option<&str>,
    ) -> Result<CopyResult, AppError> {
        let roots = self.allowed_roots_refs();
        let ws_canonical = validate_path(workspace, &roots)?;

        let sr_canonical = match source_root {
            Some(sr) => Some(validate_path(sr, &roots)?),
            None => None,
        };

        let file_paths_owned: Vec<String> = file_paths.to_vec();
        let roots_owned: Vec<std::path::PathBuf> = self.allowed_roots.clone();

        tokio::task::spawn_blocking(move || {
            let roots_refs: Vec<&Path> = roots_owned.iter().map(|p| p.as_path()).collect();
            let mut copied = Vec::new();
            let mut failed = Vec::new();

            for fp in &file_paths_owned {
                let src = match validate_path(fp, &roots_refs) {
                    Ok(p) if p.is_file() => p,
                    _ => {
                        failed.push(fp.clone());
                        continue;
                    }
                };

                let relative = match &sr_canonical {
                    Some(sr) => src
                        .strip_prefix(sr)
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|_| Path::new(src.file_name().unwrap_or_default()).to_path_buf()),
                    None => Path::new(src.file_name().unwrap_or_default()).to_path_buf(),
                };

                let dest = ws_canonical.join(&relative);
                match copy_single_file_sync(&src, &dest, &ws_canonical) {
                    Ok(()) => copied.push(fp.clone()),
                    Err(_) => failed.push(fp.clone()),
                }
            }

            Ok(CopyResult {
                copied_files: copied,
                failed_files: failed,
            })
        })
        .await
        .map_err(|e| AppError::Internal(format!("copy task failed: {e}")))?
    }

    async fn remove_entry(&self, owner_id: &str, path: &str, workspace: &str) -> Result<(), AppError> {
        self.remove_entry_impl(owner_id, path, workspace, &self.base_authority(None))
            .await
    }

    async fn remove_entry_scoped(
        &self,
        owner_id: &str,
        path: &str,
        workspace: &str,
        authority: &PathAuthority,
    ) -> Result<(), AppError> {
        self.remove_entry_impl(owner_id, path, workspace, authority).await
    }

    async fn rename_entry(&self, path: &str, new_name: &str) -> Result<String, AppError> {
        self.rename_entry_impl(path, new_name, &self.base_authority(None)).await
    }

    async fn rename_entry_scoped(
        &self,
        path: &str,
        new_name: &str,
        authority: &PathAuthority,
    ) -> Result<String, AppError> {
        self.rename_entry_impl(path, new_name, authority).await
    }

    async fn create_upload_file(
        &self,
        file_name: &str,
        data: &[u8],
        conversation_id: Option<&str>,
    ) -> Result<String, AppError> {
        if file_name.is_empty() {
            return Err(AppError::BadRequest("file name must not be empty".to_owned()));
        }
        if has_traversal(file_name) {
            return Err(AppError::BadRequest(format!(
                "file name '{}' contains invalid traversal patterns",
                file_name
            )));
        }
        if file_name.contains('/') || file_name.contains('\\') {
            return Err(AppError::BadRequest(format!(
                "file name '{}' must not contain path separators",
                file_name
            )));
        }

        if is_unsafe_path_segment(file_name) {
            return Err(AppError::BadRequest(format!(
                "file name '{}' is not a valid file name",
                file_name
            )));
        }

        // Validate optional conversation_id: it becomes a directory segment.
        let conv_id = match conversation_id {
            Some(id) if !id.is_empty() => {
                if is_unsafe_path_segment(id) {
                    return Err(AppError::BadRequest(format!(
                        "conversation id '{}' contains invalid characters",
                        id
                    )));
                }
                Some(id.to_owned())
            }
            _ => None,
        };

        let name = file_name.to_owned();
        let bytes = data.to_vec();

        tokio::task::spawn_blocking(move || {
            let mut dir = std::env::temp_dir().join("nomifun");
            if let Some(conv_id) = conv_id.as_deref() {
                dir = dir.join(conv_id);
            } else {
                dir = dir.join("general");
            }
            std::fs::create_dir_all(&dir)
                .map_err(|e| AppError::Internal(format!("cannot create upload directory: {e}")))?;

            let (base, ext) = split_base_ext(&name);
            let mut candidate = name.clone();
            let mut counter: u32 = 2;
            loop {
                let file_path = dir.join(&candidate);
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&file_path)
                {
                    Ok(mut f) => {
                        f.write_all(&bytes).map_err(|e| {
                            AppError::Internal(format!("cannot write upload file '{}': {e}", file_path.display()))
                        })?;
                        return Ok(file_path.to_string_lossy().into_owned());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        if counter > 1000 {
                            return Err(AppError::Internal(format!(
                                "too many name collisions for upload file '{}'",
                                name
                            )));
                        }
                        candidate = format!("{base}({counter}){ext}");
                        counter += 1;
                    }
                    Err(e) => {
                        return Err(AppError::Internal(format!(
                            "cannot write upload file '{}': {e}",
                            file_path.display()
                        )));
                    }
                }
            }
        })
        .await
        .map_err(|e| AppError::Internal(format!("create upload file task failed: {e}")))?
    }

    async fn get_image_base64(&self, path: &str, extra_root: Option<&Path>) -> Result<String, AppError> {
        if has_traversal(path) {
            return Err(AppError::BadRequest(format!(
                "path '{}' contains invalid traversal patterns",
                path
            )));
        }

        let roots = self.allowed_roots_refs();
        let canonical = validate_path_with_extra_root(path, &roots, extra_root)?;

        tokio::task::spawn_blocking(move || get_image_base64_sync(&canonical))
            .await
            .map_err(|e| AppError::Internal(format!("image base64 task failed: {e}")))?
    }

    async fn fetch_remote_image(&self, url: &str) -> String {
        let parsed = match validate_remote_image_url(url) {
            Ok(u) => u,
            Err(e) => {
                warn!("remote image rejected: {e}");
                return placeholder_svg_data_url();
            }
        };

        let client = match reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if let Err(error) = validate_remote_image_url(attempt.url().as_str()) {
                    return attempt.error(error);
                }
                reqwest::redirect::Policy::limited(MAX_REDIRECTS).redirect(attempt)
            }))
            .timeout(REMOTE_IMAGE_TIMEOUT)
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!("failed to build HTTP client: {e}");
                return placeholder_svg_data_url();
            }
        };

        let response = match client.get(parsed.clone()).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("remote image fetch failed for '{}': {e}", url);
                return placeholder_svg_data_url();
            }
        };

        if !response.status().is_success() {
            warn!("remote image fetch returned status {} for '{}'", response.status(), url);
            return placeholder_svg_data_url();
        }

        // Early reject if Content-Length exceeds limit
        if let Some(len) = response.content_length()
            && len > MAX_REMOTE_IMAGE_SIZE as u64
        {
            warn!("remote image too large ({} bytes) for '{}'", len, url);
            return placeholder_svg_data_url();
        }

        // Determine MIME from Content-Type header, fall back to URL extension
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|ct| ct.split(';').next())
            .map(|s| s.trim().to_owned());

        let mime = content_type.unwrap_or_else(|| {
            mime_guess::from_path(parsed.path())
                .first()
                .map(|m| m.to_string())
                .unwrap_or_else(|| "application/octet-stream".to_owned())
        });

        let bytes = match read_remote_image_body(response).await {
            Ok(b) => b,
            Err(e) => {
                warn!("failed to read remote image body for '{}': {e}", url);
                return placeholder_svg_data_url();
            }
        };

        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        format!("data:{mime};base64,{encoded}")
    }

    async fn create_zip(
        &self,
        path: &str,
        mut entries: Vec<ZipEntry>,
        request_id: Option<String>,
    ) -> Result<bool, AppError> {
        // Validate output path is within the sandbox
        let roots = self.allowed_roots_refs();
        let output = validate_path_for_write(path, &roots)?;

        // Validate all Disk entry source paths are within the sandbox
        for entry in &mut entries {
            if let ZipEntry::Disk { file_path, .. } = entry {
                let source = validate_path(file_path, &roots)?;
                if source == output || !source.is_file() {
                    return Err(AppError::BadRequest("ZIP source must be a regular file distinct from the output".into()));
                }
                *file_path = source.to_string_lossy().into_owned();
            }
        }

        let cancelled = Arc::new(AtomicBool::new(false));

        if let Some(ref id) = request_id {
            match self.zip_cancellations.entry(id.clone()) {
                dashmap::mapref::entry::Entry::Occupied(_) => {
                    return Err(AppError::Conflict("ZIP request is already in progress".into()));
                }
                dashmap::mapref::entry::Entry::Vacant(entry) => {
                    entry.insert(Arc::clone(&cancelled));
                }
            }
        }
        // Dropping the caller future must cancel its blocking worker and release
        // the registration, just like an I/O error or successful completion.
        let _registration = ZipCancellationGuard {
            cancellations: &self.zip_cancellations,
            request_id: request_id.as_deref(),
            cancelled: Arc::clone(&cancelled),
        };

        tokio::task::spawn_blocking(move || create_zip_sync(&output, &entries, &cancelled))
            .await
            .map_err(|e| AppError::Internal(format!("ZIP creation task failed: {e}")))?
    }

    async fn cancel_zip(&self, request_id: &str) -> bool {
        if let Some((_, flag)) = self.zip_cancellations.remove(request_id) {
            flag.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(windows)]
    fn cleanup_race_fixture() -> tempfile::TempDir {
        let mut builder = tempfile::Builder::new();
        builder.prefix("cleanup-race-");
        match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent) => builder.disable_cleanup(true).tempdir_in(parent).unwrap(),
            None => builder.tempdir().unwrap(),
        }
    }

    #[cfg(windows)]
    async fn directory_tree_race(child_window: bool) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let fixture = cleanup_race_fixture();
        let root = fixture.path().join("workspace");
        let directory = root.join("directory");
        let retained = fixture.path().join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&directory).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(directory.join("inside.txt"), b"inside").unwrap();
        fs::write(outside.join("outside.txt"), b"outside").unwrap();
        let canonical_directory = fs::canonicalize(&directory).unwrap();
        let redirected = Arc::new(AtomicBool::new(false));
        let replace = {
            let (directory, retained, outside, redirected) = (directory.clone(), retained.clone(), outside.clone(), redirected.clone());
            move || {
                if !redirected.swap(true, Ordering::SeqCst) {
                    fs::rename(&directory, &retained).unwrap();
                    junction::create(&outside, &directory).unwrap();
                }
            }
        };
        let before_tree = { let replace = replace.clone(); move || { if !child_window { replace(); } } };
        let before_children = move |path: &Path| { if child_window && path == canonical_directory { replace(); } };
        let service = make_service();
        let result = service.get_files_by_dir_with_hooks(
            if child_window { &root } else { &directory }.to_str().unwrap(), root.to_str().unwrap(),
            &PathAuthority::Workspace(root.clone()), before_tree, before_children).await;
        assert!(redirected.load(Ordering::SeqCst), "the intended boundary must be reached");
        junction::delete(&directory).unwrap();
        fs::rename(&retained, &directory).unwrap();
        if let Ok(items) = &result {
            let names = items.iter().flat_map(|item| std::iter::once(&item.name).chain(item.children.iter().map(|child| &child.name)))
                .cloned().collect::<Vec<_>>();
            if names.iter().any(|name| name == "outside.txt") {
                fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&serde_json::json!({
                    "child_window": child_window, "names": names,
                })).unwrap()).unwrap();
                panic!("directory tree disclosed outside entries; retained fixture: {}", fixture.keep().display());
            }
            assert!(names.iter().any(|name| name == "inside.txt"), "a failed nested read must not look empty");
        }
        assert_eq!(fs::read(directory.join("inside.txt")).unwrap(), b"inside");
        assert_eq!(fs::read(outside.join("outside.txt")).unwrap(), b"outside");
        let items = service.get_files_by_dir_impl(directory.to_str().unwrap(), root.to_str().unwrap(), &PathAuthority::Workspace(root.clone())).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "inside.txt");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn directory_tree_rejects_replacement_after_path_validation() { directory_tree_race(false).await; }

    #[cfg(windows)]
    #[tokio::test]
    async fn directory_tree_rejects_child_replacement_before_prefetch() { directory_tree_race(true).await; }

    #[cfg(windows)]
    #[tokio::test]
    async fn agent_metadata_never_returns_attributes_from_a_transient_parent_escape() {
        let fixture = cleanup_race_fixture();
        let root = fixture.path().join("workspace");
        let parent = root.join("parent");
        let retained = root.join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(parent.join("source.txt"), b"inside").unwrap();
        fs::write(outside.join("source.txt"), b"outside private metadata").unwrap();
        let before = {
            let (parent, retained, outside) = (parent.clone(), retained.clone(), outside.clone());
            move || { fs::rename(&parent, &retained).unwrap(); junction::create(&outside, &parent).unwrap(); }
        };
        let after = {
            let (parent, retained) = (parent.clone(), retained.clone());
            move || { junction::delete(&parent).unwrap(); fs::rename(&retained, &parent).unwrap(); }
        };
        let service = make_service();
        let binding = patch_scope(&root);
        let result = service.get_file_metadata_with_hooks(parent.join("source.txt").to_str().unwrap(), &binding.authority(), before, after).await;
        if let Ok(metadata) = &result {
            if metadata.size != b"inside".len() as u64 {
                fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&serde_json::json!({
                    "name": metadata.name, "path": metadata.path, "size": metadata.size, "is_directory": metadata.is_directory,
                })).unwrap()).unwrap();
                panic!("metadata came from outside the workspace; retained fixture: {}", fixture.keep().display());
            }
        }
        assert_eq!(fs::read(parent.join("source.txt")).unwrap(), b"inside");
        assert_eq!(fs::read(outside.join("source.txt")).unwrap(), b"outside private metadata");
        let metadata = service.get_file_metadata_for_agent_session(&binding, "parent/source.txt").await.unwrap();
        assert_eq!(metadata.size, b"inside".len() as u64);
        assert_eq!(metadata.name, "source.txt");
        assert_eq!(metadata.mime_type, "text/plain");
    }

    #[cfg(windows)]
    async fn patch_guard_read_race(precondition: bool, replace_parent: bool) {
        let fixture = cleanup_race_fixture();
        let root = fixture.path().join("workspace");
        let parent = root.join("parent");
        let retained = root.join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(parent.join("value.txt"), if replace_parent { b"inside" } else { b"source" }).unwrap();
        fs::write(outside.join("value.txt"), b"source").unwrap();
        let path = fs::canonicalize(parent.join("value.txt")).unwrap();
        let authority = PathAuthority::Workspace(root);
        let redirected = std::cell::Cell::new(false);
        let before_open = || {
            if replace_parent {
                fs::rename(&parent, &retained).unwrap();
                junction::create(&outside, &parent).unwrap();
                redirected.set(true);
            }
        };
        let restore = || {
            if redirected.replace(false) {
                junction::delete(&parent).unwrap();
                fs::rename(&retained, &parent).unwrap();
            }
        };
        let after_read = || {
            restore();
            if !replace_parent {
                fs::rename(&path, parent.join("original.txt")).unwrap();
                fs::write(&path, b"source").unwrap();
            }
        };
        let accepted = if precondition {
            let file = PreparedAgentPatchFile {
                path: path.clone(), relative_path: "parent/value.txt".into(),
                before: b"source".to_vec(), after: b"patched".to_vec(), existed: true, hunks_applied: 1,
            };
            make_service().verify_agent_patch_precondition_with_hooks(&file, &authority, before_open, after_read).await.is_ok()
        } else {
            current_file_matches_with_hooks(&path, b"source", &authority, before_open, after_read)
        };
        restore();
        if accepted {
            fs::write(fixture.path().join("observation.txt"), format!(
                "precondition={precondition}; replace_parent={replace_parent}; incorrectly_accepted={accepted}"
            )).unwrap();
            panic!("patch guard accepted a different source; retained fixture: {}", fixture.keep().display());
        }
        assert_eq!(fs::read(&path).unwrap(), if replace_parent { b"inside" } else { b"source" });
        assert_eq!(fs::read(outside.join("value.txt")).unwrap(), b"source");
        assert_eq!(fs::read_dir(&parent).unwrap().count(), if replace_parent { 1 } else { 2 });
        if !replace_parent { assert_eq!(fs::read(parent.join("original.txt")).unwrap(), b"source"); }
        assert!(current_file_matches(&path, if replace_parent { b"inside" } else { b"source" }, &authority));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn patch_precondition_rejects_a_transient_parent_escape() {
        patch_guard_read_race(true, true).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn patch_precondition_rejects_replacement_during_read() {
        patch_guard_read_race(true, false).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn patch_content_guard_rejects_a_transient_parent_escape() {
        patch_guard_read_race(false, true).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn patch_content_guard_rejects_replacement_during_read() {
        patch_guard_read_race(false, false).await;
    }

    #[cfg(windows)]
    fn new_publication_stage_race(attack: &str) {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("new.txt");
        let temporary = fixture.path().join("stage.tmp");
        let foreign = fixture.path().join("foreign.txt");
        let retained = fixture.path().join("retained.tmp");
        fs::write(&foreign, b"foreign bytes").unwrap();
        let mut attack_result = None;
        let result = publish_patch_file_with_hooks(&target, b"intended bytes", &temporary,
            PublicationSource::Absent, || {
                attack_result = Some(match attack {
                    "rename" => fs::rename(&temporary, &retained).and_then(|()| fs::copy(&foreign, &temporary).map(|_| ())),
                    "posix-replace" => crate::windows_test_support::rename_with_posix_semantics(&foreign, &temporary, true),
                    "write" => fs::write(&temporary, b"foreign bytes"),
                    _ => unreachable!(),
                });
            }, || Ok(()));
        let bytes = fs::read(&target).ok();
        if bytes.as_deref() != Some(b"intended bytes") {
            fs::write(fixture.path().join("observation.txt"), format!(
                "attack={attack}; attack_result={attack_result:?}; publication={result:?}; target={bytes:?}"
            )).unwrap();
            panic!("new publication used a changed stage; retained fixture: {}", fixture.keep().display());
        }
        result.unwrap();
        let error = attack_result.unwrap().unwrap_err();
        assert_eq!(error.raw_os_error(), Some(32), "{attack}: {error}");
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign bytes");
        assert!(!temporary.exists());
        assert!(!retained.exists());
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 2);
        // Publication releases its lock; this is an operation boundary, not
        // a promise to prevent subsequent native edits to the completed file.
        fs::write(&target, b"later edit").unwrap();
        fs::rename(&target, &retained).unwrap();
        assert_eq!(fs::read(&retained).unwrap(), b"later edit");
    }

    #[cfg(windows)]
    #[test]
    fn new_publication_keeps_the_staged_name_until_publish() {
        new_publication_stage_race("rename");
    }

    #[cfg(windows)]
    #[test]
    fn new_publication_keeps_the_staged_object_during_posix_replace() {
        new_publication_stage_race("posix-replace");
    }

    #[cfg(windows)]
    #[test]
    fn new_publication_keeps_the_staged_bytes_until_publish() {
        new_publication_stage_race("write");
    }

    #[cfg(windows)]
    #[test]
    fn new_publication_preserves_concurrent_targets_and_cleans_its_stage() {
        for entry in ["file", "directory", "junction"] {
            let fixture = cleanup_race_fixture();
            let target = fixture.path().join("new.txt");
            let temporary = fixture.path().join("stage.tmp");
            let outside = fixture.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("sentinel"), b"outside").unwrap();
            let failure = publish_patch_file_with_hooks(&target, b"intended bytes", &temporary,
                PublicationSource::Absent, || match entry {
                    "file" => fs::write(&target, b"concurrent").unwrap(),
                    "directory" => {
                        fs::create_dir(&target).unwrap();
                        fs::write(target.join("sentinel"), b"concurrent").unwrap();
                    }
                    "junction" => junction::create(&outside, &target).unwrap(),
                    _ => unreachable!(),
                }, || panic!("must not report a rejected publication as successful")).unwrap_err();
            assert!(!failure.published, "{entry}: {failure:?}");
            assert!(!failure.temporary_cleanup_unconfirmed, "{entry}: {failure:?}");
            assert!(!file_write_outcome_unknown(&file_write_publication_error(failure)));
            assert!(!temporary.exists());
            match entry {
                "file" => assert_eq!(fs::read(&target).unwrap(), b"concurrent"),
                "directory" => assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"concurrent"),
                "junction" => {
                    assert!(junction::exists(&target).unwrap());
                    junction::delete(&target).unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        }
    }

    #[cfg(windows)]
    #[test]
    fn staging_cleanup_preserves_a_foreign_file_at_the_temporary_name() {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        let retained = fixture.path().join("retained.tmp");
        fs::write(&target, b"original").unwrap();
        let result = publish_patch_file_with_hooks(&target, b"patched", &temporary,
            PublicationSource::Matching(b"stale"), || {
                fs::rename(&temporary, &retained).unwrap();
                fs::write(&temporary, b"foreign file").unwrap();
            }, || Ok(()));
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(fs::read(&retained).unwrap(), b"patched");
        if fs::read(&temporary).ok().as_deref() != Some(b"foreign file") {
            fs::write(fixture.path().join("observation.txt"), format!("result={result:?}; foreign temporary was removed")).unwrap();
            panic!("unowned temporary removed; retained fixture: {}", fixture.keep().display());
        }
        let failure = result.unwrap_err();
        assert!(!failure.published);
        assert!(failure.temporary_cleanup_unconfirmed);
        assert!(file_write_outcome_unknown(&file_write_publication_error(failure)));
    }

    #[test]
    fn post_publication_failure_does_not_clean_a_reused_temporary_name() {
        for source in [PublicationSource::Absent, PublicationSource::Existing] {
            let fixture = tempfile::tempdir().unwrap();
            let target = fixture.path().join("target.txt");
            let temporary = fixture.path().join("stage.tmp");
            if matches!(source, PublicationSource::Existing) {
                fs::write(&target, b"original").unwrap();
            }
            let result = publish_patch_file_with_hooks(&target, b"published", &temporary, source,
                || {}, || {
                    fs::write(&temporary, b"foreign file").unwrap();
                    Err(AppError::Internal("injected failure after publication".into()))
                });
            let failure = result.unwrap_err();
            assert!(failure.published);
            assert!(!failure.temporary_cleanup_unconfirmed);
            assert_eq!(fs::read(&target).unwrap(), b"published");
            assert_eq!(fs::read(&temporary).unwrap(), b"foreign file");
        }
    }

    #[cfg(windows)]
    #[test]
    fn replacement_cleanup_preserves_a_foreign_file_at_the_backup_name() {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        let retained = fixture.path().join("retained-original.txt");
        fs::write(&target, b"original").unwrap();
        fs::write(&temporary, b"patched").unwrap();
        let mut published = false;
        let backup_name = std::cell::RefCell::new(PathBuf::new());
        let result = replace_file_path_windows_with(&temporary, &target, &mut published, None,
            |source, target, backup| {
                replace_file_windows_native(source, target, backup)?;
                fs::rename(backup, &retained)?;
                fs::write(backup, b"foreign backup")?;
                *backup_name.borrow_mut() = backup.to_path_buf();
                Ok(())
            }, |backup, owner| owner.remove(backup));
        assert_eq!(fs::read(&target).unwrap(), b"patched");
        assert_eq!(fs::read(&retained).unwrap(), b"original");
        if fs::read(backup_name.borrow().as_path()).ok().as_deref() != Some(b"foreign backup") {
            fs::write(fixture.path().join("observation.txt"), format!("published={published}; result={result:?}; foreign backup was removed")).unwrap();
            panic!("unowned backup removed; retained fixture: {}", fixture.keep().display());
        }
        assert!(published);
        assert!(result.unwrap_err().to_string().contains(FILE_WRITE_OUTCOME_UNKNOWN));
    }

    #[cfg(windows)]
    fn matching_publication_guard_race(changed: &[u8]) {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        fs::write(&target, b"original").unwrap();
        let result = publish_patch_file_with_prepublication_hook(&target, b"patched", &temporary,
            PublicationSource::Matching(b"original"), || {}, || {
                fs::write(&target, changed).unwrap();
            }, || Ok(()), replace_file_windows_native);
        let observed = fs::read(&target).unwrap();
        fs::write(fixture.path().join("observation.txt"), format!("result={result:?}; target={observed:?}")).unwrap();
        assert_eq!(observed, changed, "a changed target must survive; fixture: {}", fixture.path().display());
        let failure = result.unwrap_err();
        assert!(matches!(failure.error, AppError::Conflict(_)));
        assert!(!failure.published);
        assert!(!failure.temporary_cleanup_unconfirmed);
        assert!(!temporary.exists());
    }

    #[cfg(windows)]
    #[test]
    fn matching_publication_preserves_a_target_changed_after_the_early_guard() {
        matching_publication_guard_race(b"concurrent");
    }

    #[cfg(windows)]
    #[test]
    fn matching_publication_checks_equal_length_bytes_under_the_target_handle() {
        matching_publication_guard_race(b"modified");
    }

    #[cfg(windows)]
    #[test]
    fn replacement_keeps_the_target_bytes_locked_through_the_native_call() {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        fs::write(&target, b"original").unwrap();
        fs::write(&temporary, b"patched").unwrap();
        let mut published = false;
        let mut attack = None;
        let result = replace_file_path_windows_with(&temporary, &target, &mut published, None,
            |source, target, backup| {
                attack = Some(fs::write(target, b"concurrent"));
                replace_file_windows_native(source, target, backup)
            }, |backup, owner| owner.remove(backup));
        fs::write(fixture.path().join("observation.txt"), format!("result={result:?}; attack={attack:?}; published={published}")).unwrap();
        let error = attack.unwrap().expect_err("concurrent writes must be excluded while the original is verified and replaced");
        assert_eq!(error.raw_os_error(), Some(32));
        result.unwrap();
        assert!(published);
        assert_eq!(fs::read(&target).unwrap(), b"patched");
        assert!(!temporary.exists());
        fs::write(&target, b"later edit").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"later edit");
    }

    #[cfg(windows)]
    fn replacement_source_race(swap_identity: bool) {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        let retained = fixture.path().join("retained-stage");
        fs::write(&target, b"original").unwrap();
        fs::write(&temporary, b"intended").unwrap();
        let mut published = false;
        let mut backup_path = None;
        let result = replace_file_path_windows_with(&temporary, &target, &mut published, Some(b"original"),
            |source, target, backup| {
                if swap_identity {
                    fs::rename(source, &retained)?;
                    fs::write(source, b"intended")?;
                } else {
                    fs::write(source, b"tampered")?;
                }
                backup_path = Some(backup.to_owned());
                replace_file_windows_native(source, target, backup)
            }, |backup, owner| owner.remove(backup));
        let backup = backup_path.unwrap();
        fs::write(fixture.path().join("observation.txt"), format!(
            "swap_identity={swap_identity}; result={result:?}; published={published}; target={:?}; backup={:?}",
            fs::read(&target), fs::read(&backup),
        )).unwrap();
        assert!(result.as_ref().is_err_and(file_write_outcome_unknown),
            "unverified source must not report success; fixture: {}", fixture.path().display());
        assert!(published);
        assert_eq!(fs::read(&backup).unwrap(), b"original", "recovery evidence must survive");
        if swap_identity {
            assert_eq!(fs::read(&retained).unwrap(), b"intended");
            assert_eq!(fs::read(&target).unwrap(), b"intended");
        } else {
            assert_eq!(fs::read(&target).unwrap(), b"tampered");
        }
    }

    #[cfg(windows)]
    #[test]
    fn replacement_rejects_native_success_with_modified_staged_bytes() {
        replacement_source_race(false);
    }

    #[cfg(windows)]
    #[test]
    fn replacement_rejects_native_success_with_a_foreign_staged_identity() {
        replacement_source_race(true);
    }

    #[derive(Default)]
    struct PublicationEvents(std::sync::Mutex<Vec<WebSocketMessage<serde_json::Value>>>);

    impl UserEventSink for PublicationEvents {
        fn send_to_user(&self, _: &str, event: WebSocketMessage<serde_json::Value>) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn unverified_publication_does_not_emit_intended_content_and_revokes_cached_inventory() {
        let fixture = inventory_fixture();
        let root = fixture.path().canonicalize().unwrap();
        let target = root.join("target.txt");
        fs::write(&target, b"actual unverified bytes").unwrap();
        let events = Arc::new(PublicationEvents::default());
        let service = FileService::new(events.clone(), vec![root.clone()]);
        let workspace = root.to_string_lossy().into_owned();
        service.list_workspace_files_impl(&workspace, &PathAuthority::Workspace(root.clone())).await.unwrap();
        let mut failure = PatchPublicationFailure::from(AppError::Internal(format!("{FILE_WRITE_OUTCOME_UNKNOWN}; unverified source")));
        failure.published = true;
        service.observe_file_publication::<()>("owner", &target, b"intended", &workspace, &Err(failure));
        let observed = serde_json::to_value(&*events.0.lock().unwrap()).unwrap();
        fs::write(fixture.path().join("event-observation.json"), serde_json::to_vec_pretty(&observed).unwrap()).unwrap();
        assert!(!observed.to_string().contains("\"content\":\"intended\""), "an unverified publication must not advertise expected bytes");
        assert!(service.workspace_files_cache.is_empty());
        assert_eq!(fs::read(&target).unwrap(), b"actual unverified bytes");
        service.list_workspace_files_impl(&workspace, &PathAuthority::Workspace(root.clone())).await.unwrap();
        let mut cleanup = PatchPublicationFailure::from(AppError::Conflict("unowned staging name".into()));
        cleanup.temporary_cleanup_unconfirmed = true;
        service.observe_file_publication::<()>("owner", &target, b"intended", &workspace, &Err(cleanup));
        assert!(service.workspace_files_cache.is_empty(), "unconfirmed residues also require a fresh inventory");
    }

    #[cfg(windows)]
    #[test]
    fn staged_publication_rejects_changed_bytes_before_native_dispatch() {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        fs::write(&target, b"original").unwrap();
        let failure = publish_patch_file_with_prepublication_hook(&target, b"intended", &temporary,
            PublicationSource::Matching(b"original"), || { fs::write(&temporary, b"tampered").unwrap(); }, || {},
            || panic!("rejected source must not complete"),
            |_, _, _| panic!("changed source must be rejected before native dispatch"),
        ).unwrap_err();
        assert!(matches!(failure.error, AppError::Conflict(_)));
        assert!(!failure.published);
        assert!(!failure.content_verified);
        assert!(!failure.temporary_cleanup_unconfirmed);
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(!temporary.exists());
    }

    #[cfg(windows)]
    #[test]
    fn staged_publication_propagates_unverified_content_and_source_ownership() {
        for swap in [false, true] {
            let fixture = cleanup_race_fixture();
            let target = fixture.path().join("target.txt");
            let temporary = fixture.path().join("stage.tmp");
            let retained = fixture.path().join("retained-stage");
            fs::write(&target, b"original").unwrap();
            let mut backup_path = None;
            let failure = publish_patch_file_with_prepublication_hook(&target, b"intended", &temporary,
                PublicationSource::Matching(b"original"), || {}, || {}, || panic!("unverified publication must not complete"),
                |source, target, backup| {
                    if swap { fs::rename(source, &retained)?; fs::write(source, b"intended")?; }
                    else { fs::write(source, b"tampered")?; }
                    backup_path = Some(backup.to_owned());
                    replace_file_windows_native(source, target, backup)
                },
            ).unwrap_err();
            assert!(failure.published);
            assert!(!failure.content_verified);
            assert_eq!(failure.temporary_cleanup_unconfirmed, swap);
            assert!(file_write_outcome_unknown(&file_write_publication_error(failure)));
            assert_eq!(fs::read(backup_path.unwrap()).unwrap(), b"original");
            if swap { assert_eq!(fs::read(&retained).unwrap(), b"intended"); }
            assert_eq!(fs::read(&target).unwrap(), if swap { b"intended" } else { b"tampered" });
        }
    }

    #[cfg(windows)]
    #[test]
    fn staged_publication_holds_verified_output_through_backup_cleanup() {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        fs::write(&target, b"original").unwrap();
        fs::write(&temporary, b"intended").unwrap();
        let mut published = false;
        replace_file_path_windows_with(&temporary, &target, &mut published, Some(b"original"),
            replace_file_windows_native, |backup, owner| {
                assert_eq!(fs::write(&target, b"concurrent").unwrap_err().raw_os_error(), Some(32));
                owner.remove(backup)
            }).unwrap();
        assert!(published);
        assert_eq!(fs::read(&target).unwrap(), b"intended");
        fs::write(&target, b"later edit").unwrap();
    }

    #[test]
    fn verified_publication_can_emit_content_after_a_later_cleanup_failure() {
        let fixture = inventory_fixture();
        let root = fixture.path().canonicalize().unwrap();
        let target = root.join("target.txt");
        fs::write(&target, b"verified").unwrap();
        let events = Arc::new(PublicationEvents::default());
        let service = FileService::new(events.clone(), vec![root.clone()]);
        let mut failure = PatchPublicationFailure::from(AppError::Internal(format!("{FILE_WRITE_OUTCOME_UNKNOWN}; backup cleanup pending")));
        failure.published = true;
        failure.content_verified = true;
        service.observe_file_publication::<()>("owner", &target, b"verified", &root.to_string_lossy(), &Err(failure));
        let events = events.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "fileStream.contentUpdate");
        assert_eq!(events[0].data["content"], "verified");
    }

    #[tokio::test]
    async fn unverified_publication_is_not_rolled_back_even_when_expected_bytes_match() {
        let fixture = inventory_fixture();
        let root = fixture.path().canonicalize().unwrap();
        let scope = patch_scope(&root);
        let service = make_service();
        let files = ["first.txt", "second.txt"].map(|name| PreparedAgentPatchFile {
            path: root.join(name), relative_path: name.into(), before: b"before".to_vec(),
            after: b"after".to_vec(), existed: true, hunks_applied: 1,
        });
        for file in &files { fs::write(&file.path, &file.after).unwrap(); }
        let mut failure = PatchPublicationFailure::from(AppError::Internal(format!("{FILE_WRITE_OUTCOME_UNKNOWN}; unverified source identity")));
        failure.published = true;
        let mut eligible = vec![0];
        let mut observation = AgentPatchFailureObservation::failed_publication(1, &mut eligible, &failure);
        assert_eq!(eligible, [0]);
        assert_eq!(observation.published, [0, 1]);
        assert_eq!(observation.unverified_publications, [1]);
        service.rollback_agent_patch_files(&scope, &scope.authority(), &root.to_string_lossy(), &files, &eligible, &mut observation).await;
        assert_eq!(observation.restored, [0]);
        assert_eq!(fs::read(&files[0].path).unwrap(), b"before");
        assert_eq!(fs::read(&files[1].path).unwrap(), b"after");
        assert_eq!(serde_json::to_value(&observation).unwrap()["unverified_publications"], serde_json::json!([1]));
    }

    #[test]
    fn unverified_publication_recovery_and_cleanup_cannot_settle_as_known_failure() {
        for kind in ["publication", "restore", "cleanup"] {
            let mut observation = AgentPatchFailureObservation { failed_file: Some(1), ..Default::default() };
            match kind {
                "publication" => observation.unverified_publications.push(1),
                "restore" => observation.restore_published_unconfirmed.push(0),
                "cleanup" => observation.temporary_cleanup_unconfirmed.push(1),
                _ => unreachable!(),
            }
            let failure = finish_agent_patch_failure(AppError::Conflict("initial known failure".into()), observation);
            assert!(file_write_outcome_unknown(&failure.error), "{kind} uncertainty must retain the effect fence: {failure:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn unverified_publication_cleanup_from_a_real_source_swap_retains_the_fence() {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        let retained = fixture.path().join("retained-stage");
        fs::write(&target, b"original").unwrap();
        let failure = publish_patch_file_with_hooks(&target, b"intended", &temporary,
            PublicationSource::Matching(b"original"), || {
                fs::rename(&temporary, &retained).unwrap();
                fs::write(&temporary, b"foreign").unwrap();
            }, || panic!("unowned staging source must not complete")).unwrap_err();
        assert!(!failure.published);
        assert!(failure.temporary_cleanup_unconfirmed);
        let observation = AgentPatchFailureObservation::failed_publication(0, &mut Vec::new(), &failure);
        let failure = finish_agent_patch_failure(failure.error, observation);
        fs::write(fixture.path().join("observation.txt"), format!("{failure:?}")).unwrap();
        assert!(file_write_outcome_unknown(&failure.error), "real staging cleanup uncertainty must not become a settled failure");
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(fs::read(&temporary).unwrap(), b"foreign");
        assert_eq!(fs::read(&retained).unwrap(), b"intended");
    }

    #[cfg(windows)]
    #[test]
    fn partial_replacement_never_restores_a_foreign_backup_as_the_original() {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let temporary = fixture.path().join("stage.tmp");
        let retained = fixture.path().join("retained-original.txt");
        fs::write(&target, b"original").unwrap();
        fs::write(&temporary, b"patched").unwrap();
        let mut published = false;
        let mut backup_name = PathBuf::new();
        let result = replace_file_path_windows_with(&temporary, &target, &mut published, None,
            |_, target, backup| {
                fs::rename(target, backup)?;
                fs::rename(backup, &retained)?;
                fs::write(backup, b"foreign backup")?;
                backup_name = backup.to_path_buf();
                Err(std::io::Error::from_raw_os_error(1177))
            }, |backup, owner| owner.remove(backup));
        assert_eq!(fs::read(&retained).unwrap(), b"original");
        assert_eq!(fs::read(&temporary).unwrap(), b"patched");
        if target.exists() || fs::read(&backup_name).ok().as_deref() != Some(b"foreign backup") {
            fs::write(fixture.path().join("observation.txt"), format!("published={published}; result={result:?}; foreign backup used as original")).unwrap();
            panic!("unowned backup restored; retained fixture: {}", fixture.keep().display());
        }
        assert!(!published);
        assert!(file_write_outcome_unknown(&result.unwrap_err()));
    }

    #[cfg(windows)]
    fn publication_parent_race(source: PublicationSource<'static>, label: &str) {
        let mut builder=tempfile::Builder::new();
        builder.prefix("write-race-");
        let fixture=match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent)=>builder.tempdir_in(parent).unwrap(),
            None=>builder.tempdir().unwrap(),
        };
        let root=fixture.path().join("workspace");
        let inside=root.join("inside");
        let retained=root.join("retained");
        let outside=fixture.path().join("outside");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sibling.txt"),b"outside sibling").unwrap();
        if !matches!(source,PublicationSource::Absent) {
            fs::write(inside.join("value.txt"),b"original").unwrap();
            fs::write(outside.join("value.txt"),b"original").unwrap();
        }
        let prepared=crate::workspace_write::prepare_parent(&inside.join("value.txt"),&root).unwrap();
        let redirected=match fs::rename(&inside,&retained) {
            Ok(())=>{ junction::create(&outside,&inside).unwrap(); true },
            Err(error)=>{ assert!(matches!(error.raw_os_error(),Some(5 | 32)),"{error:?}"); false },
        };
        let result=write_file_with_source_sync_atomic(prepared.as_path(),b"patched",source);
        if redirected {
            junction::delete(&inside).unwrap();
            fs::rename(&retained,&inside).unwrap();
        }
        let outside_unchanged=match source {
            PublicationSource::Absent=>!outside.join("value.txt").exists(),
            _=>fs::read(outside.join("value.txt")).unwrap()==b"original",
        };
        assert_eq!(fs::read(outside.join("sibling.txt")).unwrap(),b"outside sibling");
        if !outside_unchanged {
            fs::write(fixture.path().join("observation.txt"),format!("mode={label}; redirected={redirected}; result={result:?}")).unwrap();
            let location=fixture.keep();
            panic!("publication changed an outside target; retained fixture: {}",location.display());
        }
        if redirected { assert!(result.is_err()); }
        else {
            result.unwrap();
            assert_eq!(fs::read(inside.join("value.txt")).unwrap(),b"patched");
        }
        drop(prepared);
        fs::rename(&inside,root.join("released")).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn prepared_parent_keeps_existing_write_inside_workspace() {
        publication_parent_race(PublicationSource::Existing,"write");
    }

    #[cfg(windows)]
    #[test]
    fn prepared_parent_keeps_guarded_patch_inside_workspace() {
        publication_parent_race(PublicationSource::Matching(b"original"),"patch");
    }

    #[cfg(windows)]
    #[test]
    fn prepared_parent_keeps_absent_publication_inside_workspace() {
        publication_parent_race(PublicationSource::Absent,"create");
    }

    #[test]
    fn patch_temp_collision_preserves_unowned_file() {
        for marker in [FILE_WRITE_OUTCOME_UNKNOWN, "artifact publication outcome is unknown"] {
            let dir = tempfile::tempdir().unwrap();
            let named_directory = dir.path().join(marker);
            fs::create_dir(&named_directory).unwrap();
            let target = named_directory.join("target.txt");
            let temporary = named_directory.join("collision.tmp");
            fs::write(&target, "original").unwrap();
            fs::write(&temporary, "belongs to another operation").unwrap();

            let failure = publish_patch_file(&target, b"patched", &temporary, PublicationSource::Absent).unwrap_err();
            assert!(!failure.published);
            assert!(!failure.temporary_cleanup_unconfirmed);
            let error = file_write_publication_error(failure);
            assert!(!file_write_outcome_unknown(&error), "a path in an ordinary IO error is not an outcome marker: {error}");
            assert!(!crate::artifact_publication_outcome_unknown(&error), "an artifact marker in a file path is not publication uncertainty: {error}");
            assert_eq!(fs::read(&temporary).unwrap(), b"belongs to another operation");
            assert_eq!(fs::read(&target).unwrap(), b"original");
        }
    }

    #[cfg(windows)]
    #[test]
    fn atomic_replace_restores_original_after_partial_native_failure() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let temporary = root.path().join("temporary");
        fs::write(&target, b"original").unwrap();
        fs::write(&temporary, b"new").unwrap();
        let mut published = false;
        let error = replace_file_path_windows_with(&temporary, &target, &mut published, None,
            |_, target, backup| {
                fs::rename(target, backup)?;
                Err(std::io::Error::from_raw_os_error(1177))
            }, |path, owner| owner.remove(path)).unwrap_err();
        assert!(!published);
        assert!(!file_write_outcome_unknown(&error));
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(fs::read(&temporary).unwrap(), b"new");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn atomic_replace_partial_failure_never_overwrites_concurrent_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let temporary = root.path().join("temporary");
        fs::write(&target, b"original").unwrap();
        fs::write(&temporary, b"new").unwrap();
        let mut backup_path = None;
        let mut published = false;
        let error = replace_file_path_windows_with(&temporary, &target, &mut published, None,
            |_, target, backup| {
                fs::rename(target, backup)?;
                fs::write(target, b"concurrent")?;
                backup_path = Some(backup.to_path_buf());
                Err(std::io::Error::from_raw_os_error(1177))
            }, |path, owner| owner.remove(path)).unwrap_err();
        assert!(!published);
        assert!(file_write_outcome_unknown(&error));
        assert_eq!(fs::read(&target).unwrap(), b"concurrent");
        assert_eq!(fs::read(backup_path.unwrap()).unwrap(), b"original");
    }

    #[cfg(windows)]
    #[test]
    fn atomic_replace_reports_publication_when_backup_cleanup_is_locked() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let temporary = root.path().join("temporary");
        fs::write(&target, b"original").unwrap();
        fs::write(&temporary, b"new").unwrap();
        let mut locker = None;
        let mut published = false;
        let error = replace_file_path_windows_with(&temporary, &target, &mut published, None,
            replace_file_windows_native, |backup, owner| {
                locker = Some(fs::OpenOptions::new().read(true).share_mode(3).open(backup)?);
                owner.remove(backup)
            }).unwrap_err();
        assert!(published);
        assert!(file_write_outcome_unknown(&error));
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(!temporary.exists());
        drop(locker);
    }

    #[test]
    fn atomic_write_errors_preserve_uncertainty_and_cleanup_observations() {
        let error = file_write_publication_error(PatchPublicationFailure {
            error: AppError::Internal(format!("{FILE_WRITE_OUTCOME_UNKNOWN}; original backup retained")),
            published: true, content_verified: true, temporary_cleanup_unconfirmed: false,
        });
        assert!(file_write_outcome_unknown(&error));
        assert!(error.to_string().contains("original backup retained"));
        let error = file_write_publication_error(PatchPublicationFailure {
            error: AppError::Internal("write failed".into()),
            published: false, content_verified: false, temporary_cleanup_unconfirmed: true,
        });
        assert!(file_write_outcome_unknown(&error));
        let rejected = file_write_publication_error(AppError::Forbidden("denied".into()).into());
        assert!(matches!(rejected, AppError::Forbidden(_)));
        assert!(!file_write_outcome_unknown(&rejected));
    }

    #[tokio::test]
    async fn remote_image_body_enforces_limit_without_content_length() {
        for size in [0, MAX_REMOTE_IMAGE_SIZE, MAX_REMOTE_IMAGE_SIZE + 1] {
            let response = axum::http::Response::new(vec![b'x'; size]);
            assert!(!response.headers().contains_key("content-length"));
            let result = read_remote_image_body(response.into()).await;
            if size <= MAX_REMOTE_IMAGE_SIZE {
                assert_eq!(result.unwrap().len(), size);
            } else {
                assert!(result.unwrap_err().contains("size limit"));
            }
        }
    }

    #[tokio::test]
    async fn zip_registration_cleans_up_errors_and_preserves_reused_ids() {
        use crate::traits::IFileService;
        let dir = tempfile::tempdir().unwrap();
        let svc = FileService::new(Arc::new(NullBroadcaster), vec![dir.path().to_path_buf()]);
        let output = dir.path().join("output.zip");
        fs::create_dir(&output).unwrap(); // Validation succeeds; opening as a file fails.
        assert!(svc.create_zip(output.to_str().unwrap(), vec![], Some("id".into())).await.is_err());
        assert!(!svc.cancel_zip("id").await);

        let old_flag = Arc::new(AtomicBool::new(false));
        svc.zip_cancellations.insert("id".into(), old_flag.clone());
        let guard = ZipCancellationGuard {
            cancellations: &svc.zip_cancellations,
            request_id: Some("id"),
            cancelled: old_flag.clone(),
        };
        let result = svc.create_zip(output.to_str().unwrap(), vec![], Some("id".into())).await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
        assert!(svc.cancel_zip("id").await);
        let new_flag = Arc::new(AtomicBool::new(false));
        svc.zip_cancellations.insert("id".into(), new_flag.clone());
        drop(guard);
        assert!(old_flag.load(Ordering::Relaxed));
        assert!(!new_flag.load(Ordering::Relaxed));
        assert!(svc.cancel_zip("id").await);
        assert!(new_flag.load(Ordering::Relaxed));
        let detached_flag = Arc::new(AtomicBool::new(false));
        drop(ZipCancellationGuard {
            cancellations: &svc.zip_cancellations,
            request_id: None,
            cancelled: detached_flag.clone(),
        });
        assert!(detached_flag.load(Ordering::Relaxed));

        let source = dir.path().join("source.txt");
        fs::write(&source, "preserve source").unwrap();
        let entries = vec![ZipEntry::Disk {
            name: "source.txt".into(), file_path: source.to_string_lossy().into_owned(),
        }];
        assert!(svc.create_zip(source.to_str().unwrap(), entries, None).await.is_err());
        assert_eq!(fs::read(&source).unwrap(), b"preserve source");
    }

    #[test]
    fn zip_single_entry_checks_cancellation_between_chunks() {
        struct CancellingReader<'a>(&'a AtomicBool, usize);
        impl Read for CancellingReader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.1 += 1;
                assert_eq!(self.1, 1, "must not read another chunk after cancellation");
                assert!(buffer.len() <= 64 * 1024);
                buffer.fill(b'x');
                self.0.store(true, Ordering::Relaxed);
                Ok(buffer.len())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let file = fs::File::create(dir.path().join("chunked.zip")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let cancelled = AtomicBool::new(false);
        assert!(!write_zip_entry(
            &mut zip, "data", CancellingReader(&cancelled, 0), &cancelled,
            zip::write::SimpleFileOptions::default(),
        ).unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_write_rejects_escape_links_but_preserves_in_root_links() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("outside.txt");
        let inside = workspace.path().join("inside.txt");
        fs::write(&secret, "outside").unwrap();
        fs::write(&inside, "inside").unwrap();
        let svc = make_service();
        let scope = patch_scope(workspace.path());
        for (name, target, allowed) in [
            ("escape", secret.clone(), false),
            ("dangling", outside.path().join("new.txt"), false),
            ("local", inside.clone(), true),
        ] {
            std::os::unix::fs::symlink(&target, workspace.path().join(name)).unwrap();
            let result = svc.write_file_for_agent_session(&scope, name, b"changed").await;
            assert_eq!(result.is_ok(), allowed, "{name}: {result:?}");
        }
        assert_eq!(fs::read(&secret).unwrap(), b"outside");
        assert!(!outside.path().join("new.txt").exists());
        assert_eq!(fs::read(&inside).unwrap(), b"changed");
        // The trusted local-owner authority still has OS-user access.
        use crate::traits::IFileService;
        svc.write_file_scoped(
            "owner-1", workspace.path().join("escape").to_str().unwrap(), b"owner write",
            workspace.path().to_str().unwrap(), &PathAuthority::Unrestricted,
        ).await.unwrap();
        assert_eq!(fs::read(&secret).unwrap(), b"owner write");
    }

    #[cfg(unix)]
    #[test]
    fn copy_rejects_final_and_parent_links_outside_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = workspace.path().join("source.txt");
        let target = outside.path().join("target.txt");
        fs::write(&source, "source").unwrap();
        fs::write(&target, "original").unwrap();
        std::os::unix::fs::symlink(&target, workspace.path().join("final")).unwrap();
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("parent")).unwrap();
        for relative in ["final", "parent/new/nested.txt"] {
            assert!(copy_single_file_sync(&source, &workspace.path().join(relative), workspace.path()).is_err());
        }
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(!outside.path().join("new").exists());
    }

    #[test]
    fn build_dir_tree_sync_lists_files_and_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::write(dir.path().join("b.rs"), "fn main(){}").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/c.txt"), "nested").unwrap();

        let result = build_dir_tree_sync(dir.path(), dir.path()).unwrap();

        // sub/ should come first (directories first)
        assert_eq!(result[0].name, "sub");
        assert!(result[0].is_dir);
        // sub/ should have c.txt as child
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].name, "c.txt");

        // Then files alphabetically
        assert_eq!(result[1].name, "a.txt");
        assert!(!result[1].is_dir);
        assert_eq!(result[2].name, "b.rs");
    }

    #[test]
    fn build_dir_tree_sync_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = build_dir_tree_sync(dir.path(), dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn build_dir_tree_sync_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("folder");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("file.txt"), "data").unwrap();

        let result = build_dir_tree_sync(dir.path(), dir.path()).unwrap();

        assert_eq!(result[0].relative_path, "folder");
        assert_eq!(result[0].children[0].relative_path, "folder/file.txt");
    }

    #[test]
    fn build_dir_tree_sync_nonexistent_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("nonexistent");
        let result = build_dir_tree_sync(&fake, dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn list_workspace_files_sync_basic() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/b.txt"), "world").unwrap();

        let files = list_workspace_files_sync(&dir.path().canonicalize().unwrap()).unwrap();

        assert_eq!(files.len(), 2);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
    }

    #[test]
    fn inventory_does_not_import_parent_ignore_rules() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(fixture.path().join(".ignore"), b"visible.txt\n").unwrap();
        fs::write(root.join("visible.txt"), b"inside").unwrap();
        let files = list_workspace_files_sync(&root.canonicalize().unwrap()).unwrap();
        assert_eq!(inventory_names(&files), ["visible.txt"],
            "parent rules are outside the selected workspace; fixture: {}", fixture.path().display());
    }

    #[test]
    fn inventory_preserves_hidden_files_ignore_precedence_and_nested_git_boundaries() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        for path in [".git/info", "nested", "repository/.git/info"] {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        fs::write(root.join(".ignore"), b"priority.txt\n!override.log\n").unwrap();
        fs::write(root.join(".gitignore"), b"*.log\nnested/*.txt\n!keep.txt\n").unwrap();
        fs::write(root.join(".git/info/exclude"), b"excluded.txt\nkeep.txt\n").unwrap();
        fs::write(root.join("nested/.gitignore"), b"!keep.txt\n!priority.txt\n").unwrap();
        fs::write(root.join("repository/.gitignore"), b"own.txt\n").unwrap();
        for path in ["keep.txt", "skip.log", "priority.txt", "override.log", ".hidden", "excluded.txt",
            "nested/keep.txt", "nested/no.txt", "nested/priority.txt", "nested/code.rs", "repository/allow.log", "repository/own.txt"] {
            fs::write(root.join(path), b"inside").unwrap();
        }
        let files = list_workspace_files_sync(&root.canonicalize().unwrap()).unwrap();
        let paths = files.iter().map(|file| file.relative_path.as_str()).collect::<BTreeSet<_>>();
        for path in ["keep.txt", "override.log", ".hidden", "nested/keep.txt", "nested/code.rs", "repository/allow.log", ".gitignore"] {
            assert!(paths.contains(path), "required entry missing: {path}; {paths:?}");
        }
        for path in ["skip.log", "priority.txt", "excluded.txt", "nested/no.txt", "nested/priority.txt", "repository/own.txt"] {
            assert!(!paths.contains(path), "ignore precedence changed: {path}; {paths:?}");
        }
    }

    #[test]
    fn inventory_reads_in_root_gitdir_and_commondir_but_rejects_outside_targets() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir_all(root.join("metadata/worktree")).unwrap();
        fs::create_dir_all(root.join("metadata/common/info")).unwrap();
        fs::write(root.join(".git"), b"gitdir: metadata/worktree\n").unwrap();
        fs::write(root.join("metadata/worktree/commondir"), b"../common\n").unwrap();
        fs::write(root.join("metadata/common/info/exclude"), b"ignored.txt\n").unwrap();
        fs::write(root.join("visible.txt"), b"inside").unwrap();
        fs::write(root.join("ignored.txt"), b"inside").unwrap();
        let canonical = root.canonicalize().unwrap();
        let files = list_workspace_files_sync(&canonical).unwrap();
        let names = inventory_names(&files);
        assert!(names.contains(&"visible.txt".to_owned()));
        assert!(!names.contains(&"ignored.txt".to_owned()));
        let outside = fixture.path().join("outside");
        fs::create_dir_all(outside.join("info")).unwrap();
        fs::write(outside.join("info/exclude"), b"visible.txt\n").unwrap();
        fs::write(root.join("metadata/worktree/commondir"), b"../../../outside\n").unwrap();
        assert!(matches!(list_workspace_files_sync(&canonical), Err(AppError::Forbidden(_))));
        fs::write(root.join(".git"), b"gitdir: ../outside\n").unwrap();
        assert!(matches!(list_workspace_files_sync(&canonical), Err(AppError::Forbidden(_))));
        assert_eq!(fs::read(outside.join("info/exclude")).unwrap(), b"visible.txt\n");
    }

    #[cfg(windows)]
    #[test]
    fn inventory_child_replacement_cannot_supply_entries_or_ignore_rules() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        let child = root.join("child");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(child.join("inside.txt"), b"inside").unwrap();
        fs::write(outside.join(".ignore"), b"*\n").unwrap();
        fs::write(outside.join("outside-only.txt"), b"outside").unwrap();
        let canonical = root.canonicalize().unwrap();
        let canonical_child = child.canonicalize().unwrap();
        let mut redirected = false;
        let result = list_workspace_files_sync_with_hooks(&canonical, || {}, |path| {
            if path == canonical_child {
                fs::rename(&child, fixture.path().join("retained-child")).unwrap();
                junction::create(&outside, &child).unwrap();
                redirected = true;
            }
        });
        assert!(redirected);
        assert!(result.is_err(), "partial or outside listing must not be cached: {result:?}");
        assert_eq!(fs::read(outside.join("outside-only.txt")).unwrap(), b"outside");
        assert_eq!(fs::read(fixture.path().join("retained-child/inside.txt")).unwrap(), b"inside");
    }

    #[test]
    fn inventory_rejects_unreadable_ignore_text_instead_of_caching_a_partial_policy() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join(".gitignore"), b"visible.txt\n\xff\n").unwrap();
        fs::write(root.join("visible.txt"), b"inside").unwrap();
        let result = list_workspace_files_sync(&root.canonicalize().unwrap());
        assert!(result.is_err(), "invalid ignore text must be explicit: {result:?}; fixture: {}", fixture.path().display());
    }

    #[tokio::test]
    async fn inventory_rule_failure_releases_cache_and_allows_a_corrected_retry() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join(".ignore"), b"\xff").unwrap();
        fs::write(root.join("visible.txt"), b"inside").unwrap();
        let service = make_service();
        let authority = PathAuthority::Workspace(root.clone());
        let key = root.to_string_lossy().into_owned();
        assert!(service.list_workspace_files_impl(&key, &authority).await.is_err());
        assert!(service.workspace_files_cache.is_empty());
        fs::write(root.join(".ignore"), b"ignored.txt\n").unwrap();
        fs::write(root.join("ignored.txt"), b"inside").unwrap();
        let files = service.list_workspace_files_impl(&key, &authority).await.unwrap();
        assert_eq!(inventory_names(&files), [".ignore", "visible.txt"]);
    }

    #[test]
    fn inventory_file_budget_rejects_partial_success_and_accepts_the_exact_limit() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        for index in 0..MAX_WORKSPACE_FILES {
            fs::write(root.join(format!("{index:05}.txt")), b"").unwrap();
        }
        let extra = root.join("extra.txt");
        fs::write(&extra, b"extra").unwrap();
        let canonical = root.canonicalize().unwrap();
        for _ in 0..20 {
            assert!(matches!(list_workspace_files_sync(&canonical), Err(AppError::Conflict(_))));
        }
        fs::rename(&extra, fixture.path().join("retained-extra.txt")).unwrap();
        for _ in 0..20 {
            assert_eq!(list_workspace_files_sync(&canonical).unwrap().len(), MAX_WORKSPACE_FILES);
        }
    }

    #[cfg(windows)]
    #[test]
    fn inventory_cannot_read_git_exclude_through_an_outside_junction() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        let outside = fixture.path().join("outside-git");
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(outside.join("info")).unwrap();
        fs::write(outside.join("info/exclude"), b"visible.txt\n").unwrap();
        fs::write(root.join("visible.txt"), b"inside").unwrap();
        junction::create(&outside, root.join(".git")).unwrap();
        let result = list_workspace_files_sync(&root.canonicalize().unwrap());
        assert_eq!(fs::read(outside.join("info/exclude")).unwrap(), b"visible.txt\n");
        assert!(result.is_err(), "unbound Git rules must not supply inventory policy: {result:?}; fixture: {}", fixture.path().display());
    }

    #[cfg(windows)]
    #[test]
    fn inventory_root_replaced_after_validation_cannot_supply_outside_names() {
        let fixture = inventory_fixture();
        let root = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("inside.txt"), b"inside").unwrap();
        fs::write(outside.join("outside-only.txt"), b"outside").unwrap();
        let canonical = root.canonicalize().unwrap();
        let result = list_workspace_files_sync_with_hook(&canonical, || {
            fs::rename(&root, fixture.path().join("retained-original")).unwrap();
            junction::create(&outside, &root).unwrap();
        });
        assert_eq!(fs::read(outside.join("outside-only.txt")).unwrap(), b"outside");
        assert!(result.is_err(), "replaced root supplied an inventory: {result:?}; fixture: {}", fixture.path().display());
    }

    #[test]
    fn list_workspace_files_sync_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(dir.path().join("kept.txt"), "keep").unwrap();
        fs::write(dir.path().join("ignored.txt"), "skip").unwrap();

        let files = list_workspace_files_sync(&dir.path().canonicalize().unwrap()).unwrap();

        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"kept.txt"));
        assert!(names.contains(&".gitignore"));
        assert!(!names.contains(&"ignored.txt"));
    }

    #[test]
    fn list_workspace_files_sync_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let files = list_workspace_files_sync(&dir.path().canonicalize().unwrap()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn list_workspace_files_sync_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main(){}").unwrap();

        let files = list_workspace_files_sync(&dir.path().canonicalize().unwrap()).unwrap();
        let main_file = files.iter().find(|f| f.name == "main.rs").unwrap();

        assert_eq!(main_file.relative_path, "src/main.rs");
    }

    #[test]
    fn list_workspace_files_sync_excludes_owner_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".nomifun/artifacts")).unwrap();
        fs::write(dir.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        fs::write(dir.path().join("visible.txt"), "visible").unwrap();

        let files = list_workspace_files_sync(&dir.path().canonicalize().unwrap()).unwrap();
        assert_eq!(
            files
                .iter()
                .map(|file| file.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["visible.txt"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_workspace_files_sync_skips_directory_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("builtin-skills/auto-inject/nomifun-skills");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\ndescription: test\n---\nbody").unwrap();

        let workspace = dir.path().join("workspace/.claude/skills");
        fs::create_dir_all(&workspace).unwrap();
        std::os::unix::fs::symlink(&skill_dir, workspace.join("nomifun-skills")).unwrap();

        let files = list_workspace_files_sync(&dir.path().join("workspace").canonicalize().unwrap()).unwrap();

        assert!(
            files.iter().all(|f| f.name != "nomifun-skills"),
            "directory symlink should not be surfaced as a file: {files:?}"
        );
    }

    #[test]
    fn get_file_metadata_sync_text_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "hello world").unwrap();

        let meta = get_file_metadata_sync(&file).unwrap();
        assert_eq!(meta.name, "hello.txt");
        assert_eq!(meta.size, 11);
        assert_eq!(meta.mime_type, "text/plain");
        assert!(!meta.is_directory);
        assert!(meta.last_modified > 0);
    }

    #[test]
    fn get_file_metadata_sync_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("mydir");
        fs::create_dir(&sub).unwrap();

        let meta = get_file_metadata_sync(&sub).unwrap();
        assert_eq!(meta.name, "mydir");
        assert!(meta.is_directory);
        assert_eq!(meta.mime_type, "inode/directory");
    }

    #[test]
    fn get_file_metadata_sync_rust_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        fs::write(&file, "pub fn foo() {}").unwrap();

        let meta = get_file_metadata_sync(&file).unwrap();
        assert_eq!(meta.name, "lib.rs");
        // rust files should get a reasonable mime type
        assert!(!meta.mime_type.is_empty());
    }

    #[test]
    fn get_file_metadata_sync_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("missing.txt");
        let result = get_file_metadata_sync(&fake);
        assert!(result.is_err());
    }

    #[test]
    fn get_file_metadata_sync_image_mime() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("icon.png");
        fs::write(&png, [0x89, 0x50, 0x4E, 0x47]).unwrap();

        let meta = get_file_metadata_sync(&png).unwrap();
        assert_eq!(meta.mime_type, "image/png");
    }

    #[test]
    fn get_file_metadata_sync_unknown_extension() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.xyz123");
        fs::write(&file, "binary data").unwrap();

        let meta = get_file_metadata_sync(&file).unwrap();
        assert_eq!(meta.mime_type, "application/octet-stream");
    }

    // -- read_file_sync tests (task 7.4) --

    #[test]
    fn read_file_sync_normal_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "hello world").unwrap();

        let result = read_file_sync(&file).unwrap();
        assert_eq!(result.as_deref(), Some("hello world"));
    }

    #[test]
    fn read_file_sync_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("empty.txt");
        fs::write(&file, "").unwrap();

        let result = read_file_sync(&file).unwrap();
        assert_eq!(result.as_deref(), Some(""));
    }

    #[test]
    fn read_file_sync_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("missing.txt");

        let result = read_file_sync(&fake).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_file_sync_rejects_directory() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("subdir");
        fs::create_dir(&folder).unwrap();

        let err = read_file_sync(&folder).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        assert!(err.to_string().contains("is a directory"));
    }

    // -- validate_file_for_read tests --

    #[test]
    fn validate_file_for_read_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("valid.txt");
        fs::write(&file, "data").unwrap();

        let result = validate_file_for_read(&file).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn validate_file_for_read_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("nope.txt");

        let result = validate_file_for_read(&fake).unwrap();
        assert!(result.is_none());
    }

    // -- write_file_sync tests --

    #[test]
    fn write_file_sync_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("output.txt");

        let ok = write_file_sync(&file, b"hello").unwrap();
        assert!(ok);
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
    }

    #[test]
    fn write_file_sync_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("overwrite.txt");
        fs::write(&file, "old").unwrap();

        let ok = write_file_sync(&file, b"new content").unwrap();
        assert!(ok);
        assert_eq!(fs::read_to_string(&file).unwrap(), "new content");
    }

    #[test]
    fn write_file_sync_binary() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.bin");
        let data = vec![0x00, 0xFF, 0xAB];

        let ok = write_file_sync(&file, &data).unwrap();
        assert!(ok);
        assert_eq!(fs::read(&file).unwrap(), data);
    }

    // -- remove_entry_sync tests (task 7.5) --

    fn inventory_fixture() -> tempfile::TempDir {
        let mut builder = tempfile::Builder::new();
        builder.prefix("inventory-cache-");
        match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent) => builder.disable_cleanup(true).tempdir_in(parent).unwrap(),
            None => builder.tempdir().unwrap(),
        }
    }

    fn inventory_names(files: &[WorkspaceFlatFile]) -> Vec<String> {
        let mut names = files.iter().map(|file| file.name.clone()).collect::<Vec<_>>();
        names.sort();
        names
    }

    #[tokio::test]
    async fn inventory_invalidation_rejects_an_in_flight_stale_scan() {
        let fixture = inventory_fixture();
        fs::write(fixture.path().join("old.txt"), b"old").unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let key = root.to_string_lossy().into_owned();
        let service = make_service();
        let authority = PathAuthority::Workspace(root.clone());
        let mut fired = false;
        let observed = service.list_workspace_files_with_hook(&key, &authority, || {
            if !fired {
                fs::write(root.join("new.txt"), b"new").unwrap();
                service.invalidate_cache(&key);
                fired = true;
            }
            async {}
        }).await.unwrap();
        let next = service.list_workspace_files_impl(&key, &authority).await.unwrap();
        let expected = vec!["new.txt".to_owned(), "old.txt".to_owned()];
        if inventory_names(&observed) != expected || inventory_names(&next) != expected {
            fs::write(root.join("observation.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "first": inventory_names(&observed), "next": inventory_names(&next),
            })).unwrap()).unwrap();
            panic!("invalidation was lost; retained fixture: {}", fixture.keep().display());
        }
        assert!(fired);
    }

    #[tokio::test]
    async fn inventory_old_scan_cannot_overwrite_a_newer_completed_scan() {
        let fixture = inventory_fixture();
        fs::write(fixture.path().join("old.txt"), b"old").unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let key = root.to_string_lossy().into_owned();
        let service = make_service();
        let authority = PathAuthority::Workspace(root.clone());
        let fired = AtomicBool::new(false);
        let observed = service.list_workspace_files_with_hook(&key, &authority, || async {
            if !fired.swap(true, Ordering::SeqCst) {
                fs::write(root.join("new.txt"), b"new").unwrap();
                service.invalidate_cache(&key);
                let newer = service.list_workspace_files_impl(&key, &authority).await.unwrap();
                assert_eq!(inventory_names(&newer), ["new.txt", "old.txt"]);
            }
        }).await.unwrap();
        let next = service.list_workspace_files_impl(&key, &authority).await.unwrap();
        if inventory_names(&observed) != ["new.txt", "old.txt"] || inventory_names(&next) != ["new.txt", "old.txt"] {
            fs::write(root.join("observation.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "older_returned": inventory_names(&observed), "after_newer_scan": inventory_names(&next),
            })).unwrap()).unwrap();
            panic!("old scan replaced newer cache; retained fixture: {}", fixture.keep().display());
        }
    }

    #[tokio::test]
    async fn inventory_repeated_invalidation_is_bounded_and_does_not_cache_partial_results() {
        let fixture = inventory_fixture();
        fs::write(fixture.path().join("source.txt"), b"source").unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let key = root.to_string_lossy().into_owned();
        let service = make_service();
        let authority = PathAuthority::Workspace(root);
        let mut scans = 0;
        let result = service.list_workspace_files_with_hook(&key, &authority, || {
            scans += 1;
            service.invalidate_cache(&key);
            async {}
        }).await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
        assert_eq!(scans, 2);
        assert!(service.workspace_files_cache.is_empty());
        assert_eq!(inventory_names(&service.list_workspace_files_impl(&key, &authority).await.unwrap()), ["source.txt"]);
    }

    #[tokio::test]
    async fn inventory_canceled_scan_releases_its_unpublished_slot() {
        let fixture = inventory_fixture();
        fs::write(fixture.path().join("old.txt"), b"old").unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let key = root.to_string_lossy().into_owned();
        let service = Arc::new(make_service());
        let (started, observed) = tokio::sync::oneshot::channel();
        let task = {
            let (service, key, root) = (service.clone(), key.clone(), root.clone());
            tokio::spawn(async move {
                let mut started = Some(started);
                service.list_workspace_files_with_hook(&key, &PathAuthority::Workspace(root), || {
                    if let Some(started) = started.take() { started.send(()).unwrap(); }
                    std::future::pending::<()>()
                }).await
            })
        };
        observed.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(service.workspace_files_cache.is_empty(), "canceled readers must not retain unpublished roots");
        fs::write(root.join("new.txt"), b"new").unwrap();
        assert_eq!(inventory_names(&service.list_workspace_files_impl(&key, &PathAuthority::Workspace(root)).await.unwrap()), ["new.txt", "old.txt"]);
    }

    #[tokio::test]
    async fn inventory_invalidation_is_isolated_to_its_root() {
        let fixture = inventory_fixture();
        let first = fixture.path().join("first");
        let second = fixture.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        fs::write(first.join("old.txt"), b"old").unwrap();
        fs::write(second.join("old.txt"), b"old").unwrap();
        let service = make_service();
        let authority = PathAuthority::Confined(vec![first.clone(), second.clone()]);
        service.list_workspace_files_impl(first.to_str().unwrap(), &authority).await.unwrap();
        service.list_workspace_files_impl(second.to_str().unwrap(), &authority).await.unwrap();
        fs::write(first.join("new.txt"), b"new").unwrap();
        fs::write(second.join("new.txt"), b"new").unwrap();
        service.invalidate_cache(&fs::canonicalize(&first).unwrap().to_string_lossy());
        assert_eq!(inventory_names(&service.list_workspace_files_impl(first.to_str().unwrap(), &authority).await.unwrap()), ["new.txt", "old.txt"]);
        assert_eq!(inventory_names(&service.list_workspace_files_impl(second.to_str().unwrap(), &authority).await.unwrap()), ["old.txt"]);
    }

    #[tokio::test]
    async fn inventory_native_scope_invalidates_related_roots_without_string_prefix_collisions() {
        let fixture = inventory_fixture();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let watched = root.join("workspace");
        let nested = watched.join("nested");
        let sibling = root.join("workspace-other");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(&sibling).unwrap();
        fs::write(nested.join("old.txt"), b"old").unwrap();
        fs::write(sibling.join("keep.txt"), b"unrelated").unwrap();
        let service = make_service();
        let authority = PathAuthority::Workspace(root.clone());
        for path in [&root, &watched, &nested, &sibling] {
            service.list_workspace_files_impl(path.to_str().unwrap(), &authority).await.unwrap();
        }
        let sibling_slot = service.workspace_files_cache.get(sibling.to_str().unwrap()).unwrap().clone();
        fs::write(nested.join("new.txt"), b"new").unwrap();
        service.invalidate_caches_for_path(&watched);
        assert!(!service.workspace_files_cache.contains_key(root.to_str().unwrap()));
        assert!(!service.workspace_files_cache.contains_key(watched.to_str().unwrap()));
        assert!(!service.workspace_files_cache.contains_key(nested.to_str().unwrap()));
        assert!(Arc::ptr_eq(&sibling_slot, service.workspace_files_cache.get(sibling.to_str().unwrap()).unwrap().value()));
        assert_eq!(inventory_names(&service.list_workspace_files_impl(nested.to_str().unwrap(), &authority).await.unwrap()), ["new.txt", "old.txt"]);
        // Retired paths still identify which parent inventory needs re-reading.
        fs::remove_file(nested.join("new.txt")).unwrap();
        service.invalidate_caches_for_path(&nested.join("new.txt"));
        assert_eq!(inventory_names(&service.list_workspace_files_impl(nested.to_str().unwrap(), &authority).await.unwrap()), ["old.txt"]);
    }

    struct InventoryReadingEvents {
        service: std::sync::Mutex<std::sync::Weak<FileService>>,
        root: PathBuf,
        observations: std::sync::Mutex<Vec<Vec<String>>>,
    }

    impl UserEventSink for InventoryReadingEvents {
        fn send_to_user(&self, _: &str, _: WebSocketMessage<serde_json::Value>) {
            let service = self.service.lock().unwrap().upgrade().unwrap();
            let root = self.root.clone();
            let runtime = tokio::runtime::Handle::current();
            // A separate subscriber reads the public API while delivery is in
            // progress, making the event/cache ordering deterministic.
            let names = std::thread::spawn(move || runtime.block_on(async move {
                use crate::IFileService;
                inventory_names(&service.list_workspace_files(root.to_str().unwrap()).await.unwrap())
            })).join().unwrap();
            self.observations.lock().unwrap().push(names);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inventory_is_invalidated_before_write_and_delete_events() {
        use crate::IFileService;
        let fixture = inventory_fixture();
        fs::write(fixture.path().join("old.txt"), b"old").unwrap();
        let events = Arc::new(InventoryReadingEvents { service: std::sync::Mutex::new(std::sync::Weak::new()),
            root: fixture.path().to_path_buf(), observations: std::sync::Mutex::new(Vec::new()) });
        let service = Arc::new(FileService::new(events.clone(), vec![fixture.path().to_path_buf()]));
        *events.service.lock().unwrap() = Arc::downgrade(&service);
        let binding = crate::workspace_binding(nomifun_common::generate_id(), "cache-binding", "workspace", "owner",
            [crate::WORKSPACE_READ_OPERATION, crate::WORKSPACE_WRITE_OPERATION, crate::WORKSPACE_DELETE_OPERATION], fixture.path()).unwrap();
        service.list_workspace_files(fixture.path().to_str().unwrap()).await.unwrap();
        service.write_file_for_agent_session(&binding, "new.txt", b"new").await.unwrap();
        service.list_workspace_files(fixture.path().to_str().unwrap()).await.unwrap();
        service.remove_entry_for_agent_session(&binding, "old.txt").await.unwrap();
        let observed = events.observations.lock().unwrap().clone();
        if observed != [vec!["new.txt".to_owned(), "old.txt".to_owned()], vec!["new.txt".to_owned()]] {
            fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&observed).unwrap()).unwrap();
            panic!("subscriber received stale inventory after changes; retained fixture: {}", fixture.keep().display());
        }
        assert!(!fixture.path().join("old.txt").exists());
        assert_eq!(fs::read(fixture.path().join("new.txt")).unwrap(), b"new");
    }

    #[cfg(windows)]
    fn scoped_delete_parent_race(directory: bool) {
        let fixture = cleanup_race_fixture();
        let root = fixture.path().join("workspace");
        let parent = root.join("parent");
        let retained = root.join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir(&outside).unwrap();
        for base in [&parent, &outside] {
            if directory { fs::create_dir(base.join("target")).unwrap(); }
            fs::write(base.join(if directory { "target/keep.txt" } else { "target" }), b"keep").unwrap();
        }
        let canonical = fs::canonicalize(parent.join("target")).unwrap();
        let result = remove_entry_scoped_sync_with_hooks(&canonical, &PathAuthority::Workspace(root.clone()), || {
            fs::rename(&parent, &retained).unwrap();
            junction::create(&outside, &parent).unwrap();
        }, || {});
        junction::delete(&parent).unwrap();
        fs::rename(&retained, &parent).unwrap();
        let relative = if directory { "target/keep.txt" } else { "target" };
        if fs::read(outside.join(relative)).ok().as_deref() != Some(b"keep") {
            fs::write(fixture.path().join("observation.txt"), format!("directory={directory}; result={result:?}")).unwrap();
            panic!("scoped deletion escaped through a replaced parent; retained fixture: {}", fixture.keep().display());
        }
        assert!(result.is_err());
        assert_eq!(fs::read(parent.join(relative)).unwrap(), b"keep");
        remove_entry_scoped_sync_with_hooks(&canonical, &PathAuthority::Workspace(root), || {}, || {}).unwrap();
        assert!(!parent.join("target").exists());
        assert_eq!(fs::read(outside.join(relative)).unwrap(), b"keep");
    }

    #[cfg(windows)]
    #[test]
    fn scoped_delete_rejects_file_parent_escape() { scoped_delete_parent_race(false); }

    #[cfg(windows)]
    #[test]
    fn scoped_delete_rejects_directory_parent_escape() { scoped_delete_parent_race(true); }

    #[cfg(windows)]
    #[test]
    fn scoped_delete_keeps_opened_entries_inside_their_root() {
        for directory in [false, true] {
            for posix in [false, true] {
                let fixture = cleanup_race_fixture();
                let root = fixture.path().join("workspace");
                let parent = root.join("parent");
                let outside = fixture.path().join("outside");
                let moved = outside.join("moved");
                fs::create_dir_all(&parent).unwrap();
                fs::create_dir(&outside).unwrap();
                fs::write(outside.join("sentinel"), b"outside").unwrap();
                if directory { fs::create_dir(parent.join("target")).unwrap(); }
                let relative = if directory { "target/value.txt" } else { "target" };
                fs::write(parent.join(relative), b"original").unwrap();
                let canonical = fs::canonicalize(parent.join("target")).unwrap();
                let mut relocated = false;
                let result = remove_entry_scoped_sync_with_hooks(&canonical, &PathAuthority::Workspace(root), || {}, || {
                    let rename = if posix { crate::windows_test_support::rename_with_posix_semantics(&parent, &moved, false) }
                        else { fs::rename(&parent, &moved) };
                    match rename {
                        Ok(()) => relocated = true,
                        Err(error) => assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}")
                    }
                });
                if relocated && fs::read(moved.join(relative)).ok().as_deref() != Some(b"original") {
                    fs::write(fixture.path().join("observation.txt"), format!("directory={directory}; posix={posix}; result={result:?}")).unwrap();
                    panic!("delete followed an opened ancestor outside; retained fixture: {}", fixture.keep().display());
                }
                assert!(!relocated, "the deletion handle must keep its ancestor inside the bound root");
                result.unwrap();
                assert!(!parent.join("target").exists());
                assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
                crate::windows_test_support::rename_with_posix_semantics(&parent, &moved, false).unwrap();
            }
        }
    }

    #[cfg(windows)]
    fn directory_delete_name_race(posix: bool) {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target");
        let retained = fixture.path().join("retained");
        let foreign = fixture.path().join("foreign");
        fs::create_dir(&target).unwrap();
        fs::create_dir(&foreign).unwrap();
        fs::write(target.join("old.txt"), b"original").unwrap();
        fs::write(foreign.join("keep.txt"), b"foreign").unwrap();
        let mut swapped = false;
        let result = remove_entry_sync_with_hook(&target, || {
            let rename = if posix {
                crate::windows_test_support::rename_with_posix_semantics(&target, &retained, false)
            } else { fs::rename(&target, &retained) };
            match rename {
                Ok(()) => { fs::rename(&foreign, &target).unwrap(); swapped = true; }
                Err(error) => assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}")
            }
        });
        let foreign_name = if swapped { &target } else { &foreign };
        if fs::read(foreign_name.join("keep.txt")).ok().as_deref() != Some(b"foreign") {
            fs::write(fixture.path().join("observation.txt"), format!("posix={posix}; swapped={swapped}; result={result:?}")).unwrap();
            panic!("recursive deletion removed a concurrent directory; retained fixture: {}", fixture.keep().display());
        }
        result.unwrap();
        assert!(!swapped);
        assert!(!target.exists());
        assert!(!retained.exists());
        fs::rename(&foreign, &target).unwrap();
        assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"foreign");
    }

    #[cfg(windows)]
    #[test]
    fn directory_delete_preserves_a_replacement_after_access_check() { directory_delete_name_race(false); }

    #[cfg(windows)]
    #[test]
    fn directory_delete_preserves_a_posix_replacement_after_access_check() { directory_delete_name_race(true); }

    #[cfg(windows)]
    fn file_delete_name_race(posix: bool) {
        let fixture = cleanup_race_fixture();
        let target = fixture.path().join("target.txt");
        let original = fixture.path().join("original.txt");
        let foreign = fixture.path().join("foreign.txt");
        fs::write(&target, b"delete me").unwrap();
        fs::write(&foreign, b"keep me").unwrap();
        let mut swapped = false;
        let result = remove_entry_sync_with_hook(&target, || {
            let rename = if posix {
                crate::windows_test_support::rename_with_posix_semantics(&target, &original, false)
            } else {
                fs::rename(&target, &original)
            };
            match rename {
                Ok(()) => { fs::rename(&foreign, &target).unwrap(); swapped = true; }
                Err(error) => assert_eq!(error.raw_os_error(), Some(32)),
            }
        });
        let foreign_preserved = if swapped { fs::read(&target) } else { fs::read(&foreign) };
        if foreign_preserved.ok().as_deref() != Some(b"keep me") {
            fs::write(fixture.path().join("observation.txt"), format!("posix={posix}; swapped={swapped}; result={result:?}")).unwrap();
            panic!("delete removed the concurrent file; retained fixture: {}", fixture.keep().display());
        }
        result.unwrap();
        assert!(!swapped, "the original must be held until deletion finishes");
        assert!(!target.exists());
        assert!(!original.exists());
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
        fs::rename(&foreign, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"keep me");
    }

    #[cfg(windows)]
    #[test]
    fn file_delete_preserves_a_replacement_after_access_check() {
        file_delete_name_race(false);
    }

    #[cfg(windows)]
    #[test]
    fn file_delete_preserves_a_posix_replacement_after_access_check() {
        file_delete_name_race(true);
    }

    #[test]
    fn remove_entry_sync_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("to_delete.txt");
        fs::write(&file, "bye").unwrap();
        assert!(file.exists());

        remove_entry_sync(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn remove_entry_sync_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("a.txt"), "a").unwrap();

        remove_entry_sync(&sub).unwrap();
        assert!(!sub.exists());
    }

    #[test]
    fn remove_entry_sync_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("ghost.txt");
        let result = remove_entry_sync(&fake);
        assert!(result.is_err());
    }

    // -- rename_entry_sync tests (task 7.5) --

    #[test]
    fn rename_entry_sync_file() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.txt");
        fs::write(&old, "data").unwrap();

        let new_path = rename_entry_sync(&old, "new.txt").unwrap();
        assert!(!old.exists());
        assert!(new_path.exists());
        assert_eq!(fs::read_to_string(&new_path).unwrap(), "data");
    }

    #[test]
    fn rename_entry_sync_directory() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old_dir");
        fs::create_dir(&old).unwrap();

        let new_path = rename_entry_sync(&old, "new_dir").unwrap();
        assert!(!old.exists());
        assert!(new_path.is_dir());
    }

    #[test]
    fn rename_entry_sync_target_exists() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.txt");
        let existing = dir.path().join("existing.txt");
        fs::write(&old, "old").unwrap();
        fs::write(&existing, "existing").unwrap();

        let result = rename_entry_sync(&old, "existing.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    // -- copy_single_file_sync tests (task 7.5) --

    #[test]
    fn copy_single_file_sync_basic() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dest = dir.path().join("dest.txt");
        fs::write(&src, "content").unwrap();

        copy_single_file_sync(&src, &dest, dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "content");
    }

    #[test]
    fn copy_single_file_sync_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dest = dir.path().join("nested/deep/dest.txt");
        fs::write(&src, "nested").unwrap();

        copy_single_file_sync(&src, &dest, dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "nested");
    }

    #[test]
    fn copy_single_file_sync_source_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("missing.txt");
        let dest = dir.path().join("dest.txt");

        let result = copy_single_file_sync(&src, &dest, dir.path());
        assert!(result.is_err());
    }

    // -- get_image_base64_sync tests (task 7.6) --

    #[test]
    fn get_image_base64_sync_png() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.png");
        let bytes = vec![0x89, 0x50, 0x4E, 0x47]; // PNG magic bytes
        fs::write(&file, &bytes).unwrap();

        let result = get_image_base64_sync(&file).unwrap();
        assert!(result.starts_with("data:image/png;base64,"));

        // Verify the base64 part decodes back to original bytes
        let encoded_part = result.strip_prefix("data:image/png;base64,").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded_part).unwrap();
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn get_image_base64_sync_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("photo.jpg");
        let bytes = vec![0xFF, 0xD8, 0xFF, 0xE0]; // JPEG magic bytes
        fs::write(&file, &bytes).unwrap();

        let result = get_image_base64_sync(&file).unwrap();
        assert!(result.starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn get_image_base64_sync_svg() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("icon.svg");
        fs::write(&file, "<svg></svg>").unwrap();

        let result = get_image_base64_sync(&file).unwrap();
        assert!(result.starts_with("data:image/svg+xml;base64,"));
    }

    #[test]
    fn get_image_base64_sync_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("missing.png");

        let result = get_image_base64_sync(&fake);
        assert!(result.is_err());
    }

    #[test]
    fn get_image_base64_sync_unknown_extension() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.xyz999");
        fs::write(&file, b"some bytes").unwrap();

        let result = get_image_base64_sync(&file).unwrap();
        // Falls back to application/octet-stream
        assert!(result.starts_with("data:application/octet-stream;base64,"));
    }

    // -- placeholder_svg_data_url tests --

    #[test]
    fn placeholder_svg_data_url_format() {
        let url = placeholder_svg_data_url();
        assert!(url.starts_with("data:image/svg+xml;base64,"));

        // Verify it decodes to valid SVG content
        let encoded_part = url.strip_prefix("data:image/svg+xml;base64,").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded_part).unwrap();
        let svg = String::from_utf8(decoded).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }

    // -- validate_remote_image_url tests --

    #[test]
    fn validate_remote_image_url_https_allowed_host() {
        let result = validate_remote_image_url("https://raw.githubusercontent.com/owner/repo/main/image.png");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_remote_image_url_http_allowed_host() {
        let result = validate_remote_image_url("http://github.com/image.png");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_remote_image_url_disallowed_host() {
        let result = validate_remote_image_url("https://evil.com/image.png");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in the allowed"));
    }

    #[test]
    fn validate_remote_image_url_ftp_protocol() {
        let result = validate_remote_image_url("ftp://github.com/image.png");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported protocol"));
    }

    #[test]
    fn validate_remote_image_url_invalid_url() {
        let result = validate_remote_image_url("not-a-url");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid URL"));
    }

    #[test]
    fn validate_remote_image_url_file_protocol() {
        let result = validate_remote_image_url("file:///etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported protocol"));
    }

    // -- is_allowed_image_host tests --

    #[test]
    fn is_allowed_image_host_exact_match() {
        let url = reqwest::Url::parse("https://github.com/img.png").unwrap();
        assert!(is_allowed_image_host(&url));
    }

    #[test]
    fn is_allowed_image_host_subdomain_not_matched() {
        // "sub.github.com" should NOT match "github.com"
        let url = reqwest::Url::parse("https://sub.github.com/img.png").unwrap();
        assert!(!is_allowed_image_host(&url));
    }

    #[test]
    fn is_allowed_image_host_all_listed_hosts() {
        for host in ALLOWED_IMAGE_HOSTS {
            let url_str = format!("https://{host}/test.png");
            let url = reqwest::Url::parse(&url_str).unwrap();
            assert!(is_allowed_image_host(&url), "host '{host}' should be allowed");
        }
    }

    // -- create_zip_sync tests --

    #[test]
    fn create_zip_sync_text_entries() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("out.zip");
        let entries = vec![
            ZipEntry::Text {
                name: "hello.txt".into(),
                content: "Hello world".into(),
            },
            ZipEntry::Text {
                name: "sub/nested.txt".into(),
                content: "Nested content".into(),
            },
        ];
        let cancelled = AtomicBool::new(false);

        let result = create_zip_sync(&zip_path, &entries, &cancelled);
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert!(zip_path.exists());

        // Verify ZIP contents
        let file = fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.len(), 2);

        {
            let mut f0 = archive.by_name("hello.txt").unwrap();
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut f0, &mut buf).unwrap();
            assert_eq!(buf, "Hello world");
        }
        {
            let mut f1 = archive.by_name("sub/nested.txt").unwrap();
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut f1, &mut buf).unwrap();
            assert_eq!(buf, "Nested content");
        }
    }

    #[test]
    fn create_zip_sync_disk_entries() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("source.dat");
        fs::write(&src_path, b"binary data here").unwrap();

        let zip_path = dir.path().join("out.zip");
        let entries = vec![ZipEntry::Disk {
            name: "packed.dat".into(),
            file_path: src_path.to_string_lossy().into_owned(),
        }];
        let cancelled = AtomicBool::new(false);

        let result = create_zip_sync(&zip_path, &entries, &cancelled);
        assert!(result.unwrap());

        let file = fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.len(), 1);

        let mut f = archive.by_name("packed.dat").unwrap();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
        assert_eq!(buf, b"binary data here");
    }

    #[test]
    fn create_zip_sync_mixed_entries() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("disk.txt");
        fs::write(&src, "from disk").unwrap();

        let zip_path = dir.path().join("mixed.zip");
        let entries = vec![
            ZipEntry::Text {
                name: "mem.txt".into(),
                content: "from memory".into(),
            },
            ZipEntry::Disk {
                name: "disk.txt".into(),
                file_path: src.to_string_lossy().into_owned(),
            },
        ];
        let cancelled = AtomicBool::new(false);

        assert!(create_zip_sync(&zip_path, &entries, &cancelled).unwrap());

        let file = fs::File::open(&zip_path).unwrap();
        let archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.len(), 2);
    }

    #[test]
    fn create_zip_sync_empty_entries() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("empty.zip");
        let cancelled = AtomicBool::new(false);

        assert!(create_zip_sync(&zip_path, &[], &cancelled).unwrap());
        assert!(zip_path.exists());

        let file = fs::File::open(&zip_path).unwrap();
        let archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.len(), 0);
    }

    #[test]
    fn create_zip_sync_cancellation_before_start() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("cancelled.zip");
        let entries = vec![ZipEntry::Text {
            name: "a.txt".into(),
            content: "data".into(),
        }];
        let cancelled = AtomicBool::new(true);

        let result = create_zip_sync(&zip_path, &entries, &cancelled);
        assert!(!result.unwrap());
        assert!(!zip_path.exists());
        fs::write(&zip_path, "existing archive").unwrap();
        assert!(!create_zip_sync(&zip_path, &entries, &cancelled).unwrap());
        assert_eq!(fs::read(&zip_path).unwrap(), b"existing archive");
    }

    #[test]
    fn create_zip_sync_disk_entry_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("fail.zip");
        let entries = vec![ZipEntry::Disk {
            name: "missing.txt".into(),
            file_path: "/nonexistent/file.txt".into(),
        }];
        let cancelled = AtomicBool::new(false);

        let result = create_zip_sync(&zip_path, &entries, &cancelled);
        assert!(result.is_err());
    }

    #[test]
    fn create_zip_sync_error_cleans_up_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("good.txt");
        fs::write(&src, "data").unwrap();
        let zip_path = dir.path().join("partial.zip");

        // First entry succeeds, second fails → partial ZIP should be removed
        let entries = vec![
            ZipEntry::Disk {
                name: "good.txt".into(),
                file_path: src.to_string_lossy().into_owned(),
            },
            ZipEntry::Disk {
                name: "bad.txt".into(),
                file_path: "/nonexistent/missing.txt".into(),
            },
        ];
        let cancelled = AtomicBool::new(false);

        let result = create_zip_sync(&zip_path, &entries, &cancelled);
        assert!(result.is_err());
        assert!(!zip_path.exists(), "partial ZIP should be cleaned up on error");
    }

    #[test]
    fn create_zip_sync_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("deep/nested/out.zip");
        let entries = vec![ZipEntry::Text {
            name: "a.txt".into(),
            content: "data".into(),
        }];
        let cancelled = AtomicBool::new(false);

        assert!(create_zip_sync(&zip_path, &entries, &cancelled).unwrap());
        assert!(zip_path.exists());
    }

    // ---- create_upload_file -------------------------------------------------

    struct NullBroadcaster;
    impl nomifun_realtime::UserEventSink for NullBroadcaster {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    fn make_service() -> crate::service::FileService {
        crate::service::FileService::new(Arc::new(NullBroadcaster), vec![])
    }

    #[tokio::test]
    async fn create_upload_file_writes_bytes_and_returns_path() {
        use crate::traits::IFileService;
        let svc = make_service();
        let unique = format!(
            "upload_test_{}.bin",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path_str = svc.create_upload_file(&unique, b"hello bytes", None).await.unwrap();
        let path = std::path::Path::new(&path_str);
        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap().to_string_lossy(), unique);
        let contents = std::fs::read(path).unwrap();
        assert_eq!(contents, b"hello bytes");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn create_upload_file_routes_to_conversation_subdir() {
        use crate::traits::IFileService;
        let svc = make_service();
        let conv = format!(
            "conv-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let unique = format!(
            "img-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path_str = svc
            .create_upload_file(&unique, b"\x89PNG\r\n", Some(&conv))
            .await
            .unwrap();
        let path = std::path::Path::new(&path_str);
        let parent = path.parent().unwrap();
        assert_eq!(parent.file_name().unwrap().to_string_lossy(), conv);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(parent);
    }

    #[tokio::test]
    async fn create_upload_file_rejects_path_separators() {
        use crate::traits::IFileService;
        let svc = make_service();
        let result = svc.create_upload_file("nested/file.png", b"x", None).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
        let result = svc.create_upload_file("nested\\file.png", b"x", None).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn create_upload_file_rejects_traversal() {
        use crate::traits::IFileService;
        let svc = make_service();
        let result = svc.create_upload_file("..", b"x", None).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn create_upload_file_rejects_empty_name() {
        use crate::traits::IFileService;
        let svc = make_service();
        let result = svc.create_upload_file("", b"x", None).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn create_upload_file_rejects_invalid_conversation_id() {
        use crate::traits::IFileService;
        let svc = make_service();
        let result = svc.create_upload_file("good.png", b"x", Some("../escape")).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
        let result = svc.create_upload_file("good.png", b"x", Some("nested/id")).await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    // ---- name collision behaviour -----------------------------------------

    /// Generate a unique conversation id so each test gets a fresh directory.
    fn unique_conv_id(tag: &str) -> String {
        format!(
            "conv-collide-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn split_base_ext_matches_finder_conventions() {
        assert_eq!(split_base_ext("image.png"), ("image", ".png"));
        assert_eq!(split_base_ext("foo.tar.gz"), ("foo.tar", ".gz"));
        assert_eq!(split_base_ext("README"), ("README", ""));
        assert_eq!(split_base_ext(".env"), (".env", ""));
        assert_eq!(split_base_ext("a.b"), ("a", ".b"));
    }

    #[tokio::test]
    async fn create_upload_file_first_upload_uses_original_name() {
        use crate::traits::IFileService;
        let svc = make_service();
        let conv = unique_conv_id("first");
        let path_str = svc
            .create_upload_file("image.png", b"first", Some(&conv))
            .await
            .unwrap();
        let path = std::path::Path::new(&path_str);
        assert_eq!(path.file_name().unwrap().to_string_lossy(), "image.png");
        assert_eq!(std::fs::read(path).unwrap(), b"first");

        let parent = path.parent().unwrap().to_path_buf();
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[tokio::test]
    async fn create_upload_file_appends_numeric_suffix_on_conflict() {
        use crate::traits::IFileService;
        let svc = make_service();
        let conv = unique_conv_id("suffix");

        let first = svc.create_upload_file("image.png", b"one", Some(&conv)).await.unwrap();
        let second = svc.create_upload_file("image.png", b"two", Some(&conv)).await.unwrap();
        let third = svc
            .create_upload_file("image.png", b"three", Some(&conv))
            .await
            .unwrap();

        let first_path = std::path::Path::new(&first);
        let second_path = std::path::Path::new(&second);
        let third_path = std::path::Path::new(&third);

        assert_eq!(first_path.file_name().unwrap().to_string_lossy(), "image.png");
        assert_eq!(second_path.file_name().unwrap().to_string_lossy(), "image(2).png");
        assert_eq!(third_path.file_name().unwrap().to_string_lossy(), "image(3).png");

        // Originals stay intact — verifies no overwrite happened.
        assert_eq!(std::fs::read(first_path).unwrap(), b"one");
        assert_eq!(std::fs::read(second_path).unwrap(), b"two");
        assert_eq!(std::fs::read(third_path).unwrap(), b"three");

        let parent = first_path.parent().unwrap().to_path_buf();
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[tokio::test]
    async fn create_upload_file_handles_extensionless_collision() {
        use crate::traits::IFileService;
        let svc = make_service();
        let conv = unique_conv_id("noext");

        let first = svc.create_upload_file("README", b"a", Some(&conv)).await.unwrap();
        let second = svc.create_upload_file("README", b"b", Some(&conv)).await.unwrap();

        let first_path = std::path::Path::new(&first);
        let second_path = std::path::Path::new(&second);

        assert_eq!(first_path.file_name().unwrap().to_string_lossy(), "README");
        assert_eq!(second_path.file_name().unwrap().to_string_lossy(), "README(2)");

        let parent = first_path.parent().unwrap().to_path_buf();
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[tokio::test]
    async fn create_upload_file_handles_multi_dot_extension_collision() {
        use crate::traits::IFileService;
        let svc = make_service();
        let conv = unique_conv_id("multidot");

        let first = svc.create_upload_file("foo.tar.gz", b"a", Some(&conv)).await.unwrap();
        let second = svc.create_upload_file("foo.tar.gz", b"b", Some(&conv)).await.unwrap();

        let first_path = std::path::Path::new(&first);
        let second_path = std::path::Path::new(&second);

        assert_eq!(first_path.file_name().unwrap().to_string_lossy(), "foo.tar.gz");
        assert_eq!(second_path.file_name().unwrap().to_string_lossy(), "foo.tar(2).gz");

        let parent = first_path.parent().unwrap().to_path_buf();
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[tokio::test]
    async fn create_upload_file_handles_hidden_file_collision() {
        use crate::traits::IFileService;
        let svc = make_service();
        let conv = unique_conv_id("hidden");

        let first = svc.create_upload_file(".env", b"a", Some(&conv)).await.unwrap();
        let second = svc.create_upload_file(".env", b"b", Some(&conv)).await.unwrap();

        let first_path = std::path::Path::new(&first);
        let second_path = std::path::Path::new(&second);

        assert_eq!(first_path.file_name().unwrap().to_string_lossy(), ".env");
        assert_eq!(second_path.file_name().unwrap().to_string_lossy(), ".env(2)");

        let parent = first_path.parent().unwrap().to_path_buf();
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[tokio::test]
    async fn create_upload_file_preserves_all_bytes_across_collisions() {
        use crate::traits::IFileService;
        let svc = make_service();
        let conv = unique_conv_id("bytes");

        let a = svc.create_upload_file("image.png", b"AAA", Some(&conv)).await.unwrap();
        let b = svc.create_upload_file("image.png", b"BBB", Some(&conv)).await.unwrap();
        let c = svc.create_upload_file("image.png", b"CCC", Some(&conv)).await.unwrap();

        // All three files exist with distinct content — no overwrite.
        assert_eq!(std::fs::read(&a).unwrap(), b"AAA");
        assert_eq!(std::fs::read(&b).unwrap(), b"BBB");
        assert_eq!(std::fs::read(&c).unwrap(), b"CCC");

        // Sanity: three distinct paths.
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);

        let parent = std::path::Path::new(&a).parent().unwrap().to_path_buf();
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[tokio::test]
    async fn agent_write_creates_nested_project_without_a_shell_mkdir() {
        let root = tempfile::tempdir().unwrap();
        let svc = make_service();
        let scope = patch_scope(root.path());
        for (path, content) in [
            ("gomoku/index.html", "<!DOCTYPE html>\n<title>五子棋</title>\n"),
            ("gomoku/assets/game.js", "const size = 15;\n"),
            ("gomoku/index.html", "<!DOCTYPE html>\n<title>Gomoku</title>\n"),
        ] {
            svc.write_file_for_agent_session(&scope, path, content.as_bytes()).await.unwrap();
            assert_eq!(fs::read_to_string(root.path().join(path)).unwrap(), content);
            assert_eq!(svc.read_file_for_agent_session(&scope, path).await.unwrap().as_deref(), Some(content));
        }
    }

    #[tokio::test]
    async fn agent_search_accepts_root_spellings_without_broadening_mutation_paths() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("game.html"), "五子棋 needle").unwrap();
        let svc = make_service();
        let scope = patch_scope(root.path());
        for path in [None, Some(""), Some(".")] {
            let result = svc.search_text_for_agent_session(&scope, crate::AgentTextSearchRequest {
                query: "五子棋".into(), path: path.map(str::to_owned), limit: Some(50),
            }).await.unwrap();
            assert_eq!(result.matches.len(), 1);
            assert_eq!(result.matches[0].path, "game.html");
        }
        for path in ["..", "../outside", "./game.html", ".nomifun"] {
            assert!(svc.search_text_for_agent_session(&scope, crate::AgentTextSearchRequest {
                query: "needle".into(), path: Some(path.into()), limit: None,
            }).await.is_err());
        }
        assert!(scope.resolve_relative_path(".").is_err());
    }

    fn nested_creation_patch(path: &str) -> AgentSessionFilePatch {
        AgentSessionFilePatch {
            path: path.into(),
            expected_source: crate::AgentSessionPatchSource::Absent,
            hunks: vec![replace_hunk(0, 0, 1, 1, vec![
                AgentSessionPatchLine::Add { text: "<!DOCTYPE html>".into() },
            ])],
        }
    }

    #[tokio::test]
    async fn agent_patch_creates_nested_parents_only_after_whole_batch_validation() {
        let root = tempfile::tempdir().unwrap();
        let svc = make_service();
        let scope = patch_scope(root.path());
        let first = nested_creation_patch("gomoku/index.html");
        let mut bad = nested_creation_patch("other/nested/index.html");
        bad.hunks[0].new_lines = 0; // The rejected hunk in the original dev trace.
        let failure = svc.apply_patch_with_observation_for_agent_session(&scope,
            AgentSessionPatchRequest { files: vec![first.clone(), bad] }).await.unwrap_err();
        assert!(failure.observation.published.is_empty());
        assert!(!root.path().join("gomoku").exists());
        assert!(!root.path().join("other").exists());

        let result = svc.apply_patch_for_agent_session(&scope,
            AgentSessionPatchRequest { files: vec![first, nested_creation_patch("other/nested/index.html")] })
            .await.unwrap();
        assert_eq!(result.file_count, 2);
        for path in ["gomoku/index.html", "other/nested/index.html"] {
            assert_eq!(fs::read_to_string(root.path().join(path)).unwrap(), "<!DOCTYPE html>");
        }
    }

    #[tokio::test]
    async fn agent_nested_creation_preserves_invalid_parents_and_workspace_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let svc = make_service();
        let scope = patch_scope(root.path());
        fs::write(root.path().join("existing"), "keep").unwrap();
        for path in ["existing/nested/index.html", ".nomifun/new/index.html", "../escape/index.html"] {
            assert!(svc.write_file_for_agent_session(&scope, path, b"wrong").await.is_err());
            assert!(svc.apply_patch_for_agent_session(&scope,
                AgentSessionPatchRequest { files: vec![nested_creation_patch(path)] }).await.is_err());
        }
        assert_eq!(fs::read_to_string(root.path().join("existing")).unwrap(), "keep");
        assert!(!root.path().join(".nomifun").exists());
    }

    #[tokio::test]
    async fn agent_patch_rejects_file_ancestor_targets_before_any_publication() {
        for paths in [["new", "new/child.txt"], ["new/child.txt", "new"]] {
            let root = tempfile::tempdir().unwrap();
            let svc = make_service();
            let scope = patch_scope(root.path());
            let failure = svc.apply_patch_with_observation_for_agent_session(&scope,
                AgentSessionPatchRequest {
                    files: paths.iter().map(|path| nested_creation_patch(path)).collect(),
                }).await.unwrap_err();
            assert!(failure.observation.published.is_empty(),
                "conflicting file/ancestor targets must fail in preparation: {:?}", failure.observation);
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0,
                "invalid batch must not create files or parent directories: {paths:?}");
        }
    }

    #[cfg(any(unix, windows))]
    #[tokio::test]
    async fn agent_nested_creation_rejects_outside_and_owner_aliases() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".nomifun")).unwrap();
        fs::create_dir(root.path().join("inside")).unwrap();
        let svc = make_service();
        let scope = patch_scope(root.path());
        for (alias, destination, allowed) in [
            ("outside-alias", outside.path().to_path_buf(), false),
            ("owner-alias", root.path().join(".nomifun"), false),
            ("inside-alias", root.path().join("inside"), true),
        ] {
            #[cfg(windows)]
            junction::create(&destination, root.path().join(alias)).unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(&destination, root.path().join(alias)).unwrap();
            let written = svc.write_file_for_agent_session(&scope, &format!("{alias}/new/write.html"), b"ok").await;
            let patched = svc.apply_patch_for_agent_session(&scope, AgentSessionPatchRequest {
                files: vec![nested_creation_patch(&format!("{alias}/new/patch.html"))],
            }).await;
            assert_eq!(written.is_ok(), allowed, "{alias}: {written:?}");
            assert_eq!(patched.is_ok(), allowed, "{alias}: {patched:?}");
            assert_eq!(destination.join("new").exists(), allowed);
        }
    }

    fn patch_scope(root: &std::path::Path) -> AgentSessionWorkspaceBinding {
        crate::resource::workspace_binding(
            nomifun_common::generate_id(),
            "patch-binding",
            "patch-workspace",
            "owner-1",
            [
                crate::resource::READ_OPERATION,
                crate::resource::WRITE_OPERATION,
            ],
            root,
        )
        .unwrap()
    }

    async fn assert_owner_alias_is_not_agent_accessible(root: &std::path::Path) {
        let svc = make_service();
        let scope = patch_scope(root);
        assert!(
            svc.read_file_for_agent_session(&scope, "alias/artifacts/receipt")
                .await
                .is_err()
        );
        assert!(
            svc.write_file_for_agent_session(
                &scope,
                "alias/artifacts/replacement",
                b"attacker",
            )
            .await
            .is_err()
        );
        assert!(!root.join(".nomifun/artifacts/replacement").exists());
        assert_eq!(
            std::fs::read_to_string(root.join(".nomifun/artifacts/receipt")).unwrap(),
            "owned"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_file_owner_rejects_symlink_alias_into_nomifun() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".nomifun/artifacts")).unwrap();
        std::fs::write(root.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        std::os::unix::fs::symlink(".nomifun", root.path().join("alias")).unwrap();
        assert_owner_alias_is_not_agent_accessible(root.path()).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn agent_file_owner_rejects_junction_alias_into_nomifun() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".nomifun/artifacts")).unwrap();
        std::fs::write(root.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        junction::create(root.path().join(".nomifun"), root.path().join("alias")).unwrap();
        assert_owner_alias_is_not_agent_accessible(root.path()).await;
    }

    fn replace_hunk(
        old_start: usize,
        old_lines: usize,
        new_start: usize,
        new_lines: usize,
        lines: Vec<AgentSessionPatchLine>,
    ) -> AgentSessionPatchHunk {
        AgentSessionPatchHunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
            lines,
        }
    }

    #[tokio::test]
    async fn apply_agent_patch_updates_multiple_files_and_returns_bounded_result() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();
        fs::write(dir.path().join("b.txt"), "bravo\n").unwrap();
        let svc = make_service();
        let scope = patch_scope(dir.path());

        let result = svc
            .apply_patch_for_agent_session(
                &scope,
                AgentSessionPatchRequest {
                    files: vec![
                        AgentSessionFilePatch {
                            path: "a.txt".into(),
                            expected_source: Default::default(),
                            hunks: vec![replace_hunk(
                                1,
                                1,
                                1,
                                1,
                                vec![
                                    AgentSessionPatchLine::Remove {
                                        text: "alpha".into(),
                                    },
                                    AgentSessionPatchLine::Add {
                                        text: "ALPHA".into(),
                                    },
                                ],
                            )],
                        },
                        AgentSessionFilePatch {
                            path: "b.txt".into(),
                            expected_source: Default::default(),
                            hunks: vec![replace_hunk(
                                1,
                                1,
                                1,
                                1,
                                vec![
                                    AgentSessionPatchLine::Remove {
                                        text: "bravo".into(),
                                    },
                                    AgentSessionPatchLine::Add {
                                        text: "BRAVO".into(),
                                    },
                                ],
                            )],
                        },
                    ],
                },
            )
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "ALPHA\n");
        assert_eq!(fs::read_to_string(dir.path().join("b.txt")).unwrap(), "BRAVO\n");
        assert_eq!(result.file_count, 2);
        assert_eq!(result.files[0].path, "a.txt");
        assert_eq!(result.files[0].bytes_before, 6);
        assert_eq!(result.files[0].bytes_after, 6);
        assert_eq!(result.total_bytes_before, 12);
        assert_eq!(result.total_bytes_after, 12);
    }

    #[tokio::test]
    async fn apply_agent_patch_supports_ordered_multiple_hunks_and_insertions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ordered.txt");
        fs::write(&path, "a\nb\nc\nd\n").unwrap();
        let svc = make_service();
        let scope = patch_scope(dir.path());

        svc.apply_patch_for_agent_session(
            &scope,
            AgentSessionPatchRequest {
                files: vec![AgentSessionFilePatch {
                    path: "ordered.txt".into(),
                    expected_source: Default::default(),
                    hunks: vec![
                        replace_hunk(
                            1,
                            1,
                            1,
                            1,
                            vec![
                                AgentSessionPatchLine::Remove { text: "a".into() },
                                AgentSessionPatchLine::Add { text: "A".into() },
                            ],
                        ),
                        replace_hunk(
                            3,
                            1,
                            3,
                            1,
                            vec![
                                AgentSessionPatchLine::Remove { text: "c".into() },
                                AgentSessionPatchLine::Add { text: "C".into() },
                            ],
                        ),
                        replace_hunk(
                            3,
                            0,
                            4,
                            1,
                            vec![AgentSessionPatchLine::Add { text: "inserted".into() }],
                        ),
                    ],
                }],
            },
        )
        .await
        .unwrap();

        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "A\nb\nC\ninserted\nd\n"
        );
    }

    #[tokio::test]
    async fn invalid_agent_patch_hunk_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        fs::write(&first, "first\n").unwrap();
        fs::write(&second, "second\n").unwrap();
        let svc = make_service();
        let scope = patch_scope(dir.path());

        let error = svc
            .apply_patch_for_agent_session(
                &scope,
                AgentSessionPatchRequest {
                    files: vec![
                        AgentSessionFilePatch {
                            path: "first.txt".into(),
                            expected_source: Default::default(),
                            hunks: vec![replace_hunk(
                                1,
                                1,
                                1,
                                1,
                                vec![
                                    AgentSessionPatchLine::Remove {
                                        text: "first".into(),
                                    },
                                    AgentSessionPatchLine::Add {
                                        text: "changed".into(),
                                    },
                                ],
                            )],
                        },
                        AgentSessionFilePatch {
                            path: "second.txt".into(),
                            expected_source: Default::default(),
                            hunks: vec![replace_hunk(
                                1,
                                1,
                                1,
                                1,
                                vec![
                                    AgentSessionPatchLine::Remove {
                                        text: "not-the-source".into(),
                                    },
                                    AgentSessionPatchLine::Add {
                                        text: "never-written".into(),
                                    },
                                ],
                            )],
                        },
                    ],
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::BadRequest(_)));
        assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "second\n");
    }

    struct MutatingPatchEventSink {
        target: std::path::PathBuf,
        fired: std::sync::atomic::AtomicBool,
    }

    impl nomifun_realtime::UserEventSink for MutatingPatchEventSink {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
            if !self
                .fired
                .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                std::fs::write(&self.target, "external change\n")
                    .expect("mutating test sink writes its target");
            }
        }
    }

    #[tokio::test]
    async fn agent_patch_detects_external_change_and_rolls_back_prior_writes() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        fs::write(&first, "first\n").unwrap();
        fs::write(&second, "second\n").unwrap();
        let sink = Arc::new(MutatingPatchEventSink {
            target: second.clone(),
            fired: std::sync::atomic::AtomicBool::new(false),
        });
        let svc = crate::service::FileService::new(sink, vec![]);
        let scope = patch_scope(dir.path());
        let replace = |path: &str, old: &str, new: &str| AgentSessionFilePatch {
            path: path.to_owned(),
            expected_source: Default::default(),
            hunks: vec![replace_hunk(
                1,
                1,
                1,
                1,
                vec![
                    AgentSessionPatchLine::Remove {
                        text: old.to_owned(),
                    },
                    AgentSessionPatchLine::Add {
                        text: new.to_owned(),
                    },
                ],
            )],
        };

        let error = svc
            .apply_patch_for_agent_session(
                &scope,
                AgentSessionPatchRequest {
                    files: vec![
                        replace("first.txt", "first", "FIRST"),
                        replace("second.txt", "second", "SECOND"),
                    ],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
        assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
        // The failed target changed externally before its write; rollback must
        // not clobber that newer user change.
        assert_eq!(fs::read_to_string(second).unwrap(), "external change\n");
    }

    #[tokio::test]
    async fn agent_patch_rejects_traversal_without_touching_workspace_or_outside() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.txt");
        fs::write(&outside_file, "secret\n").unwrap();
        let svc = make_service();
        let scope = patch_scope(workspace.path());

        let error = svc
            .apply_patch_for_agent_session(
                &scope,
                AgentSessionPatchRequest {
                    files: vec![AgentSessionFilePatch {
                        path: format!("../{}", outside_file.file_name().unwrap().to_string_lossy()),
                        expected_source: Default::default(),
                        hunks: vec![replace_hunk(
                            1,
                            1,
                            1,
                            1,
                            vec![
                                AgentSessionPatchLine::Remove {
                                    text: "secret".into(),
                                },
                                AgentSessionPatchLine::Add {
                                    text: "escaped".into(),
                                },
                            ],
                        )],
                    }],
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::BadRequest(_)));
        assert_eq!(fs::read_to_string(outside_file).unwrap(), "secret\n");
        assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agent_patch_rejects_a_final_symlink_target() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.txt");
        fs::write(&outside_file, "secret\n").unwrap();
        std::os::unix::fs::symlink(&outside_file, workspace.path().join("link.txt")).unwrap();
        let svc = make_service();
        let scope = patch_scope(workspace.path());

        let error = svc
            .apply_patch_for_agent_session(
                &scope,
                AgentSessionPatchRequest {
                    files: vec![AgentSessionFilePatch {
                        path: "link.txt".into(),
                        expected_source: Default::default(),
                        hunks: vec![replace_hunk(
                            1,
                            1,
                            1,
                            1,
                            vec![
                                AgentSessionPatchLine::Remove { text: "secret".into() },
                                AgentSessionPatchLine::Add { text: "escaped".into() },
                            ],
                        )],
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Forbidden(_)));
        assert_eq!(fs::read_to_string(outside_file).unwrap(), "secret\n");
    }

    #[tokio::test]
    async fn agent_patch_enforces_file_count_and_size_limits() {
        let dir = tempfile::tempdir().unwrap();
        let svc = make_service();
        let scope = patch_scope(dir.path());
        let tiny_hunk = || {
            replace_hunk(
                0,
                0,
                1,
                1,
                vec![AgentSessionPatchLine::Add {
                    text: "x".into(),
                }],
            )
        };

        let too_many_files = AgentSessionPatchRequest {
            files: (0..=MAX_AGENT_PATCH_FILES)
                .map(|index| AgentSessionFilePatch {
                    path: format!("file-{index}.txt"),
                    expected_source: Default::default(),
                    hunks: vec![tiny_hunk()],
                })
                .collect(),
        };
        let error = svc
            .apply_patch_for_agent_session(&scope, too_many_files)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("maximum is"));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);

        let oversized = dir.path().join("oversized.txt");
        fs::write(&oversized, vec![b'x'; MAX_AGENT_PATCH_FILE_BYTES + 1]).unwrap();
        let error = svc
            .apply_patch_for_agent_session(
                &scope,
                AgentSessionPatchRequest {
                    files: vec![AgentSessionFilePatch {
                        path: "oversized.txt".into(),
                        expected_source: Default::default(),
                        hunks: vec![replace_hunk(
                            1,
                            1,
                            1,
                            1,
                            vec![
                                AgentSessionPatchLine::Remove {
                                    text: "x".into(),
                                },
                                AgentSessionPatchLine::Add {
                                    text: "y".into(),
                                },
                            ],
                        )],
                    }],
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("per-file limit"));
        assert_eq!(fs::metadata(&oversized).unwrap().len(), (MAX_AGENT_PATCH_FILE_BYTES + 1) as u64);
    }

    #[test]
    fn agent_patch_request_rejects_unknown_fields() {
        let value = serde_json::json!({
            "files": [],
            "unexpected": true
        });
        assert!(serde_json::from_value::<AgentSessionPatchRequest>(value).is_err());
    }
}
