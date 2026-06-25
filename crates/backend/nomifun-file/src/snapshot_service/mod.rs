//! Git-based workspace snapshot service.
//!
//! Supports two modes:
//! - **git-repo**: directory already has `.git` — uses it directly.
//! - **snapshot**: no `.git` — creates a temporary git repo that tracks the
//!   workspace via a separate worktree.

mod helpers;

use dashmap::DashMap;
use git2::Repository;
use nomifun_common::{AppError, FileChangeOperation};

use crate::types::{CompareResult, SnapshotInfo, SnapshotMode};

use helpers::{
    SNAPSHOT_DIR_PREFIX, WorkspaceState, build_info, discard_single_file, init_snapshot_repo, list_branches, open_repo,
    parse_statuses, read_baseline, reset_single_file, resolve_workspace, snapshot_guard, stage_all_with_deletions,
    stage_single_file, temp_repo_path, unstage_all_files, unstage_single_file,
};

// ---------------------------------------------------------------------------
// SnapshotService
// ---------------------------------------------------------------------------

/// Git-based workspace snapshot service.
pub struct SnapshotService {
    workspaces: DashMap<String, WorkspaceState>,
}

impl Default for SnapshotService {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotService {
    pub fn new() -> Self {
        Self {
            workspaces: DashMap::new(),
        }
    }

    /// Number of currently-tracked workspaces. Test/observability helper.
    #[doc(hidden)]
    pub fn workspace_count(&self) -> usize {
        self.workspaces.len()
    }

    /// Whether the workspace string (after canonicalization) is currently
    /// tracked. Test/observability helper.
    #[doc(hidden)]
    pub fn is_tracked(&self, workspace: &str) -> bool {
        self.workspaces.contains_key(&workspace_key(workspace))
    }

    /// The git/temp repo path backing a tracked workspace, if any.
    /// Test/observability helper.
    #[doc(hidden)]
    pub fn repo_path_for(&self, workspace: &str) -> Option<std::path::PathBuf> {
        self.workspaces.get(&workspace_key(workspace)).map(|s| s.repo_path.clone())
    }

