//! AgentSession-facing filesystem resource binding.
//!
//! The host resolves a logical workspace resource to a native path before
//! entering this crate.  This adapter validates the immutable binding facts and
//! then resolves native paths with the OS user access controls.  It does not
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

    /// Resolve a native path against the Session's default directory. The
    /// workspace is project identity and a default, never an access boundary.
    /// Leave `..` and links for native lookup so link/parent semantics are not
    /// changed by lexical normalization.
    pub fn resolve_path(&self, path: &str) -> Result<PathBuf, AppError> {
        if path.contains('\0') {
            return Err(AppError::BadRequest("file path contains a NUL byte".into()));
        }
        let path = Path::new(path);
        #[cfg(windows)]
        if path.has_root() && !path.is_absolute()
            || matches!(path.components().next(), Some(std::path::Component::Prefix(_))) && !path.is_absolute()
        {
            return Err(AppError::BadRequest("use an absolute drive/UNC path or a path relative to the Session default directory".into()));
        }
        let resolved = if path.is_absolute() { path.to_owned() } else { self.workspace_root.join(path) };
        #[cfg(windows)]
        { nomi_native_files::windows::normalize_path(&resolved).map_err(|error| crate::path_safety::native_io_error("resolving native Windows file path", error)) }
        #[cfg(not(windows))]
        { Ok(resolved) }
    }

    /// Project APIs (VCS, inventories and artifact ownership) retain a real
    /// project-relative identity independent of global native file access.
    pub fn resolve_project_path(&self, path: &str) -> Result<PathBuf, AppError> {
        let relative = crate::artifact_store::normalized_workspace_relative(path, true)?;
        let path = self.workspace_root.join(relative);
        crate::path_safety::validate_workspace_candidate(&path, &self.workspace_root)?;
        Ok(path)
    }

    pub fn authority(&self) -> PathAuthority {
        PathAuthority::Unrestricted
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
    fn validates_binding_and_accepts_native_session_relative_paths() {
        let mut wrong_kind = binding();
        wrong_kind.resource_kind = "terminal".into();
        assert!(AgentSessionWorkspaceBinding::new(valid_session_id(), wrong_kind, "C:\\work").is_err());

        let root = tempdir().unwrap().keep();
        let scope = AgentSessionWorkspaceBinding::new(valid_session_id(), binding(), &root)
            .expect("valid workspace binding");
        for path in ["../outside", r"nested\outside", ".nomifun/artifacts/receipt", "./.nomifun/artifacts/receipt", "nested//file.txt", "./nested/file.txt"] {
            #[cfg(not(windows))]
            assert_eq!(scope.resolve_path(path).unwrap(), root.join(path));
            #[cfg(windows)]
            assert_eq!(scope.resolve_path(path).unwrap(), nomi_native_files::windows::normalize_path(&root.join(path)).unwrap());
        }
        assert!(scope.resolve_path("bad\0path").is_err());
        let absolute = root.parent().unwrap().join("outside");
        assert_eq!(scope.resolve_path(absolute.to_str().unwrap()).unwrap(), absolute);
        assert!(matches!(scope.authority(), PathAuthority::Unrestricted));
        assert!(scope.resolve_project_path("../outside").is_err());
        assert!(scope.resolve_project_path(".nomifun/artifacts/receipt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn native_file_paths_accept_symlink_aliases() {
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".nomifun/artifacts")).unwrap();
        std::fs::write(root.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        std::os::unix::fs::symlink(".nomifun", root.path().join("alias")).unwrap();
        let scope = AgentSessionWorkspaceBinding::new(valid_session_id(), binding(), root.path())
            .unwrap();
        assert!(scope.resolve_path("alias/artifacts/receipt").is_ok());
        assert!(scope.resolve_path("alias/artifacts/new").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn native_file_paths_accept_junction_aliases() {
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".nomifun/artifacts")).unwrap();
        std::fs::write(root.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        junction::create(root.path().join(".nomifun"), root.path().join("alias")).unwrap();
        let scope = AgentSessionWorkspaceBinding::new(valid_session_id(), binding(), root.path())
            .unwrap();
        assert!(scope.resolve_path("alias/artifacts/receipt").is_ok());
        assert!(scope.resolve_path("alias/artifacts/new").is_ok());
    }
}
