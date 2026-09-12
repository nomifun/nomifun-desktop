//! Typed, workspace-scoped Git tools for the in-process Nomi engine.
//!
//! These tools deliberately expose Git operations instead of widening a VCS
//! capability into the general-purpose `Bash` tool.  Every path is resolved
//! against the immutable session workspace, status/diff results are projected
//! back into that workspace when it is a repository subdirectory, and commit
//! refuses to include staged paths outside the bound workspace.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::Tool;

const MAX_STATUS_ENTRIES: usize = 2_000;
const MAX_DIFF_BYTES: usize = 1024 * 1024;
const MAX_STAGE_ENTRIES: usize = 100_000;
const MAX_COMMIT_MESSAGE_CHARS: usize = 512;
const VCS_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VcsOperation {
    Status,
    Diff,
    Stage,
    Commit,
    ReviewStatus,
}

impl VcsOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::Status => "vcs.status",
            Self::Diff => "vcs.diff",
            Self::Stage => "vcs.stage",
            Self::Commit => "vcs.commit",
            Self::ReviewStatus => "review.status",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Status => {
                "Read Git status for the session workspace. Paths outside a workspace \
                 that is nested inside a larger repository are omitted."
            }
            Self::Diff => {
                "Read staged, modified, and untracked Git patches for the session workspace. \
                 An optional normalized workspace-relative path narrows the result."
            }
            Self::Stage => {
                "Stage one normalized workspace-relative file or directory, including tracked \
                 deletions. The operation cannot stage paths outside the session workspace."
            }
            Self::Commit => {
                "Commit the currently staged changes only when every staged path belongs to the \
                 session workspace. Git user.name and user.email must already be configured."
            }
            Self::ReviewStatus => {
                "Inspect the workspace's current review state through one bounded, structured \
                 status and diff snapshot. This is a read-only review result, not an approval gate."
            }
        }
    }

    const fn category(self) -> ToolCategory {
        match self {
            Self::Status | Self::Diff | Self::ReviewStatus => ToolCategory::Info,
            Self::Stage | Self::Commit => ToolCategory::Edit,
        }
    }
}

/// One native Git operation bound to a fixed session workspace.
pub struct VcsTool {
    operation: VcsOperation,
    workspace: PathBuf,
    mutation_lock: Arc<Mutex<()>>,
}

impl VcsTool {
    fn new(
        operation: VcsOperation,
        workspace: PathBuf,
        mutation_lock: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            operation,
            workspace,
            mutation_lock,
        }
    }
}

/// Build the complete local VCS family with one shared mutation lock.
pub fn local_vcs_tools(workspace: impl Into<PathBuf>) -> Vec<Box<dyn Tool>> {
    let workspace = workspace.into();
    let mutation_lock = Arc::new(Mutex::new(()));
    [
        VcsOperation::Status,
        VcsOperation::Diff,
        VcsOperation::Stage,
        VcsOperation::Commit,
        VcsOperation::ReviewStatus,
    ]
    .into_iter()
    .map(|operation| {
        Box::new(VcsTool::new(
            operation,
            workspace.clone(),
            Arc::clone(&mutation_lock),
        )) as Box<dyn Tool>
    })
    .collect()
}

#[async_trait]
impl Tool for VcsTool {
    fn name(&self) -> &str {
        self.operation.name()
    }

    fn description(&self) -> &str {
        self.operation.description()
    }

