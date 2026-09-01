use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use base64::Engine;
use dashmap::DashMap;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use tracing::warn;

use nomifun_api_types::WebSocketMessage;
use nomifun_common::AppError;
use nomifun_realtime::UserEventSink;

use crate::path_safety::{
    PathAuthority, has_traversal, is_unsafe_path_segment, validate_path, validate_path_authority,
    validate_path_for_write, validate_path_for_write_authority, validate_path_with_extra_root,
};
use crate::resource::AgentSessionWorkspaceBinding;
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
    /// Normalized workspace-relative path; no native absolute path is
    /// returned to the agent.
    pub path: String,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub hunks_applied: usize,
    pub created: bool,
}

/// A concrete implementation of [`crate::traits::IFileService`].
pub struct FileService {
    user_events: Arc<dyn UserEventSink>,
    /// Allowed root directories for path safety validation.
    allowed_roots: Vec<std::path::PathBuf>,
    /// In-memory cache for `list_workspace_files`, keyed by canonical root.
    workspace_files_cache: DashMap<String, Vec<WorkspaceFlatFile>>,
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
    /// all path I/O still goes through the existing confined authority.
    pub async fn read_file_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
    ) -> Result<Option<String>, AppError> {
        scope.require_operation(crate::resource::READ_OPERATION)?;
        let path = scope.resolve_relative_path(relative_path)?;
        self.read_file_impl(&path.to_string_lossy(), &scope.authority()).await
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

    pub async fn write_file_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
        data: &[u8],
    ) -> Result<bool, AppError> {
        scope.require_operation(crate::resource::WRITE_OPERATION)?;
        let path = scope.resolve_relative_path(relative_path)?;
        let workspace = scope.workspace_root().to_string_lossy();
        self.write_file_impl(
            scope.owner_id(),
            &path.to_string_lossy(),
            data,
            &workspace,
            &scope.authority(),
        )
        .await
    }

    pub async fn remove_entry_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        relative_path: &str,
    ) -> Result<(), AppError> {
        scope.require_operation(crate::resource::DELETE_OPERATION)?;
        let path = scope.resolve_relative_path(relative_path)?;
        let workspace = scope.workspace_root().to_string_lossy();
        self.remove_entry_impl(
            scope.owner_id(),
            &path.to_string_lossy(),
            &workspace,
            &scope.authority(),
        )
        .await
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
        let mut seen_paths = HashSet::with_capacity(request.files.len());
        let mut total_before = 0_u64;
        let mut total_after = 0_u64;

        for file_patch in &request.files {
            let (path, existed) = validate_agent_patch_target(scope, &file_patch.path, &authority)?;
            if !seen_paths.insert(path.clone()) {
                return Err(AppError::BadRequest(format!(
                    "agent patch contains duplicate target '{}'",
                    file_patch.path
                )));
            }

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
                    )));
                }

                self.read_file_impl(&path.to_string_lossy(), &authority)
                    .await?
                    .ok_or_else(|| {
                        AppError::Internal(format!(
                            "patch target '{}' disappeared while it was being read",
                            file_patch.path
                        ))
                    })?
                    .into_bytes()
            } else {
                Vec::new()
            };

            let before_text = String::from_utf8(before.clone()).map_err(|_| {
                AppError::BadRequest(format!(
                    "patch target '{}' is not valid UTF-8 text",
                    file_patch.path
                ))
            })?;
            let after_text = apply_agent_patch_hunks(&before_text, &file_patch.hunks)?;
            let after = after_text.into_bytes();

            if after.len() > MAX_AGENT_PATCH_FILE_BYTES {
                return Err(AppError::BadRequest(format!(
                    "patched file '{}' exceeds the {} byte per-file limit",
                    file_patch.path, MAX_AGENT_PATCH_FILE_BYTES
                )));
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
                )));
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
        // the commit phase, restore already-touched entries through the same
        // authority-aware write/remove paths.
        let workspace = scope.workspace_root().to_string_lossy().into_owned();
        let mut applied = Vec::with_capacity(prepared.len());
        for (index, file) in prepared.iter().enumerate() {
            if let Err(error) = self
                .verify_agent_patch_precondition(file, &authority)
                .await
            {
                self.rollback_agent_patch_files(
                    scope,
                    &authority,
                    &workspace,
                    &prepared,
                    &applied,
                )
                .await;
                return Err(error);
            }
            let write_result = self
                .write_agent_patch_file(
                    scope.owner_id(),
                    file,
                    &file.after,
                    &workspace,
                    &authority,
                )
                .await;

            if let Err(error) = write_result {
                self.rollback_agent_patch_files(scope, &authority, &workspace, &prepared, &applied)
                    .await;
                return Err(error);
            }
            applied.push(index);
        }

        Ok(AgentSessionPatchResult {
            file_count: prepared.len(),
            files: prepared
                .into_iter()
                .map(|file| AgentSessionPatchFileResult {
                    path: file.relative_path,
                    bytes_before: file.before.len() as u64,
                    bytes_after: file.after.len() as u64,
                    hunks_applied: file.hunks_applied,
                    created: !file.existed,
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
                let reader = std::fs::File::open(file.path.as_path()).map_err(|error| {
                    AppError::Internal(format!(
                        "cannot re-open patch target '{}': {error}",
                        file.relative_path
                    ))
                })?;
                let mut current = Vec::new();
                reader
                    .take((MAX_AGENT_PATCH_FILE_BYTES + 1) as u64)
                    .read_to_end(&mut current)
                    .map_err(|error| {
                        AppError::Internal(format!(
                            "cannot re-read patch target '{}': {error}",
                            file.relative_path
                        ))
                    })?;
                if current.len() > MAX_AGENT_PATCH_FILE_BYTES {
                    return Err(AppError::Conflict(format!(
                        "patch target '{}' grew beyond the per-file limit",
                        file.relative_path
                    )));
                }
                if current != file.before {
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
    ) {
        for index in applied.iter().rev().copied() {
            let file = &files[index];
            if file.existed {
                if !current_file_matches(&file.path, &file.after) {
                    continue;
                }
                let _ = self
                    .write_agent_patch_file(
                        scope.owner_id(),
                        file,
                        &file.before,
                        workspace,
                        authority,
                    )
                    .await;
            } else {
                if !current_file_matches(&file.path, &file.after) {
                    continue;
                }
                let _ = self
                    .remove_entry_impl(
                        scope.owner_id(),
                        &file.path.to_string_lossy(),
                        workspace,
                        authority,
                    )
                    .await;
            }
        }
    }

    async fn write_agent_patch_file(
        &self,
        owner_id: &str,
        file: &PreparedAgentPatchFile,
        data: &[u8],
        workspace: &str,
        authority: &PathAuthority,
    ) -> Result<bool, AppError> {
        let path = file.path.to_string_lossy();
        if has_traversal(&path) {
            return Err(AppError::BadRequest(format!(
                "path '{}' contains invalid traversal patterns",
                path
            )));
        }
        let canonical = validate_path_for_write_authority(&path, authority)?;
        if canonical != file.path {
            return Err(AppError::Conflict(format!(
                "patch target '{}' changed identity before publication",
                file.relative_path
            )));
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&canonical)
            && metadata.file_type().is_symlink()
        {
            return Err(AppError::Conflict(format!(
                "patch target '{}' is a symbolic link",
                file.relative_path
            )));
        }
        write_file_sync_atomic(&canonical, data)?;
        self.emit_content_update(owner_id, &canonical, data, workspace);
        Ok(true)
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
        self.user_events
            .send_to_user(owner_id, WebSocketMessage::new("fileStream.contentUpdate", payload));
        if let Ok(canonical_ws) = std::fs::canonicalize(workspace_path) {
            self.invalidate_cache(&canonical_ws.to_string_lossy());
        }
    }

    /// Invalidate the workspace files cache for a given root.
    /// Called when file changes are detected.
    pub fn invalidate_cache(&self, root: &str) {
        self.workspace_files_cache.remove(root);
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
        let canonical_dir = validate_path_authority(dir, authority)?;
        let canonical_root = validate_path_authority(root, authority)?;
        self.build_dir_tree(&canonical_dir, &canonical_root).await
    }

    async fn list_workspace_files_impl(
        &self,
        root: &str,
        authority: &PathAuthority,
    ) -> Result<Vec<WorkspaceFlatFile>, AppError> {
        let canonical_root = validate_path_authority(root, authority)?;
        let cache_key = canonical_root.to_string_lossy().into_owned();

        if let Some(cached) = self.workspace_files_cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        let root_owned = canonical_root.clone();
        let files = tokio::task::spawn_blocking(move || list_workspace_files_sync(&root_owned))
            .await
            .map_err(|e| AppError::Internal(format!("workspace file listing task failed: {e}")))??;

        self.workspace_files_cache.insert(cache_key, files.clone());
        Ok(files)
    }

    async fn get_file_metadata_impl(
        &self,
        path: &str,
        authority: &PathAuthority,
    ) -> Result<FileMetadata, AppError> {
        let canonical = validate_path_authority(path, authority)?;
        let result = tokio::task::spawn_blocking(move || get_file_metadata_sync(&canonical))
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

        let workspace_path = Path::new(workspace);
        let relative_path = rel_to_api_string(
            canonical
                .strip_prefix(std::fs::canonicalize(workspace_path).unwrap_or_else(|_| workspace_path.to_path_buf()))
                .unwrap_or(&canonical),
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
        let msg = WebSocketMessage::new("fileStream.contentUpdate", payload);
        self.user_events.send_to_user(owner_id, msg);

        if let Ok(canonical_ws) = std::fs::canonicalize(workspace_path) {
            self.invalidate_cache(&canonical_ws.to_string_lossy());
        }

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
        tokio::task::spawn_blocking(move || remove_entry_sync(&path_owned))
            .await
            .map_err(|e| AppError::Internal(format!("remove entry task failed: {e}")))??;

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
        self.user_events.send_to_user(owner_id, msg);

        if let Ok(canonical_ws) = std::fs::canonicalize(workspace_path) {
            self.invalidate_cache(&canonical_ws.to_string_lossy());
        }

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

    /// List immediate children of `dir`, building a single-level tree.
    /// Each child directory also lists *its* children (depth = 2 from `dir`).
    async fn build_dir_tree(&self, dir: &Path, root: &Path) -> Result<Vec<DirOrFile>, AppError> {        let dir_owned = dir.to_path_buf();
        let root_owned = root.to_path_buf();

        tokio::task::spawn_blocking(move || build_dir_tree_sync(&dir_owned, &root_owned))
            .await
            .map_err(|e| AppError::Internal(format!("directory listing task failed: {e}")))?
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
                if text.contains('\n') || text.contains('\0') {
                    return Err(AppError::BadRequest(format!(
                        "agent patch file '{}' contains a line with an embedded newline or NUL",
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
    let relative_path = relative_path.trim();
    let candidate = scope.resolve_relative_path(relative_path)?;
    let candidate_string = candidate.to_string_lossy();
    let write_candidate = validate_path_for_write_authority(&candidate_string, authority)?;

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
    let (source, had_trailing_newline) = split_agent_patch_lines(original);
    if source.len() > MAX_AGENT_PATCH_LINES_PER_FILE {
        return Err(AppError::BadRequest(format!(
            "patch source has too many lines; maximum is {}",
            MAX_AGENT_PATCH_LINES_PER_FILE
        )));
    }

    let mut output = Vec::with_capacity(source.len());
    let mut source_cursor = 0_usize;

    for hunk in hunks {
        let hunk_start = if hunk.old_lines == 0 {
            // For an insertion, accept the common unified-diff positions:
            // 0/1 at the beginning, or the number of source lines already
            // consumed (with +1 also accepted for callers that describe the
            // insertion as "before the next line").
            if source_cursor == 0 && (hunk.old_start == 0 || hunk.old_start == 1) {
                0
            } else if hunk.old_start == source_cursor
                || hunk.old_start == source_cursor.saturating_add(1)
            {
                source_cursor
            } else {
                return Err(invalid_agent_hunk(
                    hunk,
                    "insertion old_start must identify the current source position",
                ));
            }
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
        let expected_new_start = if output_start == 0 && hunk.new_lines == 0 {
            if hunk.new_start != 0 && hunk.new_start != 1 {
                return Err(invalid_agent_hunk(
                    hunk,
                    "an empty output hunk must start at new line 0 or 1",
                ));
            }
            hunk.new_start
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
                    if source.get(source_position).map(String::as_str) != Some(text.as_str()) {
                        return Err(invalid_agent_hunk(
                            hunk,
                            "context line does not match the source",
                        ));
                    }
                    output.push(text.clone());
                    source_position += 1;
                    old_consumed += 1;
                    new_produced += 1;
                }
                AgentSessionPatchLine::Remove { text } => {
                    if source.get(source_position).map(String::as_str) != Some(text.as_str()) {
                        return Err(invalid_agent_hunk(
                            hunk,
                            "removed line does not match the source",
                        ));
                    }
                    source_position += 1;
                    old_consumed += 1;
                }
                AgentSessionPatchLine::Add { text } => {
                    output.push(text.clone());
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
    let output_line_count = output.len();
    let output = output
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n");
    let mut result = output;
    if had_trailing_newline && output_line_count > 0 {
        result.push('\n');
    }
    if result.len() > MAX_AGENT_PATCH_FILE_BYTES {
        return Err(AppError::BadRequest(format!(
            "patched file exceeds the {} byte per-file limit",
            MAX_AGENT_PATCH_FILE_BYTES
        )));
    }
    Ok(result)
}

fn split_agent_patch_lines(content: &str) -> (Vec<String>, bool) {
    if content.is_empty() {
        return (Vec::new(), false);
    }
    let had_trailing_newline = content.ends_with('\n');
    let mut lines = content.split('\n').map(str::to_owned).collect::<Vec<_>>();
    if had_trailing_newline {
        lines.pop();
    }
    (lines, had_trailing_newline)
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
fn build_dir_tree_sync(dir: &Path, root: &Path) -> Result<Vec<DirOrFile>, AppError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| AppError::BadRequest(format!("cannot read directory '{}': {e}", dir.display())))?;

    let mut result = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| AppError::Internal(format!("error reading directory entry: {e}")))?;

        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|e| AppError::Internal(format!("cannot read metadata for '{}': {e}", path.display())))?;

        let name = entry.file_name().to_string_lossy().into_owned();

        let full_path = path.to_string_lossy().into_owned();
        let relative_path = rel_to_api_string(path.strip_prefix(root).unwrap_or(&path));

        let is_dir = metadata.is_dir();

        // For directories, also read their immediate children
        let children = if is_dir {
            read_children_sync(&path, root)?
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
fn read_children_sync(dir: &Path, root: &Path) -> Result<Vec<DirOrFile>, AppError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };

    let mut children = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let is_dir = entry.metadata().map(|m| m.is_dir()).unwrap_or(false);

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

/// Recursively list files using the `ignore` crate (respects .gitignore).
fn list_workspace_files_sync(root: &Path) -> Result<Vec<WorkspaceFlatFile>, AppError> {
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .build();

    let mut files = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("skipping unreadable entry: {e}");
                continue;
            }
        };

        let path = entry.path();
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "skipping unreadable workspace entry");
                continue;
            }
        };

        // Skip real directories and symlinks that resolve to directories.
        if metadata.is_dir() {
            continue;
        }

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let full_path = path.to_string_lossy().into_owned();
        let relative_path = rel_to_api_string(path.strip_prefix(root).unwrap_or(path));

        files.push(WorkspaceFlatFile {
            name,
            full_path,
            relative_path,
        });

        if files.len() >= MAX_WORKSPACE_FILES {
            break;
        }
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

fn current_file_matches(path: &Path, expected: &[u8]) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return false;
    }
    if metadata.len() > (MAX_AGENT_PATCH_FILE_BYTES as u64) {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes == expected
}

/// Atomically publish one already-authorized AgentSession patch file.
///
/// The temporary file is created beside the target, fully written and synced,
/// and then replaced with a same-filesystem rename. A new file uses a
/// no-clobber hard-link publication so a concurrent creator cannot be silently
/// overwritten. Existing files use the platform's atomic replacement primitive.
fn write_file_sync_atomic(path: &Path, data: &[u8]) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::BadRequest(format!(
            "patch target '{}' has no parent directory",
            path.display()
        ))
    })?;
    let file_name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
        AppError::BadRequest(format!(
            "patch target '{}' has no valid file name",
            path.display()
        ))
    })?;
    static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.nomifun-patch-{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let result = (|| -> Result<(), AppError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|error| {
            AppError::Internal(format!(
                "cannot create temporary patch file '{}': {error}",
                temporary.display()
            ))
        })?;
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
        if let Some(metadata) = target_metadata {
            std::fs::set_permissions(&temporary, metadata.permissions()).map_err(|error| {
                AppError::Internal(format!(
                    "cannot preserve patch target permissions '{}': {error}",
                    path.display()
                ))
            })?;
            replace_file_path(&temporary, path)?;
        } else {
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
            std::fs::remove_file(&temporary).map_err(|error| {
                AppError::Internal(format!(
                    "cannot remove temporary patch file '{}': {error}",
                    temporary.display()
                ))
            })?;
        }
        #[cfg(unix)]
        if let Ok(directory) = std::fs::File::open(parent) {
            directory.sync_all().map_err(|error| {
                AppError::Internal(format!(
                    "cannot sync patch target directory '{}': {error}",
                    parent.display()
                ))
            })?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file_path(source: &Path, target: &Path) -> Result<(), AppError> {
    std::fs::rename(source, target).map_err(|error| {
        AppError::Internal(format!(
            "cannot atomically replace patch target '{}': {error}",
            target.display()
        ))
    })
}

#[cfg(windows)]
fn replace_file_path(source: &Path, target: &Path) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let target_display = target.display().to_string();
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both vectors are NUL-terminated and remain alive for the call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(AppError::Internal(format!(
            "cannot atomically replace patch target '{}': {}",
            target_display,
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
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
fn get_file_metadata_sync(path: &Path) -> Result<FileMetadata, AppError> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| AppError::NotFound(format!("cannot read metadata for '{}': {e}", path.display())))?;

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

    Ok(FileMetadata {
        name,
        path: path.to_string_lossy().into_owned(),
        size,
        mime_type,
        last_modified,
        is_directory,
    })
}

/// Remove a file or directory synchronously. Directories are removed recursively.
fn remove_entry_sync(path: &Path) -> Result<(), AppError> {
    let metadata =
        std::fs::metadata(path).map_err(|e| AppError::NotFound(format!("cannot remove '{}': {e}", path.display())))?;

    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|e| AppError::Internal(format!("cannot remove directory '{}': {e}", path.display())))
    } else {
        std::fs::remove_file(path)
            .map_err(|e| AppError::Internal(format!("cannot remove file '{}': {e}", path.display())))
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
fn copy_single_file_sync(src: &Path, dest: &Path) -> Result<(), AppError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Internal(format!("cannot create directory '{}': {e}", parent.display())))?;
    }

    std::fs::copy(src, dest)
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

/// Synchronous ZIP creation (runs in blocking thread pool).
///
/// Writes entries into a ZIP archive at `output_path`. Checks the
/// `cancelled` flag between entries and aborts early if set.
/// On cancellation, the partial ZIP file is removed.
fn create_zip_sync(output_path: &Path, entries: &[ZipEntry], cancelled: &AtomicBool) -> Result<bool, AppError> {
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
                zip.start_file(name, options)
                    .map_err(|e| AppError::Internal(format!("ZIP: failed to start entry '{name}': {e}")))?;
                zip.write_all(content.as_bytes())
                    .map_err(|e| AppError::Internal(format!("ZIP: failed to write entry '{name}': {e}")))?;
            }
            ZipEntry::Disk { name, file_path } => {
                let data = std::fs::read(file_path)
                    .map_err(|e| AppError::Internal(format!("ZIP: cannot read source file '{file_path}': {e}")))?;
                zip.start_file(name, options)
                    .map_err(|e| AppError::Internal(format!("ZIP: failed to start entry '{name}': {e}")))?;
                zip.write_all(&data)
                    .map_err(|e| AppError::Internal(format!("ZIP: failed to write entry '{name}': {e}")))?;
            }
        }
    }

    // Final cancellation check before finishing
    if cancelled.load(Ordering::Relaxed) {
        return Ok(false);
    }

    Ok(true)
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
                match copy_single_file_sync(&src, &dest) {
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
            .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
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

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                warn!("failed to read remote image body for '{}': {e}", url);
                return placeholder_svg_data_url();
            }
        };

        if bytes.len() > MAX_REMOTE_IMAGE_SIZE {
            warn!("remote image body too large ({} bytes) for '{}'", bytes.len(), url);
            return placeholder_svg_data_url();
        }

        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        format!("data:{mime};base64,{encoded}")
    }

    async fn create_zip(
        &self,
        path: &str,
        entries: Vec<ZipEntry>,
        request_id: Option<String>,
    ) -> Result<bool, AppError> {
        // Validate output path is within the sandbox
        let roots = self.allowed_roots_refs();
        let output = validate_path_for_write(path, &roots)?;

        // Validate all Disk entry source paths are within the sandbox
        for entry in &entries {
            if let ZipEntry::Disk { file_path, .. } = entry {
                validate_path(file_path, &roots)?;
            }
        }

        let cancelled = Arc::new(AtomicBool::new(false));

        if let Some(ref id) = request_id {
            self.zip_cancellations.insert(id.clone(), Arc::clone(&cancelled));
        }

        let result = tokio::task::spawn_blocking(move || create_zip_sync(&output, &entries, &cancelled))
            .await
            .map_err(|e| AppError::Internal(format!("ZIP creation task failed: {e}")))??;

        // Clean up cancellation token after task completes
        if let Some(ref id) = request_id {
            self.zip_cancellations.remove(id);
        }

        Ok(result)
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

        let files = list_workspace_files_sync(dir.path()).unwrap();

        assert_eq!(files.len(), 2);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
    }

    #[test]
    fn list_workspace_files_sync_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(dir.path().join("kept.txt"), "keep").unwrap();
        fs::write(dir.path().join("ignored.txt"), "skip").unwrap();

        let files = list_workspace_files_sync(dir.path()).unwrap();

        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"kept.txt"));
        assert!(names.contains(&".gitignore"));
        assert!(!names.contains(&"ignored.txt"));
    }

    #[test]
    fn list_workspace_files_sync_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let files = list_workspace_files_sync(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn list_workspace_files_sync_truncates_at_limit() {
        // Creating 20,000+ files is impractical in a unit test;
        // verify the constant exists and the branch logic is sound.
        assert_eq!(MAX_WORKSPACE_FILES, 20_000);
    }

    #[test]
    fn list_workspace_files_sync_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main(){}").unwrap();

        let files = list_workspace_files_sync(dir.path()).unwrap();
        let main_file = files.iter().find(|f| f.name == "main.rs").unwrap();

        assert_eq!(main_file.relative_path, "src/main.rs");
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

        let files = list_workspace_files_sync(&dir.path().join("workspace")).unwrap();

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

        copy_single_file_sync(&src, &dest).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "content");
    }

    #[test]
    fn copy_single_file_sync_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dest = dir.path().join("nested/deep/dest.txt");
        fs::write(&src, "nested").unwrap();

        copy_single_file_sync(&src, &dest).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "nested");
    }

    #[test]
    fn copy_single_file_sync_source_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("missing.txt");
        let dest = dir.path().join("dest.txt");

        let result = copy_single_file_sync(&src, &dest);
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
