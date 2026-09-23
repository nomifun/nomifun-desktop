use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    PluginActionPublication, PluginArtifact, PluginArtifactRef, PluginId,
    PluginMigrationManifest, PluginMutationId, StrictJsonValue,
};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::{
    ArtifactImportResult, DataGeneration, ImportCancellation, InstallCommit, NeverCancel,
    PluginArtifactStore, PluginArtifactStoreError, PluginDataRootError, PluginDataRootHandle,
    PluginDataRootManager, PluginInventory, PluginMutationKind, PluginMutationPhase,
    PluginMutationRecord, PluginRecord, PluginRepository, PluginRepositoryError,
    ImportedPluginBackup,
    StoredArtifactRecord,
};

#[derive(Debug, Error)]
pub enum PluginInstallError {
    #[error(transparent)]
    Artifact(#[from] PluginArtifactStoreError),
    #[error(transparent)]
    DataRoot(#[from] PluginDataRootError),
    #[error(transparent)]
    Repository(#[from] PluginRepositoryError),
    #[error("Plugin permissions require confirmation: {0:?}")]
    PermissionConfirmationRequired(BTreeSet<String>),
    #[error("Plugin Credential slots require confirmation: {0:?}")]
    SecretConfirmationRequired(BTreeSet<String>),
    #[error("Plugin local Service code requires confirmation")]
    LocalServiceConfirmationRequired,
    #[error("Plugin Config does not match configSchema: {0}")]
    InvalidConfig(String),
    #[error("Plugin update is invalid: {0}")]
    InvalidUpdate(String),
    #[error("Plugin staging validation failed: {0}")]
    Validation(String),
    #[error("Plugin activation failed: {0}")]
    Activation(String),
    #[error("Plugin mutation failed and recovery also failed: mutation={mutation}; recovery={recovery}")]
    Recovery { mutation: String, recovery: String },
}

pub type PluginInstallResult<T> = Result<T, PluginInstallError>;

#[derive(Clone, Debug)]
pub enum InstallTarget {
    New { plugin_id: PluginId },
    Existing {
        plugin_id: PluginId,
        expected_revision: u64,
    },
}

impl InstallTarget {
    pub fn new() -> Self {
        Self::New {
            plugin_id: PluginId::from(Uuid::now_v7().to_string()),
        }
    }

    pub fn plugin_id(&self) -> &PluginId {
        match self {
            Self::New { plugin_id } | Self::Existing { plugin_id, .. } => plugin_id,
        }
    }

    pub fn expected_revision(&self) -> Option<u64> {
        match self {
            Self::New { .. } => None,
            Self::Existing {
                expected_revision, ..
            } => Some(*expected_revision),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InstallArtifactRequest {
    pub owner_user_id: String,
    pub target: InstallTarget,
    /// Local package identity. Omit for the author's manifest id; explicit
    /// copies receive a Host-generated identity without rewriting the Artifact.
    pub local_package_id: Option<String>,
    pub config: Value,
    pub credential_bindings: BTreeMap<String, String>,
    /// Exact permissions the user confirmed for this Artifact. Existing grants
    /// are reused without prompting and need not be repeated here.
    pub confirmed_permissions: BTreeSet<String>,
    pub confirmed_secret_slots: BTreeSet<String>,
    pub trusted_local_service_confirmed: bool,
}

#[derive(Clone, Debug)]
pub struct InstallArtifactOutcome {
    pub plugin: PluginRecord,
    pub artifact: PluginArtifact,
    pub actions: Vec<PluginActionPublication>,
}

#[derive(Clone, Debug)]
pub struct PluginRuntimeContext {
    pub owner_user_id: String,
    pub plugin: PluginRecord,
    pub artifact: PluginArtifact,
    pub data_root: PluginDataRootHandle,
    pub credential_bindings: BTreeMap<String, String>,
    pub granted_permissions: BTreeSet<String>,
}

#[async_trait]
pub trait PluginRuntimePort: Send + Sync {
    /// Execute the exact JS migration chain using the same storage adapter as
    /// normal Plugin execution.
    async fn migrate(
        &self,
        artifact: &PluginArtifact,
        data_root: &PluginDataRootHandle,
        migrations: &[PluginMigrationManifest],
    ) -> Result<(), String>;

    /// Import/activate health check plus static UI load check. Business Actions
    /// must not be invoked by validation.
    async fn validate(&self, context: &PluginRuntimeContext) -> Result<(), String>;

    /// Revoke Service and Surface admission and drain/cancel bounded in-flight
    /// work before the pointer transaction commits.
    async fn quiesce(&self, plugin_id: &PluginId) -> Result<(), String>;

    async fn activate(&self, context: PluginRuntimeContext) -> Result<(), String>;

    async fn remove(&self, plugin_id: &PluginId) -> Result<(), String>;
}

#[async_trait]
pub trait PluginBindingPort: Send + Sync {
    async fn validate(
        &self,
        _owner_user_id: &str,
        _plugin: &PluginRecord,
        _actions: &[PluginActionPublication],
    ) -> Result<(), String> {
        Ok(())
    }

    async fn replace(
        &self,
        owner_user_id: &str,
        plugin: &PluginRecord,
        actions: &[PluginActionPublication],
    ) -> Result<(), String>;

    async fn remove(&self, owner_user_id: &str, plugin_id: &PluginId) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct NoopPluginRuntime;

#[async_trait]
impl PluginRuntimePort for NoopPluginRuntime {
    async fn migrate(
        &self,
        _artifact: &PluginArtifact,
        _data_root: &PluginDataRootHandle,
        migrations: &[PluginMigrationManifest],
    ) -> Result<(), String> {
        if migrations.is_empty() {
            Ok(())
        } else {
            Err("JS migration runtime is not configured".into())
        }
    }

    async fn validate(&self, context: &PluginRuntimeContext) -> Result<(), String> {
        if context.artifact.manifest.has_service() {
            Err("Plugin Service runtime is not configured".into())
        } else {
            Ok(())
        }
    }

    async fn quiesce(&self, _plugin_id: &PluginId) -> Result<(), String> {
        Ok(())
    }

    async fn activate(&self, _context: PluginRuntimeContext) -> Result<(), String> {
        Ok(())
    }

    async fn remove(&self, _plugin_id: &PluginId) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct NoopPluginBindings;

#[async_trait]
impl PluginBindingPort for NoopPluginBindings {
    async fn replace(
        &self,
        _owner_user_id: &str,
        _plugin: &PluginRecord,
        _actions: &[PluginActionPublication],
    ) -> Result<(), String> {
        Ok(())
    }

    async fn remove(&self, _owner_user_id: &str, _plugin_id: &PluginId) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Default)]
struct MutationCoordinator {
    locks: Mutex<HashMap<PluginId, Weak<AsyncMutex<()>>>>,
}

impl MutationCoordinator {
    async fn acquire(&self, plugin_id: &PluginId) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self
                .locks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            locks.retain(|_, lock| lock.strong_count() > 0);
            locks
                .get(plugin_id)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| {
                    let lock = Arc::new(AsyncMutex::new(()));
                    locks.insert(plugin_id.clone(), Arc::downgrade(&lock));
                    lock
                })
        };
        lock.lock_owned().await
    }
}

pub struct PluginInstallService {
    repository: Arc<dyn PluginRepository>,
    artifacts: Arc<PluginArtifactStore>,
    data_roots: Arc<PluginDataRootManager>,
    runtime: Arc<dyn PluginRuntimePort>,
    bindings: Arc<dyn PluginBindingPort>,
    mutations: MutationCoordinator,
}

struct BackupDataSeed {
    data_sqlite: Vec<u8>,
    files: BTreeMap<String, Vec<u8>>,
    grants: BTreeMap<String, bool>,
}

impl std::fmt::Debug for PluginInstallService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginInstallService")
            .field("artifact_root", &self.artifacts.managed_root())
            .field("data_root", &self.data_roots.root())
            .finish_non_exhaustive()
    }
}

impl PluginInstallService {
    pub fn new(
        repository: Arc<dyn PluginRepository>,
        artifacts: Arc<PluginArtifactStore>,
        data_roots: Arc<PluginDataRootManager>,
        runtime: Arc<dyn PluginRuntimePort>,
        bindings: Arc<dyn PluginBindingPort>,
    ) -> Self {
        Self {
            repository,
            artifacts,
            data_roots,
            runtime,
            bindings,
            mutations: MutationCoordinator::default(),
        }
    }

    pub async fn install_directory(
        &self,
        request: InstallArtifactRequest,
        source: impl AsRef<Path>,
        cancellation: Option<&dyn ImportCancellation>,
    ) -> PluginInstallResult<InstallArtifactOutcome> {
        let imported = self
            .artifacts
            .import_directory(source, cancellation.unwrap_or(&NeverCancel))?;
        self.install_artifact(request, imported).await
    }

    pub async fn install_zip(
        &self,
        request: InstallArtifactRequest,
        source: impl AsRef<Path>,
        cancellation: Option<&dyn ImportCancellation>,
    ) -> PluginInstallResult<InstallArtifactOutcome> {
        let imported = self
            .artifacts
            .import_zip(source, cancellation.unwrap_or(&NeverCancel))?;
        self.install_artifact(request, imported).await
    }

    /// Chat Draft save uses this exact path after freezing its managed working
    /// directory into normalized package bytes.
    pub async fn install_files(
        &self,
        request: InstallArtifactRequest,
        files: &BTreeMap<String, Vec<u8>>,
        cancellation: Option<&dyn ImportCancellation>,
    ) -> PluginInstallResult<InstallArtifactOutcome> {
        let imported = self
            .artifacts
            .import_files(files, cancellation.unwrap_or(&NeverCancel))?;
        self.install_artifact(request, imported).await
    }

    /// Restore a Backup as a new local Plugin while still entering the same
    /// Artifact validation, mutation journal, activation, and Binding path as
    /// Directory, ZIP, and Chat Draft installs. Credential bindings are never
    /// carried by the Backup; callers bind them explicitly after import.
    pub async fn install_backup(
        &self,
        mut request: InstallArtifactRequest,
        backup: ImportedPluginBackup,
        cancellation: Option<&dyn ImportCancellation>,
    ) -> PluginInstallResult<InstallArtifactOutcome> {
        if !matches!(request.target, InstallTarget::New { .. }) {
            return Err(PluginInstallError::InvalidUpdate(
                "a Plugin Backup must be restored as a new local Plugin".into(),
            ));
        }
        let restored_grants = backup
            .grants
            .iter()
            .map(|grant| (grant.permission.clone(), grant.granted))
            .collect::<BTreeMap<_, _>>();
        if restored_grants.len() != backup.grants.len()
            || restored_grants.keys().cloned().collect::<BTreeSet<_>>()
                != backup.package.artifact.manifest.permissions
        {
            return Err(PluginInstallError::InvalidUpdate(
                "Backup Grants must exactly match the Artifact permissions".into(),
            ));
        }
        let imported = self.artifacts.import_files(
            &backup.package.files,
            cancellation.unwrap_or(&NeverCancel),
        )?;
        if imported.stored.artifact.artifact_digest != backup.artifact_digest
            || imported.stored.artifact != backup.package.artifact
        {
            return Err(PluginInstallError::InvalidUpdate(
                "Backup package does not match its Artifact".into(),
            ));
        }
        request.config = backup.config;
        self.install_artifact_seeded(
            request,
            imported,
            Some(BackupDataSeed {
                data_sqlite: backup.data_sqlite,
                files: backup.files,
                grants: restored_grants,
            }),
        )
        .await
    }

    pub async fn install_artifact(
        &self,
        request: InstallArtifactRequest,
        imported: ArtifactImportResult,
    ) -> PluginInstallResult<InstallArtifactOutcome> {
        self.install_artifact_seeded(request, imported, None).await
    }

    async fn install_artifact_seeded(
        &self,
        request: InstallArtifactRequest,
        imported: ArtifactImportResult,
        backup: Option<BackupDataSeed>,
    ) -> PluginInstallResult<InstallArtifactOutcome> {
        let plugin_id = request.target.plugin_id().clone();
        let _guard = self.mutations.acquire(&plugin_id).await;
        let current = self
            .repository
            .inventory(&request.owner_user_id, &plugin_id)
            .await?;
        validate_target(&request, current.as_ref())?;
        if let Some(current) = current.as_ref()
            && current.artifact.artifact.manifest.id != imported.stored.artifact.manifest.id
        {
            return Err(PluginInstallError::InvalidUpdate(
                "an update must retain the Artifact package id".into(),
            ));
        }
        validate_config(&imported.stored.artifact, &request.config)?;
        if request
            .credential_bindings
            .keys()
            .any(|slot| !imported.stored.artifact.manifest.secrets.contains(slot))
        {
            return Err(PluginInstallError::InvalidUpdate(
                "Credential slot is not declared by the staged Artifact".into(),
            ));
        }
        if let Some(current) = current.as_ref()
            && current.plugin.active_artifact_digest
                == imported.stored.artifact.artifact_digest
        {
            if current.plugin.config != request.config
                || current.credential_bindings != request.credential_bindings
            {
                return Err(PluginInstallError::InvalidUpdate(
                    "use the Config action to change setup for an unchanged Plugin Artifact"
                        .into(),
                ));
            }
            return Ok(InstallArtifactOutcome {
                plugin: current.plugin.clone(),
                artifact: imported.stored.artifact.clone(),
                actions: action_publications(&current.plugin, &imported.stored.artifact),
            });
        }
        validate_non_permission_confirmations(
            &imported.stored.artifact,
            current.as_ref(),
            &request.confirmed_secret_slots,
            request.trusted_local_service_confirmed,
        )?;
        let mut grants = resolved_grants(
            &imported.stored.artifact,
            current.as_ref(),
            &request.confirmed_permissions,
        )?;
        if let Some(backup) = backup.as_ref() {
            grants = backup.grants.clone();
        }

        let old_artifact = current
            .as_ref()
            .map(|inventory| inventory.plugin.active_artifact_digest.clone());
        let old_generation = current
            .as_ref()
            .map(|inventory| inventory.plugin.data_generation.clone());
        let old_data_version = current
            .as_ref()
            .map(|inventory| inventory.artifact.artifact.manifest.data_version)
            .unwrap_or(0);
        let new_data_version = imported.stored.artifact.manifest.data_version;
        if new_data_version < old_data_version {
            return Err(PluginInstallError::InvalidUpdate(format!(
                "dataVersion cannot decrease from {old_data_version} to {new_data_version}; use restore"
            )));
        }

        let changes_data = current.is_none() || old_data_version != new_data_version;
        let committed_generation = if changes_data {
            Uuid::now_v7().to_string()
        } else {
            old_generation
                .clone()
                .expect("an existing Plugin has a data generation")
        };
        let mutation_id = PluginMutationId::from(Uuid::now_v7().to_string());
        let now_ms = positive_now_ms();
        let mutation = PluginMutationRecord {
            mutation_id: mutation_id.clone(),
            owner_user_id: request.owner_user_id.clone(),
            plugin_id: plugin_id.clone(),
            kind: if current.is_some() {
                PluginMutationKind::Update
            } else {
                PluginMutationKind::Install
            },
            phase: PluginMutationPhase::Staging,
            old_artifact_digest: old_artifact.clone(),
            new_artifact_digest: Some(imported.stored.artifact.artifact_digest.clone()),
            old_data_generation: old_generation.clone(),
            new_data_generation: Some(committed_generation.clone()),
            expected_revision: request.target.expected_revision(),
            error: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        self.repository.begin_mutation(&mutation).await?;

        let result = self
            .prepare_and_commit(
                &request,
                current.as_ref(),
                imported,
                grants,
                mutation,
                old_data_version,
                new_data_version,
                committed_generation,
                backup,
            )
            .await;
        if let Err(error) = &result {
            let _ = self
                .repository
                .update_mutation_phase(
                    &mutation_id,
                    PluginMutationPhase::Failed,
                    Some(&error.to_string()),
                    positive_now_ms(),
                )
                .await;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_and_commit(
        &self,
        request: &InstallArtifactRequest,
        current: Option<&PluginInventory>,
        imported: ArtifactImportResult,
        grants: BTreeMap<String, bool>,
        mutation: PluginMutationRecord,
        old_data_version: u32,
        new_data_version: u32,
        committed_generation: String,
        backup: Option<BackupDataSeed>,
    ) -> PluginInstallResult<InstallArtifactOutcome> {
        let plugin_id = request.target.plugin_id().clone();
        let changes_data = current.is_none() || old_data_version != new_data_version;
        let staging_generation = DataGeneration::new(if changes_data {
            committed_generation.clone()
        } else {
            Uuid::now_v7().to_string()
        })?;
        let staged = match (current, backup.as_ref()) {
            (None, Some(backup)) => self.data_roots.stage_import(
                plugin_id.clone(),
                staging_generation,
                &backup.data_sqlite,
                &backup.files,
            )?,
            (Some(_), Some(_)) => {
                return Err(PluginInstallError::InvalidUpdate(
                    "a Plugin Backup cannot overwrite an installed Plugin".into(),
                ));
            }
            (Some(inventory), None) => {
                let source = self.data_roots.open_generation(
                    plugin_id.clone(),
                    DataGeneration::new(inventory.plugin.data_generation.clone())?,
                )?;
                self.data_roots.stage_clone(&source, staging_generation)?
            }
            (None, None) => self
                .data_roots
                .stage_empty(plugin_id.clone(), staging_generation)?,
        };

        let migrations = if backup.is_some() {
            Vec::new()
        } else {
            match migration_chain(
                &imported.stored.artifact,
                old_data_version,
                new_data_version,
            ) {
                Ok(migrations) => migrations,
                Err(error) => {
                    self.data_roots.discard_staging(staged)?;
                    self.repository
                        .finish_mutation(&mutation.mutation_id)
                        .await?;
                    return Err(error);
                }
            }
        };
        if let Err(error) = self
            .runtime
            .migrate(&imported.stored.artifact, &staged, &migrations)
            .await
        {
            let _ = self.data_roots.discard_staging(staged);
            return Err(PluginInstallError::Validation(format!(
                "migration failed: {error}"
            )));
        }
        let candidate_plugin = staged_plugin_record(
            request,
            current,
            &imported.stored.artifact,
            &committed_generation,
        )?;
        let candidate_actions = action_publications(&candidate_plugin, &imported.stored.artifact);
        if let Err(error) = self
            .bindings
            .validate(&request.owner_user_id, &candidate_plugin, &candidate_actions)
            .await
        {
            let _ = self.data_roots.discard_staging(staged);
            return Err(PluginInstallError::Validation(format!(
                "Binding validation failed: {error}"
            )));
        }
        let mut validation_plugin = candidate_plugin.clone();
        validation_plugin.data_generation = staged.generation().as_str().to_owned();
        let validation_context = PluginRuntimeContext {
            owner_user_id: request.owner_user_id.clone(),
            plugin: validation_plugin,
            artifact: imported.stored.artifact.clone(),
            data_root: staged.clone(),
            credential_bindings: request.credential_bindings.clone(),
            granted_permissions: grants
                .iter()
                .filter_map(|(permission, granted)| granted.then_some(permission.clone()))
                .collect(),
        };
        if let Err(error) = self.runtime.validate(&validation_context).await {
            let _ = self.data_roots.discard_staging(staged);
            return Err(PluginInstallError::Validation(error));
        }

        let published_generation = if changes_data {
            Some(self.data_roots.publish(staged)?)
        } else {
            self.data_roots.discard_staging(staged)?;
            None
        };
        self.repository
            .update_mutation_phase(
                &mutation.mutation_id,
                PluginMutationPhase::Prepared,
                None,
                positive_now_ms(),
            )
            .await?;

        if let Err(error) = self.runtime.quiesce(&plugin_id).await {
            if let Some(generation) = published_generation {
                let _ = self
                    .data_roots
                    .delete_generation_exact(&plugin_id, generation.generation());
            }
            return Err(PluginInstallError::Activation(format!(
                "could not quiesce the previous runtime: {error}"
            )));
        }
        self.bindings
            .remove(&request.owner_user_id, &plugin_id)
            .await
            .map_err(PluginInstallError::Activation)?;

        let stored_artifact = StoredArtifactRecord {
            artifact: imported.stored.artifact.clone(),
            artifact_root: imported.stored.artifact_root.to_string_lossy().into_owned(),
            created_at_ms: positive_now_ms(),
        };
        let commit = InstallCommit {
            mutation_id: mutation.mutation_id.clone(),
            owner_user_id: request.owner_user_id.clone(),
            plugin_id: plugin_id.clone(),
            package_id: request
                .local_package_id
                .clone()
                .unwrap_or_else(|| imported.stored.artifact.manifest.id.clone()),
            expected_revision: request.target.expected_revision(),
            artifact: stored_artifact,
            data_generation: committed_generation,
            config: request.config.clone(),
            credential_bindings: request.credential_bindings.clone(),
            grants,
            now_ms: positive_now_ms(),
        };
        let plugin = match self.repository.commit_install(&commit).await {
            Ok(plugin) => plugin,
            Err(error) => {
                if let Some(generation) = published_generation {
                    let _ = self
                        .data_roots
                        .delete_generation_exact(&plugin_id, generation.generation());
                }
                return Err(error.into());
            }
        };
        let inventory = self
            .repository
            .inventory(&request.owner_user_id, &plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        let actions = action_publications(&plugin, &imported.stored.artifact);
        let activation = self.admit_inventory(&inventory).await;
        if let Err(error) = activation {
            let rollback = self
                .repository
                .rollback_install(
                    &mutation.mutation_id,
                    &request.owner_user_id,
                    &plugin_id,
                    positive_now_ms(),
                )
                .await;
            match rollback {
                Ok(previous) => {
                    if let Some(generation) = published_generation.as_ref() {
                        if generation.generation().as_ref() != previous.data_generation {
                            let _ = self.data_roots.delete_generation_exact(
                                &plugin_id,
                                generation.generation(),
                            );
                        }
                    }
                    return match self
                        .restore_runtime(&request.owner_user_id, &previous)
                        .await
                    {
                        Ok(()) => Err(PluginInstallError::Activation(error.to_string())),
                        Err(recovery) => Err(PluginInstallError::Recovery {
                            mutation: error.to_string(),
                            recovery: recovery.to_string(),
                        }),
                    };
                }
                Err(PluginRepositoryError::NotFound) if current.is_none() => {
                    if let Some(generation) = published_generation.as_ref() {
                        let _ = self.data_roots.delete_generation_exact(
                            &plugin_id,
                            generation.generation(),
                        );
                    }
                    let _ = self.runtime.remove(&plugin_id).await;
                    let _ = self
                        .bindings
                        .remove(&request.owner_user_id, &plugin_id)
                        .await;
                    return Err(PluginInstallError::Activation(error.to_string()));
                }
                Err(recovery) => {
                    return Err(PluginInstallError::Recovery {
                        mutation: error.to_string(),
                        recovery: recovery.to_string(),
                    });
                }
            }
        }

        self.prune_retired_generations(&plugin)?;
        self.repository
            .finish_mutation(&mutation.mutation_id)
            .await?;
        Ok(InstallArtifactOutcome {
            plugin,
            artifact: imported.stored.artifact,
            actions,
        })
    }

    async fn restore_runtime(
        &self,
        owner_user_id: &str,
        plugin: &PluginRecord,
    ) -> PluginInstallResult<()> {
        let inventory = self
            .repository
            .inventory(owner_user_id, &plugin.plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        self.admit_inventory(&inventory).await
    }

    async fn admit_inventory(&self, inventory: &PluginInventory) -> PluginInstallResult<()> {
        let plugin = &inventory.plugin;
        if plugin.trashed_at_ms.is_some() {
            self.runtime
                .remove(&plugin.plugin_id)
                .await
                .map_err(PluginInstallError::Activation)?;
            self.bindings
                .remove(&plugin.owner_user_id, &plugin.plugin_id)
                .await
                .map_err(PluginInstallError::Activation)?;
            return Ok(());
        }
        if plugin.enabled {
            let root = self.data_roots.open_generation(
                plugin.plugin_id.clone(),
                DataGeneration::new(plugin.data_generation.clone())?,
            )?;
            if let Err(error) = self
                .runtime
                .activate(runtime_context(inventory.clone(), root))
                .await
            {
                return Err(PluginInstallError::Activation(error));
            }
        } else {
            self.runtime
                .remove(&plugin.plugin_id)
                .await
                .map_err(PluginInstallError::Activation)?;
        }
        let actions = action_publications(plugin, &inventory.artifact.artifact);
        if let Err(error) = self
            .bindings
            .replace(&plugin.owner_user_id, plugin, &actions)
            .await
        {
            let _ = self.runtime.remove(&plugin.plugin_id).await;
            return Err(PluginInstallError::Activation(error));
        }
        Ok(())
    }

    async fn revoke_admission(&self, inventory: &PluginInventory) -> PluginInstallResult<()> {
        let plugin = &inventory.plugin;
        if let Err(error) = self.runtime.quiesce(&plugin.plugin_id).await {
            let recovery = self.admit_inventory(inventory).await;
            return match recovery {
                Ok(()) => Err(PluginInstallError::Activation(error)),
                Err(recovery) => Err(PluginInstallError::Recovery {
                    mutation: error,
                    recovery: recovery.to_string(),
                }),
            };
        }
        if let Err(error) = self
            .bindings
            .remove(&plugin.owner_user_id, &plugin.plugin_id)
            .await
        {
            let recovery = self.admit_inventory(inventory).await;
            return match recovery {
                Ok(()) => Err(PluginInstallError::Activation(error)),
                Err(recovery) => Err(PluginInstallError::Recovery {
                    mutation: error,
                    recovery: recovery.to_string(),
                }),
            };
        }
        Ok(())
    }

    async fn inventory_at_revision(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
    ) -> PluginInstallResult<PluginInventory> {
        let inventory = self
            .repository
            .inventory(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        if inventory.plugin.revision != expected_revision {
            return Err(PluginRepositoryError::Conflict.into());
        }
        Ok(inventory)
    }

    pub async fn set_enabled(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        enabled: bool,
    ) -> PluginInstallResult<PluginRecord> {
        let _guard = self.mutations.acquire(plugin_id).await;
        let previous = self
            .inventory_at_revision(owner_user_id, plugin_id, expected_revision)
            .await?;
        if previous.plugin.trashed_at_ms.is_some() {
            return Err(PluginInstallError::InvalidUpdate(
                "a trashed Plugin cannot be enabled or disabled".into(),
            ));
        }
        self.revoke_admission(&previous).await?;
        let committed = match self
            .repository
            .set_enabled(
                owner_user_id,
                plugin_id,
                expected_revision,
                enabled,
                positive_now_ms(),
            )
            .await
        {
            Ok(plugin) => plugin,
            Err(error) => {
                self.admit_inventory(&previous).await?;
                return Err(error.into());
            }
        };
        let current = self
            .repository
            .inventory(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        if let Err(error) = self.admit_inventory(&current).await {
            let rollback = self
                .repository
                .set_enabled(
                    owner_user_id,
                    plugin_id,
                    committed.revision,
                    previous.plugin.enabled,
                    positive_now_ms(),
                )
                .await;
            return self
                .finish_lifecycle_rollback(owner_user_id, error, rollback)
                .await;
        }
        Ok(committed)
    }

    pub async fn configure(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        config: Value,
        credential_bindings: BTreeMap<String, String>,
        grants: BTreeMap<String, bool>,
    ) -> PluginInstallResult<PluginRecord> {
        let _guard = self.mutations.acquire(plugin_id).await;
        let previous = self
            .inventory_at_revision(owner_user_id, plugin_id, expected_revision)
            .await?;
        if previous.plugin.trashed_at_ms.is_some() {
            return Err(PluginInstallError::InvalidUpdate(
                "a trashed Plugin cannot be configured".into(),
            ));
        }
        validate_config(&previous.artifact.artifact, &config)?;
        if credential_bindings
            .keys()
            .any(|slot| !previous.artifact.artifact.manifest.secrets.contains(slot))
        {
            return Err(PluginInstallError::InvalidUpdate(
                "Credential slot is not declared by the active Artifact".into(),
            ));
        }
        if grants
            .keys()
            .any(|permission| !previous.artifact.artifact.manifest.permissions.contains(permission))
        {
            return Err(PluginInstallError::InvalidUpdate(
                "permission Grant is not declared by the active Artifact".into(),
            ));
        }
        self.revoke_admission(&previous).await?;
        let committed = match self
            .repository
            .set_config(
                owner_user_id,
                plugin_id,
                expected_revision,
                &config,
                &credential_bindings,
                &grants,
                positive_now_ms(),
            )
            .await
        {
            Ok(plugin) => plugin,
            Err(error) => {
                self.admit_inventory(&previous).await?;
                return Err(error.into());
            }
        };
        let current = self
            .repository
            .inventory(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        if let Err(error) = self.admit_inventory(&current).await {
            let previous_grants = previous
                .grants
                .iter()
                .map(|(permission, grant)| (permission.clone(), grant.granted))
                .collect::<BTreeMap<_, _>>();
            let rollback = self
                .repository
                .set_config(
                    owner_user_id,
                    plugin_id,
                    committed.revision,
                    &previous.plugin.config,
                    &previous.credential_bindings,
                    &previous_grants,
                    positive_now_ms(),
                )
                .await;
            return self
                .finish_lifecycle_rollback(owner_user_id, error, rollback)
                .await;
        }
        Ok(committed)
    }

    async fn finish_lifecycle_rollback(
        &self,
        owner_user_id: &str,
        mutation: PluginInstallError,
        rollback: Result<PluginRecord, PluginRepositoryError>,
    ) -> PluginInstallResult<PluginRecord> {
        match rollback {
            Ok(plugin) => match self.restore_runtime(owner_user_id, &plugin).await {
                Ok(()) => Err(PluginInstallError::Activation(mutation.to_string())),
                Err(recovery) => Err(PluginInstallError::Recovery {
                    mutation: mutation.to_string(),
                    recovery: recovery.to_string(),
                }),
            },
            Err(recovery) => Err(PluginInstallError::Recovery {
                mutation: mutation.to_string(),
                recovery: recovery.to_string(),
            }),
        }
    }

    pub async fn trash(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
    ) -> PluginInstallResult<PluginRecord> {
        self.set_trashed(owner_user_id, plugin_id, expected_revision, true)
            .await
    }

    pub async fn restore_from_trash(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
    ) -> PluginInstallResult<PluginRecord> {
        self.set_trashed(owner_user_id, plugin_id, expected_revision, false)
            .await
    }

    async fn set_trashed(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        trashed: bool,
    ) -> PluginInstallResult<PluginRecord> {
        let _guard = self.mutations.acquire(plugin_id).await;
        let previous = self
            .inventory_at_revision(owner_user_id, plugin_id, expected_revision)
            .await?;
        if previous.plugin.trashed_at_ms.is_some() == trashed {
            return Err(PluginInstallError::InvalidUpdate(if trashed {
                "Plugin is already trashed".into()
            } else {
                "Plugin is not in Trash".into()
            }));
        }
        self.revoke_admission(&previous).await?;
        let committed = match self
            .repository
            .trash(
                owner_user_id,
                plugin_id,
                expected_revision,
                trashed,
                positive_now_ms(),
            )
            .await
        {
            Ok(plugin) => plugin,
            Err(error) => {
                self.admit_inventory(&previous).await?;
                return Err(error.into());
            }
        };
        if !trashed {
            let current = self
                .repository
                .inventory(owner_user_id, plugin_id)
                .await?
                .ok_or(PluginRepositoryError::NotFound)?;
            if let Err(error) = self.admit_inventory(&current).await {
                let rollback = self
                    .repository
                    .trash(
                        owner_user_id,
                        plugin_id,
                        committed.revision,
                        true,
                        positive_now_ms(),
                    )
                    .await;
                return self
                    .finish_lifecycle_rollback(owner_user_id, error, rollback)
                    .await;
            }
        }
        Ok(committed)
    }

    pub async fn restore_previous(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        restore_data: bool,
    ) -> PluginInstallResult<PluginRecord> {
        let _guard = self.mutations.acquire(plugin_id).await;
        let previous = self
            .inventory_at_revision(owner_user_id, plugin_id, expected_revision)
            .await?;
        if previous.plugin.trashed_at_ms.is_some() {
            return Err(PluginInstallError::InvalidUpdate(
                "a trashed Plugin cannot restore Previous".into(),
            ));
        }
        let new_artifact_digest = previous
            .plugin
            .previous_artifact_digest
            .clone()
            .ok_or_else(|| PluginInstallError::InvalidUpdate("Previous Artifact is absent".into()))?;
        let new_data_generation = if restore_data {
            previous.plugin.previous_data_generation.clone().ok_or_else(|| {
                PluginInstallError::InvalidUpdate("Previous DataRoot is absent".into())
            })?
        } else {
            previous.plugin.data_generation.clone()
        };
        self.revoke_admission(&previous).await?;
        let mutation = PluginMutationRecord {
            mutation_id: PluginMutationId::from(Uuid::now_v7().to_string()),
            owner_user_id: owner_user_id.to_owned(),
            plugin_id: plugin_id.clone(),
            kind: PluginMutationKind::Restore,
            phase: PluginMutationPhase::Prepared,
            old_artifact_digest: Some(previous.plugin.active_artifact_digest.clone()),
            new_artifact_digest: Some(new_artifact_digest),
            old_data_generation: Some(previous.plugin.data_generation.clone()),
            new_data_generation: Some(new_data_generation),
            expected_revision: Some(expected_revision),
            error: None,
            created_at_ms: positive_now_ms(),
            updated_at_ms: positive_now_ms(),
        };
        let committed = match self
            .repository
            .restore_previous(&mutation, expected_revision, restore_data, positive_now_ms())
            .await
        {
            Ok(plugin) => plugin,
            Err(error) => {
                self.admit_inventory(&previous).await?;
                return Err(error.into());
            }
        };
        let current = self
            .repository
            .inventory(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        if let Err(error) = self.admit_inventory(&current).await {
            let rollback = self
                .repository
                .rollback_install(
                    &mutation.mutation_id,
                    owner_user_id,
                    plugin_id,
                    positive_now_ms(),
                )
                .await;
            return self
                .finish_lifecycle_rollback(owner_user_id, error, rollback)
                .await;
        }
        self.repository.finish_mutation(&mutation.mutation_id).await?;
        Ok(committed)
    }

    pub async fn permanent_delete(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
    ) -> PluginInstallResult<()> {
        let _guard = self.mutations.acquire(plugin_id).await;
        let previous = self
            .inventory_at_revision(owner_user_id, plugin_id, expected_revision)
            .await?;
        if previous.plugin.trashed_at_ms.is_none() {
            return Err(PluginInstallError::InvalidUpdate(
                "Plugin must be in Trash before permanent deletion".into(),
            ));
        }
        let now = positive_now_ms();
        let mutation = PluginMutationRecord {
            mutation_id: PluginMutationId::from(Uuid::now_v7().to_string()),
            owner_user_id: owner_user_id.to_owned(),
            plugin_id: plugin_id.clone(),
            kind: PluginMutationKind::PermanentDelete,
            phase: PluginMutationPhase::Prepared,
            old_artifact_digest: Some(previous.plugin.active_artifact_digest.clone()),
            new_artifact_digest: None,
            old_data_generation: Some(previous.plugin.data_generation.clone()),
            new_data_generation: None,
            expected_revision: Some(expected_revision),
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        self.repository.begin_mutation(&mutation).await?;
        if let Err(error) = self.revoke_admission(&previous).await {
            let _ = self.repository.finish_mutation(&mutation.mutation_id).await;
            return Err(error);
        }
        if let Err(error) = self
            .repository
            .delete_plugin_rows(
                &mutation.mutation_id,
                owner_user_id,
                plugin_id,
                expected_revision,
            )
            .await
        {
            match self.repository.get_plugin(owner_user_id, plugin_id).await {
                Ok(Some(_)) => {
                    let _ = self.repository.finish_mutation(&mutation.mutation_id).await;
                    self.admit_inventory(&previous).await?;
                    return Err(error.into());
                }
                Ok(None) => {}
                Err(recovery) => {
                    return Err(PluginInstallError::Recovery {
                        mutation: error.to_string(),
                        recovery: recovery.to_string(),
                    });
                }
            }
        }
        self.data_roots.delete_plugin_exact(plugin_id)?;
        self.repository.finish_mutation(&mutation.mutation_id).await?;
        Ok(())
    }

    /// Resolve incomplete journal rows after a crash. Pre-commit rows never
    /// changed core pointers; committed rows are started from the new pointers
    /// or rolled back by the same evidence-bearing journal.
    pub async fn recover(&self) -> PluginInstallResult<()> {
        for mutation in self.repository.list_mutations().await? {
            let _guard = self.mutations.acquire(&mutation.plugin_id).await;
            let plugin = self
                .repository
                .get_plugin(&mutation.owner_user_id, &mutation.plugin_id)
                .await?;

            if mutation.kind == PluginMutationKind::PermanentDelete {
                if plugin.is_none() {
                    // `delete_plugin_rows` commits the missing Plugin row and
                    // journal phase together. Therefore absence is durable
                    // evidence that only exact Plugin DataRoot cleanup remains.
                    self.data_roots.delete_plugin_exact(&mutation.plugin_id)?;
                } else if let Some(plugin) = &plugin {
                    self.restore_runtime(&mutation.owner_user_id, plugin).await?;
                }
                self.repository.finish_mutation(&mutation.mutation_id).await?;
                continue;
            }

            match mutation.phase {
                PluginMutationPhase::Staging | PluginMutationPhase::Prepared => {
                    self.cleanup_candidate_data(&mutation)?;
                    self.repository.finish_mutation(&mutation.mutation_id).await?;
                    if let Some(plugin) = plugin {
                        self.restore_runtime(&mutation.owner_user_id, &plugin).await?;
                    }
                }
                PluginMutationPhase::Committed => {
                    match plugin {
                        Some(plugin) => {
                            if let Err(error) =
                                self.restore_runtime(&mutation.owner_user_id, &plugin).await
                            {
                                self.rollback_journaled_mutation(&mutation, error.to_string())
                                    .await?;
                            } else {
                                self.prune_retired_generations(&plugin)?;
                                self.repository
                                    .finish_mutation(&mutation.mutation_id)
                                    .await?;
                            }
                        }
                        None if mutation.kind == PluginMutationKind::Install => {
                            self.cleanup_candidate_data(&mutation)?;
                            self.repository
                                .finish_mutation(&mutation.mutation_id)
                                .await?;
                        }
                        None => return Err(PluginRepositoryError::NotFound.into()),
                    }
                }
                PluginMutationPhase::RollingBack | PluginMutationPhase::Failed => {
                    match plugin {
                        Some(plugin) if mutation_points_at_new(&mutation, &plugin) => {
                            self.rollback_journaled_mutation(
                                &mutation,
                                mutation
                                    .error
                                    .clone()
                                    .unwrap_or_else(|| "interrupted Plugin mutation".into()),
                            )
                            .await?;
                        }
                        Some(plugin) => {
                            self.cleanup_candidate_data(&mutation)?;
                            self.repository
                                .finish_mutation(&mutation.mutation_id)
                                .await?;
                            self.restore_runtime(&mutation.owner_user_id, &plugin).await?;
                        }
                        None if mutation.kind == PluginMutationKind::Install => {
                            self.cleanup_candidate_data(&mutation)?;
                            self.repository
                                .finish_mutation(&mutation.mutation_id)
                                .await?;
                        }
                        None => return Err(PluginRepositoryError::NotFound.into()),
                    }
                }
            }
        }
        Ok(())
    }

    fn prune_retired_generations(&self, plugin: &PluginRecord) -> PluginInstallResult<()> {
        let mut keep = BTreeSet::from([DataGeneration::new(
            plugin.data_generation.clone(),
        )?]);
        if let Some(previous) = &plugin.previous_data_generation {
            keep.insert(DataGeneration::new(previous.clone())?);
        }
        self.data_roots
            .prune_generations(&plugin.plugin_id, &keep)?;
        Ok(())
    }

    fn cleanup_candidate_data(
        &self,
        mutation: &PluginMutationRecord,
    ) -> PluginInstallResult<()> {
        if !matches!(
            mutation.kind,
            PluginMutationKind::Install | PluginMutationKind::Update
        ) {
            return Ok(());
        }
        let Some(generation) = mutation.new_data_generation.as_ref() else {
            return Ok(());
        };
        if mutation.old_data_generation.as_deref() == Some(generation) {
            return Ok(());
        }
        let generation = DataGeneration::new(generation.clone())?;
        self.data_roots
            .delete_staging_exact(&mutation.plugin_id, &generation)?;
        self.data_roots
            .delete_generation_exact(&mutation.plugin_id, &generation)?;
        Ok(())
    }

    async fn rollback_journaled_mutation(
        &self,
        mutation: &PluginMutationRecord,
        failure: String,
    ) -> PluginInstallResult<()> {
        let rollback = self
            .repository
            .rollback_install(
                &mutation.mutation_id,
                &mutation.owner_user_id,
                &mutation.plugin_id,
                positive_now_ms(),
            )
            .await;
        match rollback {
            Ok(previous) => {
                self.cleanup_candidate_data(mutation)?;
                self.restore_runtime(&mutation.owner_user_id, &previous)
                    .await
                    .map_err(|recovery| PluginInstallError::Recovery {
                        mutation: failure,
                        recovery: recovery.to_string(),
                    })?;
                Ok(())
            }
            Err(PluginRepositoryError::NotFound)
                if mutation.kind == PluginMutationKind::Install =>
            {
                self.cleanup_candidate_data(mutation)?;
                let _ = self.runtime.remove(&mutation.plugin_id).await;
                let _ = self
                    .bindings
                    .remove(&mutation.owner_user_id, &mutation.plugin_id)
                    .await;
                Ok(())
            }
            Err(recovery) => Err(PluginInstallError::Recovery {
                mutation: failure,
                recovery: recovery.to_string(),
            }),
        }
    }
}

fn mutation_points_at_new(mutation: &PluginMutationRecord, plugin: &PluginRecord) -> bool {
    mutation
        .new_artifact_digest
        .as_ref()
        .is_some_and(|digest| digest == &plugin.active_artifact_digest)
        && mutation
            .new_data_generation
            .as_ref()
            .is_none_or(|generation| generation == &plugin.data_generation)
}

fn validate_target(
    request: &InstallArtifactRequest,
    current: Option<&PluginInventory>,
) -> PluginInstallResult<()> {
    match (&request.target, current) {
        (InstallTarget::New { .. }, None) => Ok(()),
        (InstallTarget::Existing { expected_revision, .. }, Some(inventory))
            if inventory.plugin.revision == *expected_revision =>
        {
            Ok(())
        }
        _ => Err(PluginRepositoryError::Conflict.into()),
    }
}

fn validate_config(artifact: &PluginArtifact, config: &Value) -> PluginInstallResult<()> {
    if !config.is_object() {
        return Err(PluginInstallError::InvalidConfig(
            "Config must be a JSON object".into(),
        ));
    }
    let validator = jsonschema::validator_for(&artifact.manifest.config_schema.0)
        .map_err(|error| PluginInstallError::InvalidConfig(error.to_string()))?;
    if let Err(error) = validator.validate(config) {
        return Err(PluginInstallError::InvalidConfig(error.to_string()));
    }
    if contains_declared_secret_field(config, &artifact.manifest.secrets) {
        return Err(PluginInstallError::InvalidConfig(
            "Config cannot contain a declared Credential slot; bind it through the Credential Store"
                .into(),
        ));
    }
    Ok(())
}

fn contains_declared_secret_field(value: &Value, slots: &[String]) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            slots.iter().any(|slot| slot == key) || contains_declared_secret_field(value, slots)
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_declared_secret_field(value, slots)),
        _ => false,
    }
}

fn validate_non_permission_confirmations(
    artifact: &PluginArtifact,
    current: Option<&PluginInventory>,
    confirmed_secret_slots: &BTreeSet<String>,
    trusted_local_service_confirmed: bool,
) -> PluginInstallResult<()> {
    let existing_secret_slots = current
        .map(|inventory| {
            inventory
                .artifact
                .artifact
                .manifest
                .secrets
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let required_secret_slots = artifact
        .manifest
        .secrets
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let added_secret_slots = required_secret_slots
        .difference(&existing_secret_slots)
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing = added_secret_slots
        .difference(confirmed_secret_slots)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        return Err(PluginInstallError::SecretConfirmationRequired(missing));
    }
    let adds_local_service = artifact.manifest.has_service()
        && current
            .is_none_or(|inventory| !inventory.artifact.artifact.manifest.has_service());
    if adds_local_service && !trusted_local_service_confirmed {
        return Err(PluginInstallError::LocalServiceConfirmationRequired);
    }
    Ok(())
}

fn resolved_grants(
    artifact: &PluginArtifact,
    current: Option<&PluginInventory>,
    confirmed: &BTreeSet<String>,
) -> PluginInstallResult<BTreeMap<String, bool>> {
    let existing = current
        .map(|inventory| {
            inventory
                .grants
                .iter()
                .map(|(permission, grant)| (permission.clone(), grant.granted))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let required = &artifact.manifest.permissions;
    let existing_permissions = existing.keys().cloned().collect::<BTreeSet<_>>();
    let expansion = required
        .difference(&existing_permissions)
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing = expansion
        .difference(confirmed)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        return Err(PluginInstallError::PermissionConfirmationRequired(missing));
    }
    Ok(required
        .iter()
        .map(|permission| {
            (
                permission.clone(),
                existing.get(permission).copied().unwrap_or(true),
            )
        })
        .collect())
}

fn migration_chain(
    artifact: &PluginArtifact,
    from: u32,
    to: u32,
) -> PluginInstallResult<Vec<PluginMigrationManifest>> {
    if from == to {
        return Ok(Vec::new());
    }
    let mut chain = artifact
        .manifest
        .migrations
        .iter()
        .filter(|migration| migration.from >= from && migration.to <= to)
        .cloned()
        .collect::<Vec<_>>();
    chain.sort_by_key(|migration| migration.from);
    let mut cursor = from;
    for migration in &chain {
        if migration.from != cursor || migration.to != cursor.saturating_add(1) {
            return Err(PluginInstallError::InvalidUpdate(format!(
                "migration chain does not cover dataVersion {cursor}"
            )));
        }
        cursor = migration.to;
    }
    if cursor != to {
        return Err(PluginInstallError::InvalidUpdate(format!(
            "migration chain ends at dataVersion {cursor}, expected {to}"
        )));
    }
    Ok(chain)
}

fn staged_plugin_record(
    request: &InstallArtifactRequest,
    current: Option<&PluginInventory>,
    artifact: &PluginArtifact,
    data_generation: &str,
) -> PluginInstallResult<PluginRecord> {
    let now = positive_now_ms();
    let revision = current
        .map(|inventory| {
            inventory
                .plugin
                .revision
                .checked_add(1)
                .ok_or_else(|| PluginInstallError::InvalidUpdate("Plugin revision overflow".into()))
        })
        .transpose()?
        .unwrap_or(1);
    let changed_generation = current
        .is_some_and(|inventory| inventory.plugin.data_generation != data_generation);
    Ok(PluginRecord {
        plugin_id: request.target.plugin_id().clone(),
        owner_user_id: request.owner_user_id.clone(),
        package_id: request
            .local_package_id
            .clone()
            .or_else(|| current.map(|inventory| inventory.plugin.package_id.clone()))
            .unwrap_or_else(|| artifact.manifest.id.clone()),
        name: artifact.manifest.name.clone(),
        description: artifact.manifest.description.clone(),
        enabled: current
            .map(|inventory| inventory.plugin.enabled)
            .unwrap_or(true),
        trashed_at_ms: None,
        active_artifact_digest: artifact.artifact_digest.clone(),
        previous_artifact_digest: current
            .map(|inventory| inventory.plugin.active_artifact_digest.clone()),
        data_generation: data_generation.to_owned(),
        previous_data_generation: current
            .filter(|_| changed_generation)
            .map(|inventory| inventory.plugin.data_generation.clone()),
        revision,
        config: request.config.clone(),
        last_error: None,
        created_at_ms: current
            .map(|inventory| inventory.plugin.created_at_ms)
            .unwrap_or(now),
        updated_at_ms: now,
    })
}

pub fn action_publications(
    plugin: &PluginRecord,
    artifact: &PluginArtifact,
) -> Vec<PluginActionPublication> {
    artifact
        .manifest
        .actions
        .iter()
        .map(|(action_id, action)| PluginActionPublication {
            plugin_id: plugin.plugin_id.clone(),
            artifact: PluginArtifactRef::from(artifact),
            action_id: action_id.clone(),
            action: action.clone(),
            bindings: artifact
                .manifest
                .bindings
                .iter()
                .filter_map(|binding| {
                    (binding.action == *action_id).then_some(binding.point)
                })
                .collect(),
        })
        .collect()
}

fn runtime_context(
    inventory: PluginInventory,
    data_root: PluginDataRootHandle,
) -> PluginRuntimeContext {
    let granted_permissions = inventory
        .grants
        .into_iter()
        .filter_map(|(permission, grant)| grant.granted.then_some(permission))
        .collect();
    PluginRuntimeContext {
        owner_user_id: inventory.plugin.owner_user_id.clone(),
        plugin: inventory.plugin,
        artifact: inventory.artifact.artifact,
        data_root,
        credential_bindings: inventory.credential_bindings,
        granted_permissions,
    }
}

fn positive_now_ms() -> i64 {
    nomifun_common::now_ms().max(1)
}

#[allow(dead_code)]
fn _schema(value: Value) -> StrictJsonValue {
    StrictJsonValue(value)
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use nomifun_agent_contracts::{DigestHex, PluginArtifactFile, PluginManifest};
    use serde_json::json;

    fn artifact() -> PluginArtifact {
        let manifest = PluginManifest::parse(
            br#"{
              "schema":"nomifun.plugin/v1",
              "id":"local.config-test",
              "version":"1.0.0",
              "name":"Config test",
              "description":"Credential separation fixture",
              "hostApi":">=1 <2",
              "entrypoints":{"ui":"ui/index.html"},
              "actions":{},
              "bindings":[],
              "dataVersion":0,
              "migrations":[],
              "configSchema":{"type":"object"},
              "secrets":["api_key"],
              "permissions":[]
            }"#,
        )
        .unwrap();
        PluginArtifact::new(
            manifest,
            vec![
                PluginArtifactFile {
                    normalized_relative_path: "nomifun.plugin.json".into(),
                    digest: DigestHex::from("1".repeat(64)),
                    size_bytes: 1,
                },
                PluginArtifactFile {
                    normalized_relative_path: "ui/index.html".into(),
                    digest: DigestHex::from("2".repeat(64)),
                    size_bytes: 1,
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn config_cannot_smuggle_a_declared_credential_slot() {
        let artifact = artifact();
        assert!(validate_config(&artifact, &json!({"theme":"dark"})).is_ok());
        assert!(matches!(
            validate_config(&artifact, &json!({"nested":{"api_key":"plaintext"}})),
            Err(PluginInstallError::InvalidConfig(message))
                if message.contains("Credential Store")
        ));
    }
}