    fn input_schema(&self) -> JsonSchema {
        match self.operation {
            VcsOperation::Status => json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            VcsOperation::Diff | VcsOperation::ReviewStatus => json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Optional normalized workspace-relative path using '/' separators."
                    }
                },
                "additionalProperties": false
            }),
            VcsOperation::Stage => json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Normalized workspace-relative file or directory using '/' separators."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            VcsOperation::Commit => json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Non-empty commit message, at most 512 characters."
                    }
                },
                "required": ["message"],
                "additionalProperties": false
            }),
        }
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        matches!(self.operation, VcsOperation::Status | VcsOperation::Diff)
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let result = match self.operation {
            VcsOperation::Status => {
                let workspace = self.workspace.clone();
                run_blocking(move || vcs_status(&workspace)).await
            }
            VcsOperation::Diff => {
                let path = match optional_relative_path(&input, "path") {
                    Ok(path) => path,
                    Err(error) => return error.into_result(self.operation.name()),
                };
                let workspace = self.workspace.clone();
                run_blocking(move || vcs_diff(&workspace, path.as_deref())).await
            }
            VcsOperation::Stage => {
                let path = match required_relative_path(&input, "path") {
                    Ok(path) => path,
                    Err(error) => return error.into_result(self.operation.name()),
                };
                let _mutation = self.mutation_lock.lock().await;
                let workspace = self.workspace.clone();
                run_blocking(move || vcs_stage(&workspace, &path)).await
            }
            VcsOperation::Commit => {
                let message = match required_commit_message(&input) {
                    Ok(message) => message,
                    Err(error) => return error.into_result(self.operation.name()),
                };
                let _mutation = self.mutation_lock.lock().await;
                let workspace = self.workspace.clone();
                run_blocking(move || vcs_commit(&workspace, &message)).await
            }
            VcsOperation::ReviewStatus => {
                let path = match optional_relative_path(&input, "path") {
                    Ok(path) => path,
                    Err(error) => return error.into_result(self.operation.name()),
                };
                let workspace = self.workspace.clone();
                run_blocking(move || review_status(&workspace, path.as_deref())).await
            }
        };
        match result {
            Ok(value) => ToolResult::text(
                serde_json::to_string_pretty(&value)
                    .unwrap_or_else(|_| value.to_string()),
            ),
            Err(error) => error.into_result(self.operation.name()),
        }
    }

    fn execution_timeout(&self, _input: &Value) -> Duration {
        VCS_TOOL_TIMEOUT
    }

    fn max_result_size(&self) -> usize {
        MAX_DIFF_BYTES + 16 * 1024
    }

    fn category(&self) -> ToolCategory {
        self.operation.category()
    }

    fn describe(&self, input: &Value) -> String {
        match self.operation {
            VcsOperation::Status => "vcs.status: inspect workspace changes".to_owned(),
            VcsOperation::Diff => format!(
                "vcs.diff: {}",
                input
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("entire workspace")
            ),
            VcsOperation::Stage => format!(
                "vcs.stage: {}",
                input.get("path").and_then(Value::as_str).unwrap_or("<missing>")
            ),
            VcsOperation::Commit => {
                let message = input
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing>");
                format!("vcs.commit: {}", crate::truncate_utf8(message, 80))
            }
            VcsOperation::ReviewStatus => format!(
                "review.status: {}",
                input
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("entire workspace")
            ),
        }
    }
}

/// Stable status returned by the Nomi coding review workflow.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewWorkflowStatus {
    Clean,
    ChangesPresent,
    Incomplete,
}

/// One bounded, read-only review snapshot for the current session workspace.
///
/// Keeping status and diff together prevents callers from treating two reads
/// taken across an intervening edit as one coherent review result. The result
/// intentionally carries no approval or merge state: FullAuto remains the
/// product execution policy.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewWorkflowResult {
    pub schema_version: String,
    pub status: ReviewWorkflowStatus,
    pub change_count: usize,
    pub truncated: bool,
    pub status_snapshot: Value,
    pub diff_snapshot: Value,
}

async fn run_blocking<F>(work: F) -> Result<Value, VcsToolError>
where
    F: FnOnce() -> Result<Value, VcsToolError> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| VcsToolError::unavailable(format!("Git worker failed: {error}")))?
}

#[derive(Debug)]
struct VcsToolError {
    code: &'static str,
    message: String,
}

impl VcsToolError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_PAYLOAD",
            message: message.into(),
        }
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: "CAPABILITY_UNAVAILABLE",
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "RESOURCE_NOT_FOUND",
            message: message.into(),
        }
    }

    fn outside_workspace(message: impl Into<String>) -> Self {
        Self {
            code: "PRESET_RESOURCE_NOT_BOUND",
            message: message.into(),
        }
    }

    fn into_result(self, tool: &str) -> ToolResult {
        ToolResult::error(format!("{tool} [{}]: {}", self.code, self.message))
    }
}

fn optional_relative_path(input: &Value, field: &str) -> Result<Option<String>, VcsToolError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(path)) if path.trim().is_empty() => Ok(None),
        Some(Value::String(path)) => validate_relative_path(path).map(Some),
        Some(_) => Err(VcsToolError::invalid(format!(
            "{field} must be a string"
        ))),
    }
}

