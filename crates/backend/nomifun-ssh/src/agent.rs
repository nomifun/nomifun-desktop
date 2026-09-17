//! Product-facing SSH Module and its exact resource boundary.
//!
//! Connection establishment is deliberately absent from the authoring surface.
//! Credential lookup, host-key checks, pooling, reconnection, and teardown are
//! resource-owner concerns. An Agent receives one `ssh_host` binding plus an
//! explicit subset of the four actions below; both must authorize an operation
//! before this owner asks the pool for a backend.

use std::collections::BTreeSet;

use nomifun_ai_agent::{RemoteCommandOutput, SshBackend};
use nomifun_common::{ConversationId, SshHostId, UserId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::sink::SshActionDispatchError;
use crate::{SshConnectionPool, SshLinkKey, SshLinkPhase};

pub const SSH_MODULE_ID: &str = "ssh";
pub const SSH_FS_READ_ACTION_ID: &str = "ssh/fs.read";
pub const SSH_FS_WRITE_ACTION_ID: &str = "ssh/fs.write";
pub const SSH_EXEC_ACTION_ID: &str = "ssh/exec";
pub const SSH_SUDO_ACTION_ID: &str = "ssh/sudo";
pub const SSH_ACTION_IDS: [&str; 4] = [
    SSH_FS_READ_ACTION_ID,
    SSH_FS_WRITE_ACTION_ID,
    SSH_EXEC_ACTION_ID,
    SSH_SUDO_ACTION_ID,
];

pub const SSH_HOST_RESOURCE_KIND: &str = "ssh_host";
pub const SSH_HOST_READ_OPERATION: &str = "read";
pub const SSH_HOST_WRITE_OPERATION: &str = "write";
pub const SSH_HOST_EXECUTE_OPERATION: &str = "execute";
pub const SSH_HOST_SUDO_OPERATION: &str = "sudo";
pub const SSH_HOST_RESOURCE_OPERATIONS: [&str; 4] = [
    SSH_HOST_READ_OPERATION,
    SSH_HOST_WRITE_OPERATION,
    SSH_HOST_EXECUTE_OPERATION,
    SSH_HOST_SUDO_OPERATION,
];

pub const DEFAULT_SSH_ACTION_TIMEOUT_MS: u64 = 120_000;
pub const MAX_SSH_ACTION_TIMEOUT_MS: u64 = 600_000;
pub const MAX_SSH_ACTION_READ_BYTES: u64 = 1024 * 1024;
pub const MAX_SSH_ACTION_WRITE_BYTES: usize = 1024 * 1024;
const MAX_REMOTE_PATH_CHARS: usize = 4_096;
const MAX_PATTERN_CHARS: usize = 16_384;
const MAX_COMMAND_CHARS: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SshAction {
    FsRead,
    FsWrite,
    Exec,
    Sudo,
}

impl SshAction {
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::FsRead => SSH_FS_READ_ACTION_ID,
            Self::FsWrite => SSH_FS_WRITE_ACTION_ID,
            Self::Exec => SSH_EXEC_ACTION_ID,
            Self::Sudo => SSH_SUDO_ACTION_ID,
        }
    }

    pub const fn required_resource_operation(self) -> SshResourceOperation {
        match self {
            Self::FsRead => SshResourceOperation::Read,
            Self::FsWrite => SshResourceOperation::Write,
            Self::Exec => SshResourceOperation::Execute,
            Self::Sudo => SshResourceOperation::Sudo,
        }
    }

    pub fn from_action_id(action_id: &str) -> Option<Self> {
        match action_id {
            SSH_FS_READ_ACTION_ID => Some(Self::FsRead),
            SSH_FS_WRITE_ACTION_ID => Some(Self::FsWrite),
            SSH_EXEC_ACTION_ID => Some(Self::Exec),
            SSH_SUDO_ACTION_ID => Some(Self::Sudo),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SshResourceOperation {
    Read,
    Write,
    Execute,
    Sudo,
}

impl SshResourceOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => SSH_HOST_READ_OPERATION,
            Self::Write => SSH_HOST_WRITE_OPERATION,
            Self::Execute => SSH_HOST_EXECUTE_OPERATION,
            Self::Sudo => SSH_HOST_SUDO_OPERATION,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            SSH_HOST_READ_OPERATION => Some(Self::Read),
            SSH_HOST_WRITE_OPERATION => Some(Self::Write),
            SSH_HOST_EXECUTE_OPERATION => Some(Self::Execute),
            SSH_HOST_SUDO_OPERATION => Some(Self::Sudo),
            _ => None,
        }
    }
}

