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
            _ => Err(unavailable(capability_id)),
        }
    }
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
}