fn required_relative_path(input: &Value, field: &str) -> Result<String, VcsToolError> {
    let path = input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| VcsToolError::invalid(format!("{field} is required")))?;
    if path.trim().is_empty() {
        return Err(VcsToolError::invalid(format!("{field} must not be empty")));
    }
    validate_relative_path(path)
}

fn validate_relative_path(path: &str) -> Result<String, VcsToolError> {
    if path.trim() != path || path.contains(['\0', '\\']) {
        return Err(VcsToolError::invalid(
            "Git paths must be normalized workspace-relative values using '/' separators",
        ));
    }
    let value = Path::new(path);
    if value.is_absolute()
        || path.starts_with('/')
        || value.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(VcsToolError::outside_workspace(
            "Git path escapes the session workspace",
        ));
    }
    let normalized = value
        .components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        return Err(VcsToolError::invalid("Git path must not resolve to '.'"));
    }
    Ok(normalized)
}

fn required_commit_message(input: &Value) -> Result<String, VcsToolError> {
    let message = input
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| VcsToolError::invalid("message is required"))?
        .trim();
    if message.is_empty() {
        return Err(VcsToolError::invalid("message must not be empty"));
    }
    if message.chars().count() > MAX_COMMIT_MESSAGE_CHARS {
        return Err(VcsToolError::invalid(format!(
            "message must not exceed {MAX_COMMIT_MESSAGE_CHARS} characters"
        )));
    }
    Ok(message.to_owned())
}

fn vcs_status(workspace: &Path) -> Result<Value, VcsToolError> {
    let (repository, workspace_prefix) = scoped_repository(workspace)?;
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let statuses = repository.statuses(Some(&mut options)).map_err(|error| {
        VcsToolError::unavailable(format!("could not read Git status: {error}"))
    })?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in statuses.iter() {
        let Some(path) = entry.path() else {
            continue;
        };
        let Some(path) = path_relative_to_workspace(path, &workspace_prefix) else {
            continue;
        };
        if entries.len() == MAX_STATUS_ENTRIES {
            truncated = true;
            break;
        }
        entries.push(json!({
            "path": path,
            "status": git_status_names(entry.status()),
        }));
    }
    Ok(json!({
        "repository": "workspace",
        "entries": entries,
        "truncated": truncated,
    }))
}

fn vcs_diff(workspace: &Path, path: Option<&str>) -> Result<Value, VcsToolError> {
    let (repository, workspace_prefix) = scoped_repository(workspace)?;
    let pathspec = path.map(|path| join_repo_path(&workspace_prefix, path));
    let scope_pathspec = pathspec
        .as_deref()
        .or_else(|| (!workspace_prefix.is_empty()).then_some(workspace_prefix.as_str()));
    let head_tree = repository
        .head()
        .ok()
        .and_then(|head| head.peel_to_tree().ok());

    let mut staged_options = git2::DiffOptions::new();
    if let Some(pathspec) = scope_pathspec {
        staged_options.pathspec(pathspec);
    }
    let staged = repository
        .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut staged_options))
        .map_err(|error| {
            VcsToolError::unavailable(format!("could not read staged Git diff: {error}"))
        })?;

    let mut unstaged_options = git2::DiffOptions::new();
    unstaged_options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    if let Some(pathspec) = scope_pathspec {
        unstaged_options.pathspec(pathspec);
    }
    let unstaged = repository
        .diff_index_to_workdir(None, Some(&mut unstaged_options))
        .map_err(|error| {
            VcsToolError::unavailable(format!("could not read unstaged Git diff: {error}"))
        })?;

    let mut staged_patch = String::new();
    let mut truncated = false;
    append_diff_patch(&staged, &mut staged_patch, &mut truncated)?;
    let mut unstaged_patch = String::new();
    append_diff_patch(&unstaged, &mut unstaged_patch, &mut truncated)?;
    Ok(json!({
        "path": path,
        "patch": format!("{staged_patch}{unstaged_patch}"),
        "staged_patch": staged_patch,
        "unstaged_patch": unstaged_patch,
        "truncated": truncated,
    }))
}

