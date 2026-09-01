//! Application-owned Wave 2 capability host.
//!
//! Only operations with an existing typed, owner-scoped resource API are
//! configured here. Unsupported families fail closed instead of delegating to
//! the legacy Gateway or manufacturing an acknowledgement.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use nomifun_agent_contracts::{StrictJsonValue, TypedResourceBinding};
use nomifun_agent_domain_wave2::{
    Wave2CapabilityOperation, Wave2HostContext, Wave2HostPort, Wave2HostPortError,
    Wave2HostRequest,
};
use nomifun_api_types::{TypedResourceBindingDto, WebSocketMessage};
use nomifun_common::AppError;
use nomifun_file::{
    AgentSessionWorkspaceBinding, FileService, WORKSPACE_RESOURCE_KIND,
    WORKSPACE_ROOT_PARAMETER,
};
use nomifun_realtime::UserEventSink;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone)]
pub(crate) struct Wave2ApplicationHost {
    files: Arc<FileService>,
    configured_workspace_root: PathBuf,
}

impl Wave2ApplicationHost {
    pub(crate) fn new() -> Self {
        Self::for_workspace_root(std::env::temp_dir())
    }

    pub(crate) fn for_workspace_root(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        Self {
            files: Arc::new(FileService::new(
                Arc::new(NullUserEvents),
                vec![workspace_root.clone()],
            )),
            configured_workspace_root: workspace_root,
        }
    }
}

impl Default for Wave2ApplicationHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Wave2HostPort for Wave2ApplicationHost {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let capability_id = request.context.capability_id.as_ref().to_owned();
            match request.operation {
                Wave2CapabilityOperation::WorkspaceExecution { input } => {
                    self.invoke_workspace(&request.context, &capability_id, input)
                        .await
                }
                Wave2CapabilityOperation::Ssh { .. }
                | Wave2CapabilityOperation::McpConnectors { .. }
                | Wave2CapabilityOperation::Browser { .. }
                | Wave2CapabilityOperation::ComputerA11y { .. } => {
                    Err(unavailable(&capability_id))
                }
            }
        })
    }
}

impl Wave2ApplicationHost {
    async fn invoke_workspace(
        &self,
        context: &Wave2HostContext,
        capability_id: &str,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let scope = self.workspace_scope(context)?;
        match capability_id {
            "fs.read" => {
                let params: PathParams = decode(input)?;
                let content = self
                    .files
                    .read_file_for_agent_session(&scope, &params.path)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?
                    .ok_or_else(|| {
                        Wave2HostPortError::new(
                            "RESOURCE_NOT_FOUND",
                            format!("workspace file '{}' was not found", params.path),
                        )
                    })?;
                Ok(StrictJsonValue(json!({
                    "path": params.path,
                    "content": content
                })))
            }
            "fs.write" => {
                let params: WriteParams = decode(input)?;
                let created = self
                    .files
                    .write_file_for_agent_session(
                        &scope,
                        &params.path,
                        params.content.as_bytes(),
                    )
                    .await
                    .map_err(|error| operation_error(capability_id, error))?;
                Ok(StrictJsonValue(json!({
                    "path": params.path,
                    "written": true,
                    "created": created
                })))
            }
            "fs.delete" => {
                let params: PathParams = decode(input)?;
                self.files
                    .remove_entry_for_agent_session(&scope, &params.path)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?;
                Ok(StrictJsonValue(json!({
                    "path": params.path,
                    "deleted": true
                })))
            }
            "fs.search" => {
                let params: SearchParams = decode(input)?;
                let query = params.query.trim();
                if query.is_empty() {
                    return Err(Wave2HostPortError::invalid_payload(
                        "fs.search query must not be empty",
                    ));
                }
                let limit = params.limit.unwrap_or(100);
                if !(1..=200).contains(&limit) {
                    return Err(Wave2HostPortError::invalid_payload(
                        "fs.search limit must be between 1 and 200",
                    ));
                }
                let prefix = params
                    .path
                    .as_deref()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(|path| {
                        scope
                            .resolve_relative_path(path)
                            .and_then(|resolved| {
                                resolved
                                    .strip_prefix(scope.workspace_root())
                                    .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                                    .map_err(|_| {
                                        AppError::BadRequest(
                                            "fs.search path is outside the workspace".to_owned(),
                                        )
                                    })
                            })
                    })
                    .transpose()
                    .map_err(|error| operation_error(capability_id, error))?;
                let files = self
                    .files
                    .list_workspace_files_for_agent_session(&scope)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?;
                let mut matches = Vec::new();
                let mut truncated = false;
                for file in files {
                    let relative_path = file.relative_path.replace('\\', "/");
                    if prefix
                        .as_deref()
                        .is_some_and(|prefix| {
                            !relative_path
                                .strip_prefix(prefix)
                                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                        })
                    {
                        continue;
                    }
                    let Some(content) = self
                        .files
                        .read_file_for_agent_session(&scope, &relative_path)
                        .await
                        .map_err(|error| operation_error(capability_id, error))?
                    else {
                        continue;
                    };
                    for (line_index, line) in content.lines().enumerate() {
                        if !line.contains(query) {
                            continue;
                        }
                        if matches.len() == limit {
                            truncated = true;
                            break;
                        }
                        matches.push(json!({
                            "path": &relative_path,
                            "line": line_index + 1,
                            "text": line
                        }));
                    }
                    if truncated {
                        break;
                    }
                }
                Ok(StrictJsonValue(json!({
                    "query": query,
                    "matches": matches,
                    "truncated": truncated
                })))
            }
            "vcs.status" => self.invoke_vcs_status(&scope, capability_id).await,
            "vcs.diff" => {
                let params: VcsPathParams = decode(input)?;
                self.invoke_vcs_diff(&scope, capability_id, params.path.as_deref())
                    .await
            }
            "vcs.stage" => {
                let params: PathParams = decode(input)?;
                self.invoke_vcs_stage(&scope, capability_id, &params.path)
                    .await
            }
            _ => Err(unavailable(capability_id)),
        }
    }