    /// Remove leftover `nomifun-snapshot-*` directories from the system temp
    /// dir. Call once at application startup.
    pub fn cleanup_stale_snapshots() {
        let temp_dir = std::env::temp_dir();
        let entries = match std::fs::read_dir(&temp_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to read temp dir for snapshot cleanup"
                );
                return;
            }
        };
        for entry in entries.flatten() {
            let name = match entry.file_name().into_string() {
                Ok(n) => n,
                Err(_) => continue,
            };
            if name.starts_with(SNAPSHOT_DIR_PREFIX) {
                let path = entry.path();
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to clean up stale snapshot directory"
                    );
                } else {
                    tracing::info!(
                        path = %path.display(),
                        "Cleaned up stale snapshot directory"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: get workspace state or return error
// ---------------------------------------------------------------------------

/// Resolve the DashMap key for a workspace string. Falls back to the raw
/// string when the path can no longer be canonicalized (e.g. the directory
/// was removed) so a still-tracked entry remains reachable for dispose.
fn workspace_key(workspace: &str) -> String {
    match resolve_workspace(workspace) {
        Ok(canonical) => canonical.to_string_lossy().to_string(),
        Err(_) => workspace.to_owned(),
    }
}

fn get_state(workspaces: &DashMap<String, WorkspaceState>, workspace: &str) -> Result<WorkspaceState, AppError> {
    let key = workspace_key(workspace);
    workspaces
        .get(&key)
        .map(|r| r.clone())
        .ok_or_else(|| AppError::BadRequest(format!("Workspace not initialized: {}", workspace)))
}

// ---------------------------------------------------------------------------
// ISnapshotService implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl crate::traits::ISnapshotService for SnapshotService {
    async fn init(&self, workspace: &str) -> Result<SnapshotInfo, AppError> {
        // Canonicalize up front so the DashMap key is the canonical path
        // string. Two raw forms that resolve to the same directory (trailing
        // separator, case differences on Windows, `.`/`..` segments) collapse
        // to a single entry.
        let canonical = {
            let ws = workspace.to_owned();
            tokio::task::spawn_blocking(move || resolve_workspace(&ws))
                .await
                .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))??
        };
        let key = canonical.to_string_lossy().to_string();

        // Check if already initialized (keyed by canonical path)
        if let Some(mut entry) = self.workspaces.get_mut(&key) {
            entry.refcount += 1;
            let st = entry.clone();
            drop(entry);
            return tokio::task::spawn_blocking(move || {
                let repo = open_repo(&st)?;
                Ok(build_info(st.mode, &repo))
            })
            .await
            .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?;
        }

        let result = tokio::task::spawn_blocking(move || {
            let canonical_str = canonical.to_string_lossy().to_string();

            let git_dir = canonical.join(".git");
            if git_dir.exists() {
                // GitRepo mode is cheap and safe -- never consult the guard.
                let mode = SnapshotMode::GitRepo;
                let repo_path = canonical.clone();
                let state = WorkspaceState {
                    mode: mode.clone(),
                    repo_path: repo_path.clone(),
                    workspace_path: canonical,
                    refcount: 1,
                };
                let repo = Repository::open(&repo_path)
                    .map_err(|e| AppError::Internal(format!("Failed to open repo after init: {}", e)))?;
                let info = build_info(mode, &repo);
                return Ok::<(Option<WorkspaceState>, SnapshotInfo), AppError>((Some(state), info));
            }

            // Snapshot branch: run the safety guard BEFORE creating any temp
            // repo. On refusal, return a Disabled info and track nothing.
            if let Some(reason) = snapshot_guard(&canonical) {
                let info = SnapshotInfo {
                    mode: SnapshotMode::Disabled { reason },
                    branch: None,
                };
                return Ok((None, info));
            }

            let temp = temp_repo_path(&canonical_str);
            init_snapshot_repo(&canonical, &temp)?;
            let mode = SnapshotMode::Snapshot;
            let state = WorkspaceState {
                mode: mode.clone(),
                repo_path: temp.clone(),
                workspace_path: canonical,
                refcount: 1,
            };
            let repo = Repository::open(&temp)
                .map_err(|e| AppError::Internal(format!("Failed to open repo after init: {}", e)))?;
            let info = build_info(mode, &repo);

            Ok::<(Option<WorkspaceState>, SnapshotInfo), AppError>((Some(state), info))
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))??;

        let (maybe_state, info) = result;
        // Disabled workspaces are not tracked: there is no repo to operate on,
        // and the client reads the disabled state directly from this response.
        let state = match maybe_state {
            Some(s) => s,
            None => return Ok(info),
        };
        // If a concurrent init won the race and inserted an entry while this
        // task was building, fold into it (bump refcount) rather than clobber.
        match self.workspaces.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(mut e) => {
                e.get_mut().refcount += 1;
            }
            dashmap::mapref::entry::Entry::Vacant(e) => {
                e.insert(state);
            }
        }
        Ok(info)
    }

    async fn get_info(&self, workspace: &str) -> Result<SnapshotInfo, AppError> {
        let state = get_state(&self.workspaces, workspace)?;

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            Ok(build_info(state.mode, &repo))
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn compare(&self, workspace: &str) -> Result<CompareResult, AppError> {
        let state = get_state(&self.workspaces, workspace)?;

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            parse_statuses(&repo, &state.workspace_path)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn get_baseline_content(&self, workspace: &str, file_path: &str) -> Result<Option<String>, AppError> {
        let state = get_state(&self.workspaces, workspace)?;
        let rel = file_path.to_owned();

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            read_baseline(&repo, &rel)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn stage_file(&self, workspace: &str, file_path: &str) -> Result<(), AppError> {
        let state = get_state(&self.workspaces, workspace)?;
        let fp = file_path.to_owned();

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            stage_single_file(&repo, &fp)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn stage_all(&self, workspace: &str) -> Result<(), AppError> {
        let state = get_state(&self.workspaces, workspace)?;

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            stage_all_with_deletions(&repo)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn unstage_file(&self, workspace: &str, file_path: &str) -> Result<(), AppError> {
        let state = get_state(&self.workspaces, workspace)?;
        let fp = file_path.to_owned();

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            unstage_single_file(&repo, &fp)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn unstage_all(&self, workspace: &str) -> Result<(), AppError> {
        let state = get_state(&self.workspaces, workspace)?;

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            unstage_all_files(&repo)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn discard_file(
        &self,
        workspace: &str,
        file_path: &str,
        operation: FileChangeOperation,
    ) -> Result<(), AppError> {
        let state = get_state(&self.workspaces, workspace)?;
        let fp = file_path.to_owned();

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            discard_single_file(&repo, &state.workspace_path, &fp, operation)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn reset_file(
        &self,
        workspace: &str,
        file_path: &str,
        operation: FileChangeOperation,
    ) -> Result<(), AppError> {
        let state = get_state(&self.workspaces, workspace)?;
        let fp = file_path.to_owned();

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            reset_single_file(&repo, &state.workspace_path, &fp, operation)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn get_branches(&self, workspace: &str) -> Result<Vec<String>, AppError> {
        let state = get_state(&self.workspaces, workspace)?;

        tokio::task::spawn_blocking(move || {
            let repo = open_repo(&state)?;
            list_branches(&repo)
        })
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
    }

    async fn dispose(&self, workspace: &str) -> Result<(), AppError> {
        let key = workspace_key(workspace);

        // Decrement the refcount under the shard lock. Only the call that
        // drops it to 0 proceeds to actually remove the entry and clean up.
        // `remove_if` holds the lock across the predicate, so the decrement and
        // the remove decision are atomic w.r.t. a concurrent `init` bump.
        let removed = self.workspaces.remove_if_mut(&key, |_, state| {
            state.refcount = state.refcount.saturating_sub(1);
            state.refcount == 0
        });

        let state = match removed {
            // refcount hit 0 -> entry removed, proceed to clean up.
            Some((_, s)) => s,
            // Either not tracked (idempotent) or refcount still > 0 -> keep it.
            None => return Ok(()),
        };

        if state.mode == SnapshotMode::Snapshot {
            let repo_path = state.repo_path.clone();
            tokio::task::spawn_blocking(move || {
                if repo_path.exists() {
                    std::fs::remove_dir_all(&repo_path).map_err(|e| {
                        AppError::Internal(format!("Failed to remove snapshot dir {}: {}", repo_path.display(), e))
                    })?;
                }
                Ok(())
            })
            .await
            .map_err(|e| AppError::Internal(format!("Blocking task failed: {}", e)))?
        } else {
            // git-repo mode: nothing to clean up
            Ok(())
        }
    }
}