fn review_status(workspace: &Path, path: Option<&str>) -> Result<Value, VcsToolError> {
    // Run both reads on the same blocking worker. Git may still change because
    // another process owns the repository, so any truncation marks the result
    // incomplete instead of overstating that the full change set was reviewed.
    let status_snapshot = vcs_status(workspace)?;
    let diff_snapshot = vcs_diff(workspace, path)?;
    let change_count = status_snapshot
        .get("entries")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let truncated = status_snapshot
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(true)
        || diff_snapshot
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(true);
    let status = if truncated {
        ReviewWorkflowStatus::Incomplete
    } else if change_count == 0 {
        ReviewWorkflowStatus::Clean
    } else {
        ReviewWorkflowStatus::ChangesPresent
    };
    serde_json::to_value(ReviewWorkflowResult {
        schema_version: "1.0.0".to_owned(),
        status,
        change_count,
        truncated,
        status_snapshot,
        diff_snapshot,
    })
    .map_err(|error| {
        VcsToolError::unavailable(format!("could not encode review result: {error}"))
    })
}

fn vcs_stage(workspace: &Path, path: &str) -> Result<Value, VcsToolError> {
    let canonical_workspace = std::fs::canonicalize(workspace).map_err(|error| {
        VcsToolError::not_found(format!("workspace is unavailable: {error}"))
    })?;
    let resolved = canonical_workspace.join(path);
    let target_metadata = std::fs::symlink_metadata(&resolved).ok();
    if let Some(metadata) = target_metadata.as_ref() {
        if metadata.file_type().is_symlink() || metadata_is_windows_reparse_point(metadata) {
            return Err(VcsToolError::outside_workspace(
                "vcs.stage refuses symlink or reparse-point targets",
            ));
        }
        let canonical_target = std::fs::canonicalize(&resolved).map_err(|error| {
            VcsToolError::not_found(format!("stage target is unavailable: {error}"))
        })?;
        if !canonical_target.starts_with(&canonical_workspace) {
            return Err(VcsToolError::outside_workspace(
                "stage target escapes the session workspace",
            ));
        }
    }

    let (repository, workspace_prefix) = scoped_repository(&canonical_workspace)?;
    let repo_path = join_repo_path(&workspace_prefix, path);
    let mut index = repository
        .index()
        .map_err(|error| VcsToolError::unavailable(format!("could not open Git index: {error}")))?;
    match target_metadata {
        Some(metadata) if metadata.is_dir() => {
            let mut stage_paths = Vec::new();
            collect_directory_stage_paths(&resolved, &repo_path, &mut stage_paths)?;
            for indexed_path in indexed_paths_for_target(&index, &repo_path)? {
                index.remove_path(&indexed_path).map_err(|error| {
                    VcsToolError::unavailable(format!(
                        "could not refresh staged path {}: {error}",
                        indexed_path.display()
                    ))
                })?;
            }
            for stage_path in stage_paths {
                index.add_path(&stage_path).map_err(|error| {
                    VcsToolError::unavailable(format!(
                        "could not stage {}: {error}",
                        stage_path.display()
                    ))
                })?;
            }
        }
        Some(_) => {
            index.add_path(Path::new(&repo_path)).map_err(|error| {
                VcsToolError::unavailable(format!("could not stage {path}: {error}"))
            })?;
        }
        None => {
            let indexed_paths = indexed_paths_for_target(&index, &repo_path)?;
            if indexed_paths.is_empty() {
                return Err(VcsToolError::not_found(format!(
                    "path {path:?} is not tracked"
                )));
            }
            for indexed_path in indexed_paths {
                index.remove_path(&indexed_path).map_err(|error| {
                    VcsToolError::unavailable(format!(
                        "could not stage deletion {}: {error}",
                        indexed_path.display()
                    ))
                })?;
            }
        }
    }
    index.write().map_err(|error| {
        VcsToolError::unavailable(format!("could not persist Git index: {error}"))
    })?;
    Ok(json!({ "path": path, "staged": true }))
}