    async fn invoke_vcs_status(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let workspace = scope.workspace_root().to_path_buf();
        let capability_id = capability_id.to_owned();
        let worker_capability_id = capability_id.clone();
        let status = tokio::task::spawn_blocking(move || {
            let repository = git2::Repository::discover(&workspace).map_err(|error| {
                Wave2HostPortError::new(
                    "RESOURCE_NOT_FOUND",
                    format!("workspace is not a Git repository: {error}"),
                )
            })?;
            let mut options = git2::StatusOptions::new();
            options
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .include_ignored(false);
            let statuses = repository.statuses(Some(&mut options)).map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!(
                        "{worker_capability_id} could not read Git status: {error}"
                    ),
                )
            })?;
            let mut entries = Vec::new();
            for entry in statuses.iter() {
                let Some(path) = entry.path() else {
                    continue;
                };
                entries.push(json!({
                    "path": path.replace('\\', "/"),
                    "status": git_status_name(entry.status())
                }));
            }
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "repository": "workspace",
                "entries": entries
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} status worker failed: {error}"),
            )
        })??;
        Ok(status)
    }

    async fn invoke_vcs_diff(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        path: Option<&str>,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let workspace = scope.workspace_root().to_path_buf();
        let capability_id = capability_id.to_owned();
        let worker_capability_id = capability_id.clone();
        let path = path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| {
                scope
                    .resolve_relative_path(path)
                    .and_then(|resolved| {
                        resolved
                            .strip_prefix(scope.workspace_root())
                            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                            .map_err(|_| {
                                AppError::BadRequest(
                                    "vcs.diff path is outside the workspace".to_owned(),
                                )
                            })
                    })
            })
            .transpose()
            .map_err(|error| operation_error(&capability_id, error))?;
        tokio::task::spawn_blocking(move || {
            let repository = git2::Repository::discover(&workspace).map_err(|error| {
                Wave2HostPortError::new(
                    "RESOURCE_NOT_FOUND",
                    format!("workspace is not a Git repository: {error}"),
                )
            })?;
            let mut options = git2::DiffOptions::new();
            if let Some(path) = &path {
                options.pathspec(path);
            }
            let diff = repository
                .diff_index_to_workdir(None, Some(&mut options))
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "{worker_capability_id} could not read Git diff: {error}"
                        ),
                    )
            })?;
            let mut patch = String::new();
            diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
                if line.origin() != '\0' {
                    patch.push(line.origin());
                }
                patch.push_str(&String::from_utf8_lossy(line.content()));
                true
            })
            .map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!(
                        "{worker_capability_id} could not render Git diff: {error}"
                    ),
                )
            })?;
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "path": path,
                "patch": patch
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} diff worker failed: {error}"),
            )
        })?
    }

    async fn invoke_vcs_stage(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        path: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let relative = scope
            .resolve_relative_path(path)
            .and_then(|resolved| {
                resolved
                    .strip_prefix(scope.workspace_root())
                    .map(|relative| relative.to_path_buf())
                    .map_err(|_| {
                        AppError::BadRequest(
                            "vcs.stage path is outside the workspace".to_owned(),
                        )
                    })
            })
            .map_err(|error| operation_error(capability_id, error))?;
        let workspace = scope.workspace_root().to_path_buf();
        let path_label = path.to_owned();
        tokio::task::spawn_blocking(move || {
            let repository = git2::Repository::discover(&workspace).map_err(|error| {
                Wave2HostPortError::new(
                    "RESOURCE_NOT_FOUND",
                    format!("workspace is not a Git repository: {error}"),
                )
            })?;
            let mut index = repository.index().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("vcs.stage could not open the Git index: {error}"),
                )
            })?;
            index.add_path(&relative).map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("vcs.stage could not stage {}: {error}", path_label),
                )
            })?;
            index.write().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("vcs.stage could not persist the Git index: {error}"),
                )
            })?;
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "path": path_label,
                "staged": true
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} stage worker failed: {error}"),
            )
        })?
    }
}

