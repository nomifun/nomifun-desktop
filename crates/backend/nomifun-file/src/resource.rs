//! AgentSession-facing filesystem resource binding.
//!
//! The host resolves a logical workspace resource to a native path before
//! entering this crate.  This adapter validates the immutable binding facts and
//! then delegates to the existing path-authority implementation.  It does not
//! persist a second session mapping or infer ownership from a path.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nomifun_api_types::TypedResourceBindingDto;
use nomifun_common::AppError;

use crate::path_safety::PathAuthority;

pub const WORKSPACE_RESOURCE_KIND: &str = "workspace";
/// Host-resolved absolute path carried by a typed workspace binding.
///
/// `resource_id` remains an opaque product identity. The native path is
/// explicit, owner-scoped input and is never inferred from that identity.
pub const WORKSPACE_ROOT_PARAMETER: &str = "workspace_root";
pub const READ_OPERATION: &str = "read";
pub const WRITE_OPERATION: &str = "write";
pub const DELETE_OPERATION: &str = "delete";

/// Validated binding context for one AgentSession's workspace resource.
///
/// `workspace_root` is already resolved by the host.  The logical
/// `binding.resource_id` remains intact for audit and is deliberately not
/// treated as a filesystem path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionWorkspaceBinding {
    agent_session_id: String,
    binding: TypedResourceBindingDto,
    workspace_root: PathBuf,
}

impl AgentSessionWorkspaceBinding {
    pub fn new(
        agent_session_id: impl Into<String>,
        binding: TypedResourceBindingDto,
        workspace_root: impl Into<PathBuf>,
    ) -> Result<Self, AppError> {
        let agent_session_id = agent_session_id.into();
        nomifun_common::validate_uuidv7(&agent_session_id).map_err(|error| {
            AppError::BadRequest(format!(
                "invalid AgentSession id '{agent_session_id}': {error}"
            ))
        })?;

        if binding.resource_kind != WORKSPACE_RESOURCE_KIND {
            return Err(AppError::BadRequest(format!(
                "workspace resource binding has kind '{}', expected '{}'",
                binding.resource_kind, WORKSPACE_RESOURCE_KIND
            )));
        }
        if binding.resource_id.trim().is_empty() {
            return Err(AppError::BadRequest(
                "workspace resource binding must identify a resource".to_owned(),
            ));
        }
        if binding.owner_id.trim().is_empty() {
            return Err(AppError::BadRequest(
                "workspace resource binding must identify an owner".to_owned(),
            ));
        }

        let workspace_root = workspace_root.into();
        if workspace_root.as_os_str().is_empty() || !workspace_root.is_absolute() {
            return Err(AppError::BadRequest(
                "resolved workspace root must be an absolute native path".to_owned(),
            ));
        }

        Ok(Self {
            agent_session_id,
            binding,
            workspace_root,
        })
    }

    pub fn agent_session_id(&self) -> &str {
        &self.agent_session_id
    }

    pub fn binding(&self) -> &TypedResourceBindingDto {
        &self.binding
    }