fn vcs_commit(workspace: &Path, message: &str) -> Result<Value, VcsToolError> {
    let (repository, workspace_prefix) = scoped_repository(workspace)?;
    let mut index = repository
        .index()
        .map_err(|error| VcsToolError::unavailable(format!("could not open Git index: {error}")))?;
    let parent = match repository.head() {
        Ok(head) if head.target().is_none() => {
            if repository
                .is_empty()
                .map_err(|error| VcsToolError::unavailable(error.to_string()))?
            {
                None
            } else {
                return Err(VcsToolError::unavailable(
                    "repository has an unborn HEAD but is not empty",
                ));
            }
        }
        Ok(head) => Some(head.peel_to_commit().map_err(|error| {
            VcsToolError::unavailable(format!("could not read HEAD commit: {error}"))
        })?),
        Err(error)
            if matches!(
                error.code(),
                git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
            ) && repository
                .is_empty()
                .map_err(|inspect| VcsToolError::unavailable(inspect.to_string()))? =>
        {
            None
        }
        Err(error) => {
            return Err(VcsToolError::unavailable(format!(
                "could not read repository HEAD: {error}"
            )));
        }
    };
    let parent_tree = parent
        .as_ref()
        .map(|commit| commit.tree())
        .transpose()
        .map_err(|error| {
            VcsToolError::unavailable(format!("could not read parent tree: {error}"))
        })?;
    let staged = repository
        .diff_tree_to_index(parent_tree.as_ref(), Some(&index), None)
        .map_err(|error| {
            VcsToolError::unavailable(format!("could not inspect staged changes: {error}"))
        })?;
    let mut scoped_paths = Vec::new();
    for delta in staged.deltas() {
        let old_path = delta.old_file().path().map(git_path_to_string).transpose()?;
        let new_path = delta.new_file().path().map(git_path_to_string).transpose()?;
        if old_path.is_none() && new_path.is_none() {
            return Err(VcsToolError::unavailable(
                "staged change has no representable path",
            ));
        }
        for path in [old_path.as_deref(), new_path.as_deref()]
            .into_iter()
            .flatten()
        {
            let Some(relative) = path_relative_to_workspace(path, &workspace_prefix) else {
                return Err(VcsToolError::outside_workspace(
                    "vcs.commit refuses staged paths outside the session workspace",
                ));
            };
            scoped_paths.push(relative);
        }
    }
    if scoped_paths.is_empty() {
        return Err(VcsToolError::unavailable(
            "there are no staged changes in the session workspace",
        ));
    }
    scoped_paths.sort();
    scoped_paths.dedup();

    let tree_id = index.write_tree().map_err(|error| {
        VcsToolError::unavailable(format!("could not write Git tree: {error}"))
    })?;
    let tree = repository.find_tree(tree_id).map_err(|error| {
        VcsToolError::unavailable(format!("could not reload Git tree: {error}"))
    })?;
    let signature = repository.signature().map_err(|error| {
        VcsToolError::unavailable(format!(
            "Git user.name and user.email must be configured: {error}"
        ))
    })?;
    let parents = parent.iter().collect::<Vec<_>>();
    let commit_id = repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(|error| {
            VcsToolError::unavailable(format!("could not create Git commit: {error}"))
        })?;
    Ok(json!({
        "committed": true,
        "commit_id": commit_id.to_string(),
        "message": message,
        "paths": scoped_paths,
    }))
}

fn append_diff_patch(
    diff: &git2::Diff<'_>,
    patch: &mut String,
    truncated: &mut bool,
) -> Result<(), VcsToolError> {
    if *truncated {
        return Ok(());
    }
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if line.origin() != '\0' {
            patch.push(line.origin());
        }
        patch.push_str(&String::from_utf8_lossy(line.content()));
        if patch.len() > MAX_DIFF_BYTES {
            let mut end = MAX_DIFF_BYTES;
            while end > 0 && !patch.is_char_boundary(end) {
                end -= 1;
            }
            patch.truncate(end);
            *truncated = true;
            false
        } else {
            true
        }
    })
    .map_err(|error| VcsToolError::unavailable(format!("could not render Git diff: {error}")))
}

fn scoped_repository(workspace: &Path) -> Result<(git2::Repository, String), VcsToolError> {
    let repository = git2::Repository::discover(workspace)
        .map_err(|error| VcsToolError::not_found(format!("workspace is not a Git repository: {error}")))?;
    let repository_root = repository.workdir().ok_or_else(|| {
        VcsToolError::not_found("Git repository has no working directory")
    })?;
    let repository_root = std::fs::canonicalize(repository_root).map_err(|error| {
        VcsToolError::not_found(format!("repository working directory is unavailable: {error}"))
    })?;
    let workspace = std::fs::canonicalize(workspace)
        .map_err(|error| VcsToolError::not_found(format!("workspace is unavailable: {error}")))?;
    let relative = workspace.strip_prefix(&repository_root).map_err(|_| {
        VcsToolError::outside_workspace("workspace is outside the discovered Git repository")
    })?;
    let prefix = git_path_to_string(relative)?.trim_matches('/').to_owned();
    Ok((repository, prefix))
}

