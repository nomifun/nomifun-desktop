//! Production registry for immutable MiniApp Service module paths.
//!
//! The registry is intentionally narrower than a Release Store. The owner or
//! application service registers the already-materialized `service/main.mjs`
//! for one exact `(MiniAppId, release_digest)` pair. The process factory then
//! resolves that same pair and receives a path whose filesystem identity and
//! bytes are revalidated at launch time.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use nomifun_agent_contracts::{DigestHex, MiniAppId, digest_bytes};
use tokio::sync::RwLock;

use crate::runtime::{
    PluginRuntimePlatformError, PluginRuntimePlatformResult, PluginRuntimeServiceLaunch,
    PluginRuntimeServiceModuleResolver,
};

const SERVICE_ENTRYPOINT: &str = "service/main.mjs";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ModuleKey {
    miniapp_id: MiniAppId,
    release_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RegisteredModule {
    path: PathBuf,
    digest: DigestHex,
}

/// Resolves materialized Service modules under one trusted release root.
#[derive(Clone, Debug)]
pub struct PluginRuntimeServiceModuleRegistry {
    root: Arc<PathBuf>,
    modules: Arc<RwLock<BTreeMap<ModuleKey, RegisteredModule>>>,
}

impl PluginRuntimeServiceModuleRegistry {
    /// Creates a registry rooted at an existing, non-symlink directory.
    pub fn new(root: impl Into<PathBuf>) -> PluginRuntimePlatformResult<Self> {
        let root = root.into();
        let canonical_root = canonical_directory(&root, "Plugin Service module registry root")?;
        Ok(Self {
            root: Arc::new(canonical_root),
            modules: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }

    /// Returns the canonical directory against which registered modules are checked.
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    /// Registers one exact release module and returns its canonical path.
    ///
    /// The caller supplies the release identity; the digest is always derived
    /// from the bytes on disk and is never accepted as caller-controlled input.
    pub async fn register(
        &self,
        miniapp_id: MiniAppId,
        release_digest: DigestHex,
        module_path: impl Into<PathBuf>,
    ) -> PluginRuntimePlatformResult<PathBuf> {
        validate_identity(&miniapp_id, &release_digest)?;
        let module = validate_module_path(self.root(), module_path.into())?;
        let registered = RegisteredModule {
            path: module.path.clone(),
            digest: module.digest,
        };
        let key = ModuleKey {
            miniapp_id,
            release_digest,
        };
        self.modules.write().await.insert(key, registered);
        Ok(module.path)
    }

    /// Removes one exact release registration and reports whether it existed.
    pub async fn remove(
        &self,
        miniapp_id: &MiniAppId,
        release_digest: &DigestHex,
    ) -> bool {
        self.modules
            .write()
            .await
            .remove(&ModuleKey {
                miniapp_id: miniapp_id.clone(),
                release_digest: release_digest.clone(),
            })
            .is_some()
    }

    /// Removes all registrations atomically with respect to readers.
    pub async fn clear(&self) {
        self.modules.write().await.clear();
    }

    async fn resolve_exact(
        &self,
        miniapp_id: &MiniAppId,
        release_digest: &DigestHex,
        expected_module_digest: &DigestHex,
    ) -> PluginRuntimePlatformResult<PathBuf> {
        validate_identity(miniapp_id, release_digest)?;
        validate_digest(expected_module_digest, "service_module_digest")?;

        let registered = {
            let modules = self.modules.read().await;
            let key = ModuleKey {
                miniapp_id: miniapp_id.clone(),
                release_digest: release_digest.clone(),
            };
            if let Some(module) = modules.get(&key) {
                Some(Ok(module.clone()))
            } else if modules
                .keys()
                .any(|candidate| candidate.release_digest == *release_digest)
            {
                Some(Err(PluginRuntimePlatformError::InvalidState(
                    "Plugin Service release is registered for another Plugin".into(),
                )))
            } else {
                None
            }
        };

        let registered = match registered {
            Some(Ok(module)) => module,
            Some(Err(error)) => return Err(error),
            None => {
                return Err(PluginRuntimePlatformError::NotFound(format!(
                    "Plugin Service module for {} release {}",
                    miniapp_id.as_ref(),
                    release_digest.as_ref()
                )));
            }
        };

        let observed = validate_module_path(self.root(), registered.path.clone())?;
        if observed.path != registered.path {
            return Err(PluginRuntimePlatformError::InvalidState(
                "registered Plugin Service module path is no longer canonical".into(),
            ));
        }
        if observed.digest != registered.digest {
            return Err(PluginRuntimePlatformError::Runtime(
                "registered Plugin Service module digest no longer matches its path".into(),
            ));
        }
        if observed.digest != *expected_module_digest {
            return Err(PluginRuntimePlatformError::Runtime(
                "Plugin Service module digest does not match the launch spec".into(),
            ));
        }
        Ok(observed.path)
    }
}

#[async_trait]
impl PluginRuntimeServiceModuleResolver for PluginRuntimeServiceModuleRegistry {
    async fn resolve_module(
        &self,
        launch: &PluginRuntimeServiceLaunch,
    ) -> PluginRuntimePlatformResult<PathBuf> {
        launch
            .spec
            .validate()
            .map_err(PluginRuntimePlatformError::Contract)?;
        self.resolve_exact(
            &launch.spec.miniapp_id,
            &launch.spec.release.release_digest,
            &launch.spec.service_module_digest,
        )
        .await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ValidatedModule {
    path: PathBuf,
    digest: DigestHex,
}

fn canonical_directory(path: &Path, label: &str) -> PluginRuntimePlatformResult<PathBuf> {
    if !path.is_absolute() {
        return Err(PluginRuntimePlatformError::InvalidState(format!(
            "{label} must be an absolute path"
        )));
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        PluginRuntimePlatformError::Runtime(format!("{label} cannot be inspected: {error}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PluginRuntimePlatformError::InvalidState(format!(
            "{label} must be a regular non-symlink directory"
        )));
    }
    fs::canonicalize(path).map_err(|error| {
        PluginRuntimePlatformError::Runtime(format!("{label} cannot be canonicalized: {error}"))
    })
}

fn validate_module_path(
    root: &Path,
    path: PathBuf,
) -> PluginRuntimePlatformResult<ValidatedModule> {
    if !path.is_absolute() {
        return Err(PluginRuntimePlatformError::InvalidState(
            "Plugin Service module path must be absolute".into(),
        ));
    }

    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PluginRuntimePlatformError::NotFound(format!(
                "Plugin Service module {}",
                path.display()
            ))
        } else {
            PluginRuntimePlatformError::Runtime(format!(
                "Plugin Service module cannot be inspected: {error}"
            ))
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(PluginRuntimePlatformError::InvalidState(
            "Plugin Service module path must not be a symlink".into(),
        ));
    }
    if !metadata.is_file() {
        return Err(PluginRuntimePlatformError::InvalidState(
            "Plugin Service module path must be a regular file".into(),
        ));
    }

    let canonical = fs::canonicalize(&path).map_err(|error| {
        PluginRuntimePlatformError::Runtime(format!(
            "Plugin Service module cannot be canonicalized: {error}"
        ))
    })?;
    if !canonical.starts_with(root) {
        return Err(PluginRuntimePlatformError::InvalidState(
            "Plugin Service module path escaped the registry root".into(),
        ));
    }
    if canonical.file_name().and_then(|name| name.to_str()) != Some("main.mjs")
        || canonical
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("service")
    {
        return Err(PluginRuntimePlatformError::InvalidState(format!(
            "Plugin Service module path must end with {SERVICE_ENTRYPOINT}"
        )));
    }

    let bytes = fs::read(&canonical).map_err(|error| {
        PluginRuntimePlatformError::Runtime(format!(
            "Plugin Service module cannot be read: {error}"
        ))
    })?;
    Ok(ValidatedModule {
        path: canonical,
        digest: digest_bytes(&bytes),
    })
}

fn validate_identity(
    miniapp_id: &MiniAppId,
    release_digest: &DigestHex,
) -> PluginRuntimePlatformResult<()> {
    if miniapp_id.as_ref().is_empty() || miniapp_id.as_ref().trim() != miniapp_id.as_ref() {
        return Err(PluginRuntimePlatformError::InvalidState(
            "Plugin Service module registry MiniAppId must be non-empty and trimmed".into(),
        ));
    }
    validate_digest(release_digest, "release_digest")
}

fn validate_digest(
    digest: &DigestHex,
    field: &str,
) -> PluginRuntimePlatformResult<()> {
    let value = digest.as_ref();
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PluginRuntimePlatformError::InvalidState(format!(
            "{field} must be 64 lowercase hexadecimal characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nomifun_agent_contracts::{
        ArtifactId, MiniAppReleaseId, MiniAppReleaseRef, MiniAppServiceLifecycle,
        ResolvedMiniAppServiceSpec, ResolvedMiniAppServiceSpecInputs, RuntimeInstallationId,
        RuntimeTarget, VersionString, digest_bytes,
    };
    use tempfile::TempDir;

    use super::*;

    fn digest(seed: &str) -> DigestHex {
        digest_bytes(seed.as_bytes())
    }

    fn module_path(root: &Path) -> PathBuf {
        root.join("release-a").join("service").join("main.mjs")
    }

    fn write_module(root: &Path, source: &str) -> PathBuf {
        let path = module_path(root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        path
    }

    fn launch(
        _node: &Path,
        miniapp_id: &str,
        release_digest: DigestHex,
        module_digest: DigestHex,
    ) -> PluginRuntimeServiceLaunch {
        let miniapp = MiniAppId::from(miniapp_id);
        let spec = ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
            miniapp_id: miniapp.clone(),
            release: MiniAppReleaseRef {
                release_id: MiniAppReleaseId::from("release-id"),
                artifact_id: ArtifactId::from("artifact-id"),
                release_digest,
                manifest_digest: digest("manifest"),
            },
            active_release_epoch: 1,
            service_module_digest: module_digest,
            lifecycle: MiniAppServiceLifecycle::OnDemand,
            host_protocol_version:
                nomifun_agent_contracts::MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version:
                nomifun_agent_contracts::MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
            runtime: nomifun_agent_contracts::MiniAppServiceRuntimeFingerprint {
                runtime_installation_id: RuntimeInstallationId::from("node-installation"),
                runtime_target: RuntimeTarget::from("windows-x86_64"),
                runtime_executable_digest: digest("node"),
                node_version: VersionString::from("22.0.0"),
            },
            config_schema_digest: digest("config-schema"),
            config_snapshot_digest: digest("config-snapshot"),
            credential_slots_digest: digest("credential-slots"),
            resource_contract_digest: digest("resource-contract"),
            resource_bindings_digest: digest("resource-bindings"),
            runtime_requirements_digest: digest("runtime-requirements"),
            bridge_contract_digest: digest("bridge"),
            contribution_set_digest: digest("contributions"),
            storage: nomifun_agent_contracts::MiniAppServiceStorageDescriptor {
                kv: nomifun_agent_contracts::MiniAppKvHandleDescriptor {
                    handle_id: nomifun_agent_contracts::MiniAppKvHandleId::from("kv-handle"),
                    miniapp_id: miniapp.clone(),
                    namespace_revision: 1,
                },
                files_dir: None,
                private_database: None,
            },
        })
        .unwrap();
        PluginRuntimeServiceLaunch {
            spec,
            host_generation: 1,
        }
    }

    #[tokio::test]
    async fn registers_and_resolves_exact_release_pair() {
        let temp = TempDir::new().unwrap();
        let path = write_module(temp.path(), "export async function start() {}");
        let source_digest = digest_bytes(fs::read(&path).unwrap().as_slice());
        let release_digest = digest("release-a");
        let registry = PluginRuntimeServiceModuleRegistry::new(temp.path()).unwrap();

        let registered = registry
            .register(
                MiniAppId::from("miniapp-a"),
                release_digest.clone(),
                path.clone(),
            )
            .await
            .unwrap();
        let resolved = registry
            .resolve_module(&launch(
                Path::new("node"),
                "miniapp-a",
                release_digest,
                source_digest,
            ))
            .await
            .unwrap();

        assert_eq!(registered, path.canonicalize().unwrap());
        assert_eq!(resolved, registered);
    }

    #[tokio::test]
    async fn rejects_missing_wrong_miniapp_and_digest_mismatch() {
        let temp = TempDir::new().unwrap();
        let path = write_module(temp.path(), "module-v1");
        let module_digest = digest_bytes(fs::read(&path).unwrap().as_slice());
        let release_digest = digest("release-a");
        let registry = PluginRuntimeServiceModuleRegistry::new(temp.path()).unwrap();
        registry
            .register(
                MiniAppId::from("miniapp-a"),
                release_digest.clone(),
                path.clone(),
            )
            .await
            .unwrap();

        let missing = registry
            .resolve_module(&launch(
                Path::new("node"),
                "miniapp-a",
                digest("missing-release"),
                module_digest.clone(),
            ))
            .await
            .unwrap_err();
        assert!(matches!(missing, PluginRuntimePlatformError::NotFound(_)));

        let wrong_miniapp = registry
            .resolve_module(&launch(
                Path::new("node"),
                "miniapp-b",
                release_digest.clone(),
                module_digest.clone(),
            ))
            .await
            .unwrap_err();
        assert!(matches!(wrong_miniapp, PluginRuntimePlatformError::InvalidState(_)));

        fs::write(&path, "module-v2").unwrap();
        let changed = registry
            .resolve_module(&launch(
                Path::new("node"),
                "miniapp-a",
                release_digest.clone(),
                module_digest,
            ))
            .await
            .unwrap_err();
        assert!(matches!(changed, PluginRuntimePlatformError::Runtime(_)));

        assert!(registry
            .resolve_module(&launch(
                Path::new("node"),
                "miniapp-a",
                release_digest,
                digest("another-module"),
            ))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn rejects_non_file_and_outside_root_paths() {
        let temp = TempDir::new().unwrap();
        let registry = PluginRuntimeServiceModuleRegistry::new(temp.path()).unwrap();
        let directory = temp.path().join("directory").join("service").join("main.mjs");
        fs::create_dir_all(&directory).unwrap();
        let directory_error = registry
            .register(
                MiniAppId::from("miniapp-a"),
                digest("release-a"),
                directory,
            )
            .await
            .unwrap_err();
        assert!(matches!(directory_error, PluginRuntimePlatformError::InvalidState(_)));

        let outside = TempDir::new().unwrap();
        let outside_path = write_module(outside.path(), "outside");
        let outside_error = registry
            .register(
                MiniAppId::from("miniapp-a"),
                digest("release-b"),
                outside_path,
            )
            .await
            .unwrap_err();
        assert!(matches!(outside_error, PluginRuntimePlatformError::InvalidState(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_module_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let real = write_module(temp.path(), "real");
        let link = temp.path().join("release-b").join("service").join("main.mjs");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&real, &link).unwrap();
        let registry = PluginRuntimeServiceModuleRegistry::new(temp.path()).unwrap();

        let error = registry
            .register(
                MiniAppId::from("miniapp-a"),
                digest("release-b"),
                link,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, PluginRuntimePlatformError::InvalidState(_)));
    }

    #[tokio::test]
    async fn remove_and_clear_are_concurrent_safe() {
        let temp = TempDir::new().unwrap();
        let path = write_module(temp.path(), "module");
        let registry = Arc::new(PluginRuntimeServiceModuleRegistry::new(temp.path()).unwrap());
        let release_digest = digest("release-a");
        registry
            .register(
                MiniAppId::from("miniapp-a"),
                release_digest.clone(),
                path,
            )
            .await
            .unwrap();

        let first = {
            let registry = Arc::clone(&registry);
            let release_digest = release_digest.clone();
            tokio::spawn(async move {
                registry
                    .remove(&MiniAppId::from("miniapp-a"), &release_digest)
                    .await
            })
        };
        let second = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry.clear().await;
            })
        };
        let removed = first.await.unwrap();
        second.await.unwrap();
        assert!(removed || !registry
            .resolve_module(&launch(
                Path::new("node"),
                "miniapp-a",
                release_digest,
                digest("module"),
            ))
            .await
            .is_ok());
    }
}