/// One exact SSH host selected in a resolved AgentSession snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSshHostResource {
    binding_id: String,
    owner_id: String,
    ssh_host_id: SshHostId,
    remote_cwd: String,
    operations: BTreeSet<SshResourceOperation>,
}

impl AgentSshHostResource {
    pub fn from_operation_names<I, S>(
        binding_id: impl Into<String>,
        owner_id: impl Into<String>,
        ssh_host_id: SshHostId,
        remote_cwd: impl Into<String>,
        operations: I,
    ) -> Result<Self, SshActionError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let operations = operations
            .into_iter()
            .map(|operation| {
                SshResourceOperation::parse(operation.as_ref()).ok_or_else(|| {
                    SshActionError::InvalidResource(format!(
                        "ssh_host resource contains unsupported operation {}",
                        operation.as_ref()
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(binding_id, owner_id, ssh_host_id, remote_cwd, operations)
    }

    pub fn new(
        binding_id: impl Into<String>,
        owner_id: impl Into<String>,
        ssh_host_id: SshHostId,
        remote_cwd: impl Into<String>,
        operations: impl IntoIterator<Item = SshResourceOperation>,
    ) -> Result<Self, SshActionError> {
        let binding_id = binding_id.into();
        let owner_id = owner_id.into();
        let remote_cwd = remote_cwd.into();
        if binding_id.trim().is_empty() || owner_id.trim().is_empty() {
            return Err(SshActionError::InvalidResource(
                "ssh_host binding and owner identities must not be blank".into(),
            ));
        }
        UserId::try_from(owner_id.as_str()).map_err(|error| {
            SshActionError::InvalidResource(format!("ssh_host owner is invalid: {error}"))
        })?;
        validate_remote_path("remote_cwd", &remote_cwd)?;
        let operations = operations.into_iter().collect::<BTreeSet<_>>();
        if operations.is_empty() {
            return Err(SshActionError::InvalidResource(
                "ssh_host resource must grant at least one operation".into(),
            ));
        }
        Ok(Self {
            binding_id,
            owner_id,
            ssh_host_id,
            remote_cwd,
            operations,
        })
    }

    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub fn ssh_host_id(&self) -> &SshHostId {
        &self.ssh_host_id
    }

    pub fn remote_cwd(&self) -> &str {
        &self.remote_cwd
    }

    pub fn operation_names(&self) -> BTreeSet<&'static str> {
        self.operations
            .iter()
            .map(|operation| operation.as_str())
            .collect()
    }

    fn authorize(&self, principal_id: &str, action: SshAction) -> Result<(), SshActionError> {
        if self.owner_id != principal_id {
            return Err(SshActionError::ResourceOwnerMismatch);
        }
        let required = action.required_resource_operation();
        if !self.operations.contains(&required) {
            return Err(SshActionError::ResourceOperationDenied {
                action_id: action.action_id(),
                operation: required.as_str(),
            });
        }
        Ok(())
    }
}

/// Frozen SSH authority. An unbound Module remains visible to the product, but
/// no action can dial or execute until a host resource is explicitly selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSshAuthority {
    principal_id: String,
    resource: Option<AgentSshHostResource>,
}

impl AgentSshAuthority {
    pub fn unbound(principal_id: impl Into<String>) -> Result<Self, SshActionError> {
        Self::new(principal_id, None)
    }

    pub fn bound(
        principal_id: impl Into<String>,
        resource: AgentSshHostResource,
    ) -> Result<Self, SshActionError> {
        Self::new(principal_id, Some(resource))
    }

    fn new(
        principal_id: impl Into<String>,
        resource: Option<AgentSshHostResource>,
    ) -> Result<Self, SshActionError> {
        let principal_id = principal_id.into();
        UserId::try_from(principal_id.as_str()).map_err(|error| {
            SshActionError::InvalidContext(format!("SSH principal identity is invalid: {error}"))
        })?;
        if resource
            .as_ref()
            .is_some_and(|resource| resource.owner_id != principal_id)
        {
            return Err(SshActionError::ResourceOwnerMismatch);
        }
        Ok(Self {
            principal_id,
            resource,
        })
    }

    pub fn principal_id(&self) -> &str {
        &self.principal_id
    }

    fn require(&self, action: SshAction) -> Result<&AgentSshHostResource, SshActionError> {
        let resource = self.resource.as_ref().ok_or(SshActionError::HostUnbound)?;
        resource.authorize(&self.principal_id, action)?;
        Ok(resource)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshActionContext {
    pub principal_id: String,
    pub agent_session_id: String,
    pub operation_id: String,
}

impl SshActionContext {
    fn validate(&self, authority: &AgentSshAuthority) -> Result<(), SshActionError> {
        if self.principal_id != authority.principal_id {
            return Err(SshActionError::ResourceOwnerMismatch);
        }
        ConversationId::try_from(self.agent_session_id.as_str()).map_err(|error| {
            SshActionError::InvalidContext(format!("invalid AgentSession identity: {error}"))
        })?;
        if self.operation_id.is_empty()
            || self.operation_id.len() > 256
            || !self
                .operation_id
                .bytes()
                .all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(SshActionError::InvalidContext(
                "operation identity must contain 1 to 256 visible ASCII bytes".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum SshFsReadInput {
    Read { path: String },
    Grep { pattern: String, path: String },
    List { glob: String },
    Stat { path: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SshFsWriteInput {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SshExecInput {
    pub command: String,
    #[serde(default = "default_action_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SshSudoInput {
    /// Command body only. Supplying `sudo`, `doas`, or `su` is rejected; the
    /// owner constructs the privileged wrapper after authorizing `ssh/sudo`.
    pub command: String,
    #[serde(default = "default_action_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SshExternalActionStatus {
    Succeeded,
    ExitedNonZero,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SshFsReadOutput {
    Text { content: String },
    Paths { paths: Vec<String> },
    Stat { size: u64, is_dir: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SshFsWriteOutput {
    pub status: SshExternalActionStatus,
    pub bytes_written: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SshCommandOutput {
    pub status: SshExternalActionStatus,
    pub stdout: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SshResourceSelection {
    Unbound,
    Bound {
        binding_id: String,
        ssh_host_id: String,
        remote_cwd: String,
        operations: Vec<String>,
        connection_status: SshConnectionStatus,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SshConnectionStatus {
    Idle,
    Connecting,
    Connected,
    Degraded,
    Reconnecting,
    Dropped,
    Closed,
}

impl From<SshLinkPhase> for SshConnectionStatus {
    fn from(value: SshLinkPhase) -> Self {
        match value {
            SshLinkPhase::Idle => Self::Idle,
            SshLinkPhase::Connecting => Self::Connecting,
            SshLinkPhase::Connected => Self::Connected,
            SshLinkPhase::Degraded => Self::Degraded,
            SshLinkPhase::Reconnecting => Self::Reconnecting,
            SshLinkPhase::Dropped => Self::Dropped,
            SshLinkPhase::Closed => Self::Closed,
        }
    }
}

#[derive(Debug, Error)]
pub enum SshActionError {
    #[error("ssh Module has no bound ssh_host resource")]
    HostUnbound,
    #[error("ssh_host resource belongs to a different principal")]
    ResourceOwnerMismatch,
    #[error("{action_id} requires ssh_host operation {operation}")]
    ResourceOperationDenied {
        action_id: &'static str,
        operation: &'static str,
    },
    #[error("invalid ssh_host resource: {0}")]
    InvalidResource(String),
    #[error("invalid SSH action context: {0}")]
    InvalidContext(String),
    #[error("invalid SSH action input: {0}")]
    InvalidInput(String),
    #[error("SSH external action failed: {0}")]
    External(String),
    #[error("SSH external action outcome is unknown: {0}")]
    OutcomeUnknown(String),
}

#[derive(Clone)]
pub struct SshActionOwner {
    pool: SshConnectionPool,
}

impl SshActionOwner {
    pub fn new(pool: SshConnectionPool) -> Self {
        Self { pool }
    }

    pub fn resource_selection(
        &self,
        authority: &AgentSshAuthority,
        agent_session_id: &str,
    ) -> Result<SshResourceSelection, SshActionError> {
        ConversationId::try_from(agent_session_id).map_err(|error| {
            SshActionError::InvalidContext(format!("invalid AgentSession identity: {error}"))
        })?;
        let Some(resource) = authority.resource.as_ref() else {
            return Ok(SshResourceSelection::Unbound);
        };
        if resource.owner_id != authority.principal_id {
            return Err(SshActionError::ResourceOwnerMismatch);
        }
        let key = SshLinkKey::new(agent_session_id, resource.ssh_host_id.clone());
        let connection_status = self
            .pool
            .subscribe(&key)
            .map(|receiver| receiver.borrow().phase().into())
            .unwrap_or(SshConnectionStatus::Idle);
        Ok(SshResourceSelection::Bound {
            binding_id: resource.binding_id.clone(),
            ssh_host_id: resource.ssh_host_id.as_str().to_owned(),
            remote_cwd: resource.remote_cwd.clone(),
            operations: resource
                .operation_names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            connection_status,
        })
    }

    pub async fn fs_read(
        &self,
        authority: &AgentSshAuthority,
        context: &SshActionContext,
        input: SshFsReadInput,
    ) -> Result<SshFsReadOutput, SshActionError> {
        let backend = self
            .backend(authority, context, SshAction::FsRead)
            .await?;
        match input {
            SshFsReadInput::Read { path } => {
                validate_remote_path("path", &path)?;
                let stat = backend.stat(&path).await.map_err(external_error)?;
                if stat.is_dir {
                    return Err(SshActionError::InvalidInput(format!(
                        "{path} is a directory, not a file"
                    )));
                }
                if stat.size > MAX_SSH_ACTION_READ_BYTES {
                    return Err(SshActionError::InvalidInput(format!(
                        "remote file exceeds the {MAX_SSH_ACTION_READ_BYTES}-byte read limit"
                    )));
                }
                let bytes = backend.read_file(&path).await.map_err(external_error)?;
                Ok(SshFsReadOutput::Text {
                    content: String::from_utf8_lossy(&bytes).into_owned(),
                })
            }
            SshFsReadInput::Grep { pattern, path } => {
                validate_bounded_text("pattern", &pattern, MAX_PATTERN_CHARS)?;
                validate_remote_path("path", &path)?;
                backend
                    .grep(&pattern, &path)
                    .await
                    .map(|content| SshFsReadOutput::Text { content })
                    .map_err(external_error)
            }
            SshFsReadInput::List { glob } => {
                validate_bounded_text("glob", &glob, MAX_REMOTE_PATH_CHARS)?;
                backend
                    .list_files(&glob)
                    .await
                    .map(|paths| SshFsReadOutput::Paths { paths })
                    .map_err(external_error)
            }
            SshFsReadInput::Stat { path } => {
                validate_remote_path("path", &path)?;
                backend
                    .stat(&path)
                    .await
                    .map(|stat| SshFsReadOutput::Stat {
                        size: stat.size,
                        is_dir: stat.is_dir,
                    })
                    .map_err(external_error)
            }
        }
    }

    pub async fn fs_write(
        &self,
        authority: &AgentSshAuthority,
        context: &SshActionContext,
        input: SshFsWriteInput,
    ) -> Result<SshFsWriteOutput, SshActionError> {
        validate_remote_path("path", &input.path)?;
        if input.content.len() > MAX_SSH_ACTION_WRITE_BYTES {
            return Err(SshActionError::InvalidInput(format!(
                "content exceeds the {MAX_SSH_ACTION_WRITE_BYTES}-byte write limit"
            )));
        }
        let backend = self
            .backend(authority, context, SshAction::FsWrite)
            .await?;
        let bytes_written = input.content.len();
        backend
            .write_file(&input.path, input.content.into_bytes())
            .await
            .map_err(outcome_unknown_error)?;
        Ok(SshFsWriteOutput {
            status: SshExternalActionStatus::Succeeded,
            bytes_written,
        })
    }

    pub async fn exec(
        &self,
        authority: &AgentSshAuthority,
        context: &SshActionContext,
        input: SshExecInput,
    ) -> Result<SshCommandOutput, SshActionError> {
        validate_command(&input.command, input.timeout_ms)?;
        let link = self.link(authority, context, SshAction::Exec).await?;
        self.pool
            .run_unprivileged_action(
                authority.principal_id(),
                &link,
                &input.command,
                input.timeout_ms,
            )
            .await
            .map(command_output)
            .map_err(dispatch_error)
    }

    pub async fn sudo(
        &self,
        authority: &AgentSshAuthority,
        context: &SshActionContext,
        input: SshSudoInput,
    ) -> Result<SshCommandOutput, SshActionError> {
        validate_command(&input.command, input.timeout_ms)?;
        if contains_privileged_launcher(&input.command) {
            return Err(SshActionError::InvalidInput(
                "ssh/sudo command must not contain a second privilege launcher".into(),
            ));
        }
        let link = self.link(authority, context, SshAction::Sudo).await?;
        self.pool
            .run_sudo_action(
                authority.principal_id(),
                &link,
                &input.command,
                input.timeout_ms,
            )
            .await
            .map(command_output)
            .map_err(dispatch_error)
    }

    async fn backend(
        &self,
        authority: &AgentSshAuthority,
        context: &SshActionContext,
        action: SshAction,
    ) -> Result<std::sync::Arc<dyn SshBackend>, SshActionError> {
        let link = self.link(authority, context, action).await?;
        Ok(self.pool.backend_for(&link))
    }

    async fn link(
        &self,
        authority: &AgentSshAuthority,
        context: &SshActionContext,
        action: SshAction,
    ) -> Result<std::sync::Arc<crate::SshLink>, SshActionError> {
        context.validate(authority)?;
        let resource = authority.require(action)?;
        self
            .pool
            .acquire(
                authority.principal_id(),
                &context.agent_session_id,
                &resource.ssh_host_id,
                &resource.remote_cwd,
            )
            .await
            .map_err(external_error)
    }
}

const fn default_action_timeout_ms() -> u64 {
    DEFAULT_SSH_ACTION_TIMEOUT_MS
}

fn validate_command(command: &str, timeout_ms: u64) -> Result<(), SshActionError> {
    validate_bounded_text("command", command, MAX_COMMAND_CHARS)?;
    if timeout_ms == 0 || timeout_ms > MAX_SSH_ACTION_TIMEOUT_MS {
        return Err(SshActionError::InvalidInput(format!(
            "timeout_ms must be between 1 and {MAX_SSH_ACTION_TIMEOUT_MS}"
        )));
    }
    Ok(())
}

fn validate_remote_path(label: &str, path: &str) -> Result<(), SshActionError> {
    validate_bounded_text(label, path, MAX_REMOTE_PATH_CHARS)?;
    if path.contains('\0') || path.contains('\r') || path.contains('\n') {
        return Err(SshActionError::InvalidInput(format!(
            "{label} must not contain control delimiters"
        )));
    }
    Ok(())
}

fn validate_bounded_text(label: &str, value: &str, max: usize) -> Result<(), SshActionError> {
    if value.trim().is_empty() || value.chars().count() > max {
        return Err(SshActionError::InvalidInput(format!(
            "{label} must contain 1 to {max} characters"
        )));
    }
    Ok(())
}

fn contains_privileged_launcher(command: &str) -> bool {
    command
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    ';' | '|' | '&' | '(' | ')' | '<' | '>' | '\'' | '"' | '`'
                )
        })
        .filter(|token| !token.is_empty())
        .filter_map(|token| token.rsplit('/').next())
        .any(|program| matches!(program, "sudo" | "doas" | "su"))
}

fn command_output(output: RemoteCommandOutput) -> SshCommandOutput {
    let status = if output.timed_out {
        SshExternalActionStatus::TimedOut
    } else if output.exit_code == 0 {
        SshExternalActionStatus::Succeeded
    } else {
        SshExternalActionStatus::ExitedNonZero
    };
    SshCommandOutput {
        status,
        stdout: output.stdout,
        exit_code: output.exit_code,
        timed_out: output.timed_out,
    }
}

fn external_error(error: impl std::fmt::Display) -> SshActionError {
    SshActionError::External(error.to_string())
}

fn outcome_unknown_error(error: impl std::fmt::Display) -> SshActionError {
    SshActionError::OutcomeUnknown(error.to_string())
}

fn dispatch_error(error: SshActionDispatchError) -> SshActionError {
    match error {
        SshActionDispatchError::Rejected(message) => SshActionError::External(message),
        SshActionDispatchError::OutcomeUnknown(message) => {
            SshActionError::OutcomeUnknown(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    fn resource(
        owner: &str,
        operations: impl IntoIterator<Item = SshResourceOperation>,
    ) -> AgentSshHostResource {
        AgentSshHostResource::new(
            "binding-a",
            owner,
            SshHostId::new(),
            "/srv/project",
            operations,
        )
        .unwrap()
    }

    #[test]
    fn authoring_surface_has_exact_actions_and_no_connect_capability() {
        assert_eq!(
            SSH_ACTION_IDS,
            ["ssh/fs.read", "ssh/fs.write", "ssh/exec", "ssh/sudo"]
        );
        assert!(SshAction::from_action_id("ssh/connect").is_none());
    }

    #[test]
    fn bound_unbound_and_resource_operations_fail_closed() {
        let unbound = AgentSshAuthority::unbound(OWNER).unwrap();
        assert!(matches!(
            unbound.require(SshAction::FsRead),
            Err(SshActionError::HostUnbound)
        ));

        let authority = AgentSshAuthority::bound(
            OWNER,
            resource(OWNER, [SshResourceOperation::Read, SshResourceOperation::Execute]),
        )
        .unwrap();
        assert!(authority.require(SshAction::FsRead).is_ok());
        assert!(authority.require(SshAction::Exec).is_ok());
        assert!(matches!(
            authority.require(SshAction::FsWrite),
            Err(SshActionError::ResourceOperationDenied { .. })
        ));
        assert!(matches!(
            authority.require(SshAction::Sudo),
            Err(SshActionError::ResourceOperationDenied { .. })
        ));
    }

    #[test]
    fn resource_owner_and_operation_names_are_exact() {
        let other = "0190f5fe-7c00-7a00-8000-000000000002";
        assert!(AgentSshAuthority::bound(OWNER, resource(other, [SshResourceOperation::Read])).is_err());
        assert!(AgentSshHostResource::from_operation_names(
            "binding-a",
            OWNER,
            SshHostId::new(),
            "/tmp",
            ["read", "connect"],
        )
        .is_err());
    }

    #[test]
    fn action_inputs_reject_resource_selection_and_connection_fields() {
        assert!(serde_json::from_value::<SshExecInput>(json!({
            "command": "pwd",
            "ssh_host_id": "forbidden"
        })).is_err());
        assert!(serde_json::from_value::<SshFsWriteInput>(json!({
            "path": "/tmp/a",
            "content": "x",
            "credential": "forbidden"
        })).is_err());
        assert!(serde_json::from_value::<SshFsReadInput>(json!({
            "operation": "connect",
            "host": "forbidden"
        })).is_err());
    }

    #[test]
    fn sudo_rejects_an_obvious_nested_privilege_launcher() {
        for command in ["sudo id", "/usr/bin/sudo id", "sh -c 'doas id'", "su - root"] {
            assert!(contains_privileged_launcher(command), "{command}");
        }
        assert!(!contains_privileged_launcher("printf 'ordinary command'"));
    }

    #[test]
    fn external_command_status_is_explicit() {
        assert_eq!(
            command_output(RemoteCommandOutput {
                stdout: String::new(),
                exit_code: 0,
                timed_out: false,
            })
            .status,
            SshExternalActionStatus::Succeeded
        );
        assert_eq!(
            command_output(RemoteCommandOutput {
                stdout: String::new(),
                exit_code: 1,
                timed_out: false,
            })
            .status,
            SshExternalActionStatus::ExitedNonZero
        );
        assert_eq!(
            command_output(RemoteCommandOutput {
                stdout: String::new(),
                exit_code: 137,
                timed_out: true,
            })
            .status,
            SshExternalActionStatus::TimedOut
        );
    }
}