fn git_path_to_string(path: &Path) -> Result<String, VcsToolError> {
    path.to_str()
        .map(|value| {
            if cfg!(windows) {
                value.replace('\\', "/")
            } else {
                value.to_owned()
            }
        })
        .ok_or_else(|| {
            VcsToolError::unavailable("Git path is not valid UTF-8 and cannot be projected safely")
        })
}

fn path_component_matches(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn path_relative_to_workspace(path: &str, prefix: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    let prefix = prefix.replace('\\', "/");
    if prefix.is_empty() {
        return Some(path);
    }
    let mut path_components = path.split('/');
    for expected in prefix.split('/') {
        let actual = path_components.next()?;
        if !path_component_matches(actual, expected) {
            return None;
        }
    }
    Some(path_components.collect::<Vec<_>>().join("/"))
}

fn join_repo_path(prefix: &str, relative: &str) -> String {
    let relative = relative.trim_matches('/');
    if prefix.is_empty() {
        relative.to_owned()
    } else if relative.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}/{relative}")
    }
}

fn git_status_names(status: git2::Status) -> Vec<&'static str> {
    let mut names = Vec::new();
    for (flag, name) in [
        (git2::Status::INDEX_NEW, "index_new"),
        (git2::Status::INDEX_MODIFIED, "index_modified"),
        (git2::Status::INDEX_DELETED, "index_deleted"),
        (git2::Status::INDEX_RENAMED, "index_renamed"),
        (git2::Status::INDEX_TYPECHANGE, "index_typechange"),
        (git2::Status::WT_NEW, "worktree_new"),
        (git2::Status::WT_MODIFIED, "worktree_modified"),
        (git2::Status::WT_DELETED, "worktree_deleted"),
        (git2::Status::WT_RENAMED, "worktree_renamed"),
        (git2::Status::WT_TYPECHANGE, "worktree_typechange"),
        (git2::Status::CONFLICTED, "conflicted"),
    ] {
        if status.contains(flag) {
            names.push(name);
        }
    }
    names
}

fn indexed_paths_for_target(
    index: &git2::Index,
    repo_path: &str,
) -> Result<Vec<PathBuf>, VcsToolError> {
    let mut paths = Vec::new();
    for entry in index.iter() {
        let candidate = std::str::from_utf8(&entry.path).map_err(|_| {
            VcsToolError::unavailable("Git index contains a non-UTF-8 path")
        })?;
        if path_relative_to_workspace(candidate, repo_path).is_some() {
            paths.push(PathBuf::from(candidate));
        }
    }
    Ok(paths)
}