fn git_status_name(status: git2::Status) -> Vec<&'static str> {
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
        (git2::Status::IGNORED, "ignored"),
    ] {
        if status.contains(flag) {
            names.push(name);
        }
    }
    names
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteParams {
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchParams {
    query: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VcsPathParams {
    #[serde(default)]
    path: Option<String>,
}

fn decode<T: for<'de> Deserialize<'de>>(
    input: StrictJsonValue,
) -> Result<T, Wave2HostPortError> {
    serde_json::from_value(input.0).map_err(|error| {
        Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            format!("Wave 2 filesystem input is invalid: {error}"),
        )
    })
}

impl Wave2ApplicationHost {
    fn workspace_scope(
        &self,
        context: &Wave2HostContext,
    ) -> Result<AgentSessionWorkspaceBinding, Wave2HostPortError> {
        let mut bindings = context
            .resource_bindings
            .iter()
            .filter(|binding| binding.resource_kind.as_ref() == WORKSPACE_RESOURCE_KIND);
        let binding = bindings.next().ok_or_else(|| {
            Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Wave 2 filesystem action requires one workspace resource binding",
            )
        })?;
        if bindings.next().is_some() {
            return Err(Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Wave 2 filesystem action received more than one workspace resource binding",
            ));
        }
        if binding.owner_id != context.principal.principal_id {
            return Err(Wave2HostPortError::new(
                "RESOURCE_OWNER_MISMATCH",
                format!(
                    "workspace binding {} belongs to a different principal",
                    binding.binding_id.as_ref()
                ),
            ));
        }
        let requested_root = binding
            .typed_parameters
            .get(WORKSPACE_ROOT_PARAMETER)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                Wave2HostPortError::new(
                    "PRESET_RESOURCE_NOT_BOUND",
                    format!(
                        "workspace binding {} has no host-resolved {} parameter",
                        binding.binding_id.as_ref(),
                        WORKSPACE_ROOT_PARAMETER
                    ),
                )
            })?;
        let workspace_root = resolve_allowed_workspace_root(
            &self.configured_workspace_root,
            requested_root,
        )?;
        AgentSessionWorkspaceBinding::new(
            context.agent_session_id.as_ref(),
            binding_dto(binding),
            workspace_root,
        )
        .map_err(|error| operation_error(context.capability_id.as_ref(), error))
    }
}

fn resolve_allowed_workspace_root(
    configured_root: &Path,
    requested_root: &str,
) -> Result<PathBuf, Wave2HostPortError> {
    let configured_root = std::fs::canonicalize(configured_root).map_err(|error| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!(
                "configured host workspace root '{}' is unavailable: {error}",
                configured_root.display()
            ),
        )
    })?;
    let requested_root = PathBuf::from(requested_root.trim());
    if !requested_root.is_absolute() {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "workspace_root must be an absolute host-resolved path",
        ));
    }
    let requested_root = std::fs::canonicalize(&requested_root).map_err(|error| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!("workspace_root is unavailable: {error}"),
        )
    })?;
    if !requested_root.starts_with(&configured_root) {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!(
                "workspace_root '{}' is outside the configured host workspace root '{}'",
                requested_root.display(),
                configured_root.display()
            ),
        ));
    }
    Ok(requested_root)
}

fn binding_dto(binding: &TypedResourceBinding) -> TypedResourceBindingDto {
    TypedResourceBindingDto {
        binding_id: binding.binding_id.as_ref().to_owned(),
        resource_kind: binding.resource_kind.as_ref().to_owned(),
        resource_id: binding.resource_id.as_ref().to_owned(),
        owner_id: binding.owner_id.clone(),
        operations: binding.operations.clone(),
        connection_config_ref: binding
            .connection_config_ref
            .as_ref()
            .map(|reference| reference.as_ref().to_owned()),
        typed_parameters: binding.typed_parameters.clone(),
    }
}

fn operation_error(capability_id: &str, error: AppError) -> Wave2HostPortError {
    let code = match error {
        AppError::BadRequest(_) => "INVALID_PAYLOAD",
        AppError::Forbidden(_) => "PRESET_RESOURCE_NOT_BOUND",
        AppError::NotFound(_) => "RESOURCE_NOT_FOUND",
        _ => "CAPABILITY_UNAVAILABLE_ON_PLATFORM",
    };
    Wave2HostPortError::new(code, format!("{capability_id} failed: {error}"))
}