    pub fn owner_id(&self) -> &str {
        &self.binding.owner_id
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn allows(&self, operation: &str) -> bool {
        self.binding.operations.contains(operation)
    }

    pub fn require_operation(&self, operation: &str) -> Result<(), AppError> {
        if self.allows(operation) {
            Ok(())
        } else {
            Err(AppError::Forbidden(format!(
                "AgentSession '{}' workspace binding '{}' does not allow operation '{}'",
                self.agent_session_id, self.binding.binding_id, operation
            )))
        }
    }

    /// Resolve a workspace-relative path without allowing an absolute path,
    /// parent component, NUL byte, or portable separator escape.
    pub fn resolve_relative_path(&self, relative_path: &str) -> Result<PathBuf, AppError> {
        let relative = crate::artifact_store::normalized_workspace_relative(relative_path, true)?;
        let resolved = if relative.as_os_str().is_empty() {
            self.workspace_root.clone()
        } else {
            self.workspace_root.join(relative)
        };
        crate::path_safety::validate_workspace_candidate(&resolved, &self.workspace_root)?;
        Ok(resolved)
    }

    pub fn authority(&self) -> PathAuthority {
        PathAuthority::Workspace(self.workspace_root.clone())
    }
}

/// Build a binding for a host-resolved workspace resource.
pub fn workspace_binding(
    agent_session_id: impl Into<String>,
    binding_id: impl Into<String>,
    resource_id: impl Into<String>,
    owner_id: impl Into<String>,
    operations: impl IntoIterator<Item = impl Into<String>>,
    workspace_root: impl Into<PathBuf>,
) -> Result<AgentSessionWorkspaceBinding, AppError> {
    AgentSessionWorkspaceBinding::new(
        agent_session_id,
        TypedResourceBindingDto {
            binding_id: binding_id.into(),
            resource_kind: WORKSPACE_RESOURCE_KIND.to_owned(),
            resource_id: resource_id.into(),
            owner_id: owner_id.into(),
            operations: operations.into_iter().map(Into::into).collect::<BTreeSet<_>>(),
            connection_config_ref: None,
            typed_parameters: Default::default(),
        },
        workspace_root,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn valid_session_id() -> String {
        nomifun_common::generate_id()
    }

    fn binding() -> TypedResourceBindingDto {
        TypedResourceBindingDto {
            binding_id: "workspace-binding".into(),
            resource_kind: WORKSPACE_RESOURCE_KIND.into(),
            resource_id: "workspace-resource".into(),
            owner_id: "owner-1".into(),
            operations: BTreeSet::from([READ_OPERATION.to_owned(), WRITE_OPERATION.to_owned()]),
            connection_config_ref: None,
            typed_parameters: Default::default(),
        }
    }

    #[test]
    fn validates_agent_session_and_host_resolved_root() {
        let root = tempdir().unwrap().keep();
        let scope = AgentSessionWorkspaceBinding::new(valid_session_id(), binding(), &root)
            .expect("valid workspace binding");
        assert_eq!(scope.workspace_root(), root);
        assert!(scope.allows(READ_OPERATION));
        assert!(!scope.allows(DELETE_OPERATION));
    }

    #[test]
    fn rejects_wrong_kind_relative_escape_and_non_absolute_root() {
        let mut wrong_kind = binding();
        wrong_kind.resource_kind = "terminal".into();
        assert!(AgentSessionWorkspaceBinding::new(valid_session_id(), wrong_kind, "C:\\work").is_err());

        let root = tempdir().unwrap().keep();
        let scope = AgentSessionWorkspaceBinding::new(valid_session_id(), binding(), &root)
            .expect("valid workspace binding");
        assert!(scope.resolve_relative_path("../outside").is_err());
        assert!(scope.resolve_relative_path("/outside").is_err());
        assert!(scope.resolve_relative_path(r"nested\outside").is_err());
        assert!(scope.resolve_relative_path(".nomifun/artifacts/receipt").is_err());
        assert!(scope.resolve_relative_path("./.nomifun/artifacts/receipt").is_err());
        assert!(scope.resolve_relative_path("nested//file.txt").is_err());
        assert!(scope.resolve_relative_path("./nested/file.txt").is_err());
        #[cfg(windows)]
        assert!(scope.resolve_relative_path(".NOMIFUN/artifacts/receipt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_alias_into_workspace_owner_namespace() {
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".nomifun/artifacts")).unwrap();
        std::fs::write(root.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        std::os::unix::fs::symlink(".nomifun", root.path().join("alias")).unwrap();
        let scope = AgentSessionWorkspaceBinding::new(valid_session_id(), binding(), root.path())
            .unwrap();
        assert!(scope.resolve_relative_path("alias/artifacts/receipt").is_err());
        assert!(scope.resolve_relative_path("alias/artifacts/new").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn rejects_junction_alias_into_workspace_owner_namespace() {
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".nomifun/artifacts")).unwrap();
        std::fs::write(root.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        junction::create(root.path().join(".nomifun"), root.path().join("alias")).unwrap();
        let scope = AgentSessionWorkspaceBinding::new(valid_session_id(), binding(), root.path())
            .unwrap();
        assert!(scope.resolve_relative_path("alias/artifacts/receipt").is_err());
        assert!(scope.resolve_relative_path("alias/artifacts/new").is_err());
    }
}