fn collect_directory_stage_paths(
    directory: &Path,
    repo_path: &str,
    output: &mut Vec<PathBuf>,
) -> Result<(), VcsToolError> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| {
            VcsToolError::not_found(format!(
                "could not read directory {}: {error}",
                directory.display()
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            VcsToolError::not_found(format!(
                "could not enumerate directory {}: {error}",
                directory.display()
            ))
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if entry.file_name() == ".git" {
            continue;
        }
        if output.len() == MAX_STAGE_ENTRIES {
            return Err(VcsToolError::unavailable(format!(
                "directory exceeds the {MAX_STAGE_ENTRIES}-entry staging limit"
            )));
        }
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            VcsToolError::not_found(format!(
                "could not inspect {}: {error}",
                entry.path().display()
            ))
        })?;
        if metadata.file_type().is_symlink() || metadata_is_windows_reparse_point(&metadata) {
            return Err(VcsToolError::outside_workspace(format!(
                "vcs.stage refuses symlink or reparse-point entry {}",
                entry.path().display()
            )));
        }
        let entry_repo_path = join_repo_path(
            repo_path,
            &entry.file_name().to_string_lossy().replace('\\', "/"),
        );
        if metadata.is_dir() {
            collect_directory_stage_paths(&entry.path(), &entry_repo_path, output)?;
        } else {
            output.push(PathBuf::from(entry_repo_path));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_windows_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_windows_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize_repository(path: &Path) -> git2::Repository {
        let repository = git2::Repository::init(path).expect("repository");
        {
            let mut config = repository.config().expect("config");
            config.set_str("user.name", "NomiFun Test").unwrap();
            config
                .set_str("user.email", "nomifun-test@nomifun.invalid")
                .unwrap();
        }
        std::fs::write(path.join("tracked.txt"), "base\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repository.find_tree(tree_id).unwrap();
        let signature = repository.signature().unwrap();
        repository
            .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        drop(tree);
        repository
    }

    async fn invoke(tool: &dyn Tool, input: Value) -> ToolResult {
        tool.execute(input).await
    }

    #[tokio::test]
    async fn status_diff_stage_and_commit_are_workspace_scoped() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "base\nchanged\n").unwrap();
        let tools = local_vcs_tools(directory.path());

        let status = invoke(tools[0].as_ref(), json!({})).await;
        assert!(!status.is_error, "{}", status.content);
        assert!(
            status.content.contains("worktree_modified"),
            "{}",
            status.content
        );

        let diff = invoke(tools[1].as_ref(), json!({})).await;
        assert!(!diff.is_error, "{}", diff.content);
        assert!(diff.content.contains("changed"));

        let stage = invoke(tools[2].as_ref(), json!({"path": "tracked.txt"})).await;
        assert!(!stage.is_error, "{}", stage.content);
        assert!(
            repository
                .statuses(None)
                .unwrap()
                .iter()
                .any(|entry| entry.status().contains(git2::Status::INDEX_MODIFIED))
        );

        let commit = invoke(
            tools[3].as_ref(),
            json!({"message": "record typed VCS change"}),
        )
        .await;
        assert!(!commit.is_error, "{}", commit.content);
        assert_eq!(
            repository.head().unwrap().peel_to_commit().unwrap().message(),
            Some("record typed VCS change")
        );
    }

    #[tokio::test]
    async fn diff_includes_untracked_file_content() {
        let directory = tempfile::tempdir().unwrap();
        let _repository = git2::Repository::init(directory.path()).unwrap();
        std::fs::write(directory.path().join("new.txt"), "alpha\nbeta\n").unwrap();
        let tools = local_vcs_tools(directory.path());

        let diff = invoke(tools[1].as_ref(), json!({"path": "new.txt"})).await;
        assert!(!diff.is_error, "{}", diff.content);
        assert!(diff.content.contains("new.txt"), "{}", diff.content);
        assert!(diff.content.contains("+beta"), "{}", diff.content);
    }

    #[tokio::test]
    async fn review_status_returns_one_typed_bounded_workspace_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let _repository = initialize_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "base\nreview me\n").unwrap();
        let tools = local_vcs_tools(directory.path());

        let review = invoke(tools[4].as_ref(), json!({})).await;
        assert!(!review.is_error, "{}", review.content);
        let result: ReviewWorkflowResult = serde_json::from_str(&review.content).unwrap();
        assert_eq!(result.schema_version, "1.0.0");
        assert_eq!(result.status, ReviewWorkflowStatus::ChangesPresent);
        assert_eq!(result.change_count, 1);
        assert!(!result.truncated);
        assert_eq!(result.status_snapshot["repository"], "workspace");
        assert!(result.diff_snapshot["patch"].as_str().unwrap().contains("review me"));
    }

    #[tokio::test]
    async fn review_status_reports_clean_repository_without_approval_state() {
        let directory = tempfile::tempdir().unwrap();
        let _repository = initialize_repository(directory.path());
        let tools = local_vcs_tools(directory.path());

        let review = invoke(tools[4].as_ref(), json!({})).await;
        assert!(!review.is_error, "{}", review.content);
        let value: Value = serde_json::from_str(&review.content).unwrap();
        assert_eq!(value["status"], "clean");
        assert_eq!(value["change_count"], 0);
        assert!(value.get("approved").is_none());
    }

    #[tokio::test]
    async fn commit_rejects_staged_paths_outside_nested_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_repository(directory.path());
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(directory.path().join("tracked.txt"), "outside\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();

        let tools = local_vcs_tools(&nested);
        let result = invoke(tools[3].as_ref(), json!({"message": "must fail"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("PRESET_RESOURCE_NOT_BOUND"));
        assert_eq!(
            repository.head().unwrap().peel_to_commit().unwrap().message(),
            Some("initial")
        );
    }

    #[tokio::test]
    async fn stage_rejects_parent_traversal_before_git_access() {
        let directory = tempfile::tempdir().unwrap();
        let _repository = initialize_repository(directory.path());
        let tools = local_vcs_tools(directory.path());
        let result = invoke(tools[2].as_ref(), json!({"path": "../outside"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("PRESET_RESOURCE_NOT_BOUND"));
    }
}