fn unavailable(capability_id: &str) -> Wave2HostPortError {
    Wave2HostPortError::unavailable(format!(
        "no canonical application owner is wired for {capability_id}"
    ))
}

struct NullUserEvents;

impl UserEventSink for NullUserEvents {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<Value>) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, CorrelationId, IdempotencyKey,
        OperationId, PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId,
        ResourceKind,
    };

    fn context(root: &std::path::Path) -> Wave2HostContext {
        Wave2HostContext {
            principal: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "owner-1".to_owned(),
            },
            agent_session_id: AgentSessionId::from(nomifun_common::generate_id()),
            operation_id: OperationId::from("operation-1"),
            idempotency_key: IdempotencyKey::from("idempotency-1"),
            correlation_id: CorrelationId::from("correlation-1"),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot-1".into(),
                snapshot_digest: "a".repeat(64).into(),
            },
            registry_generation: 1,
            capability_id: CapabilityId::from("fs.write"),
            action_id: ActionId::from("fs.write.invoke"),
            resource_bindings: vec![TypedResourceBinding {
                binding_id: ResourceBindingId::from("workspace-binding"),
                resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
                resource_id: ResourceId::from("workspace-resource"),
                owner_id: "owner-1".to_owned(),
                operations: BTreeSet::from([
                    "read".to_owned(),
                    "write".to_owned(),
                    "delete".to_owned(),
                ]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::from([(
                    WORKSPACE_ROOT_PARAMETER.to_owned(),
                    root.to_string_lossy().into_owned(),
                )]),
            }],
        }
    }

    async fn invoke(
        host: &Wave2ApplicationHost,
        mut context: Wave2HostContext,
        capability_id: &str,
        input: Value,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        context.capability_id = CapabilityId::from(capability_id.to_owned());
        context.action_id = ActionId::from(format!("{capability_id}.invoke"));
        host.invoke(Wave2HostRequest {
            context,
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: StrictJsonValue(input),
            },
        })
        .await
    }

    #[tokio::test]
    async fn workspace_file_actions_use_the_typed_binding_root() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());

        let written = invoke(
            &host,
            context.clone(),
            "fs.write",
            json!({"path": "test.txt", "content": "hello"}),
        )
        .await
        .unwrap();
        assert_eq!(written.0["written"], true);

        let read = invoke(
            &host,
            context.clone(),
            "fs.read",
            json!({"path": "test.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(read.0["content"], "hello");

        let deleted = invoke(
            &host,
            context,
            "fs.delete",
            json!({"path": "test.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(deleted.0["deleted"], true);
        assert!(!directory.path().join("test.txt").exists());
    }

    #[tokio::test]
    async fn missing_workspace_root_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let mut context = context(directory.path());
        context.resource_bindings[0].typed_parameters.clear();
        let error = invoke(&host, context, "fs.read", json!({"path": "x"}))
            .await
            .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
    }

    #[tokio::test]
    async fn workspace_search_returns_real_content_matches() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("needle.txt"), "before\nneedle line\nafter\n")
            .unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let result = invoke(
            &host,
            context(directory.path()),
            "fs.search",
            json!({"query": "needle"}),
        )
        .await
        .unwrap();
        assert_eq!(result.0["matches"][0]["path"], "needle.txt");
        assert_eq!(result.0["matches"][0]["line"], 2);
        assert_eq!(result.0["truncated"], false);
    }

    fn initialize_git_repository(root: &Path) -> git2::Repository {
        let repository = git2::Repository::init(root).unwrap();
        std::fs::write(root.join("tracked.txt"), "base\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repository.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("NomiFun test", "test@nomifun.invalid").unwrap();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "initial",
                &tree,
                &[],
            )
            .unwrap();
        drop(tree);
        repository
    }

    #[tokio::test]
    async fn vcs_status_diff_and_stage_use_the_bound_repository() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "base\nchanged\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let base_context = context(directory.path());

        let status = invoke(&host, base_context.clone(), "vcs.status", json!({}))
            .await
            .unwrap();
        assert_eq!(status.0["entries"][0]["path"], "tracked.txt");
        assert!(
            status.0["entries"][0]["status"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == "worktree_modified"))
        );

        let diff = invoke(&host, base_context.clone(), "vcs.diff", json!({}))
            .await
            .unwrap();
        assert!(diff.0["patch"].as_str().unwrap().contains("changed"));

        let staged = invoke(
            &host,
            base_context,
            "vcs.stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(staged.0["staged"], true);
        let status_after = repository.statuses(None).unwrap();
        assert!(
            status_after
                .iter()
                .any(|entry| entry.status().contains(git2::Status::INDEX_MODIFIED))
        );
    }
}
