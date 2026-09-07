use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, ArtifactFileDigest, ArtifactId, CandidateTestCredentialMode,
    CandidateTestOutcome, CandidateTestReceipt, CandidateTestReceiptId, CapabilityActionDescriptor,
    CapabilityConsumer, CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest,
    CanonicalSchemaRef, CredentialSlotDeclaration, CredentialSlotKey, CredentialSlotKind,
    DigestHex, EffectClass, ExactVersionRef, JAVASCRIPT_HOST_PROTOCOL_VERSION,
    JAVASCRIPT_SDK_CONTRACT_VERSION, JavaScriptBuildProfile, JavaScriptEntrypointMetadata,
    LocalizedMetadata, MINIMUM_NODE_MAJOR, NodeRuntimeFingerprint, NodeRuntimeSourceKind,
    PLUGIN_N1_SCHEMA_VERSION, PLUGIN_PACKAGE_PROFILE_VERSION, PackageContributions, PackageId,
    PackageManifest, PlatformConstraint, PluginAutoApplyEligibility, PluginCompatibility,
    PluginContractChangeKind, PluginContractDiff, PluginHostCommitFence,
    PluginPackageArtifactV1, PluginPackageV1Manifest, RuntimeTarget, StrictJsonValue,
    VersionString,
};
use nomifun_api_types::{
    ApplyPluginCandidateRequest, ApplyPluginTargetDto, BuildPluginProjectRequest,
    ConfigurePluginRequest, CreatePluginProjectRequest, DeletePluginDataRequest,
    DeletePluginProjectRequest,
    ImportPluginRequest, PluginImportKindDto, PluginLifecycleDto, PluginProjectSourceStateDto,
    PluginCandidateOriginDto, RestorePluginPreviousRequest,
    SetPluginEnabledRequest, TestPluginCandidateRequest, UninstallPluginRequest,
};
use nomifun_db::{
    ApplyPluginCandidateParams, CreatePluginArtifactParams, CreatePluginProjectParams,
    DeletePluginProjectParams, FinishProductOperationParams,
    ListPluginCredentialBindingsParams, PluginArtifactRow,
    PluginCandidateTestReceiptRow, PluginCredentialBindingRow, PluginCredentialBindingSnapshot,
    PluginMountRow, PluginProjectRow, PluginReadyCandidateRow, ProductOperationRow,
    RecordPluginCandidateTestReceiptParams, RecordPluginReadyCandidateParams,
    ReplacePluginCredentialBindingsParams,
    RestorePluginMountParams, StartProductOperationParams, UninstallPluginMountParams,
    UpdatePluginMountConfigParams,
};
use nomifun_plugin_service::{
    BuildOutput, CandidateTestOutput, ConfigureInput, CreateProjectInput,
    CreatedPluginSource, ImportedPluginArtifact, LinkPluginProjectParams,
    PluginApplicationService, PluginArtifactStorePort, PluginBuildExecutor,
    PluginCandidateTestExecutor, PluginHostCoordinator, PluginInventory,
    PluginMountDataStore, PluginOperationCancellation, PluginRegistryPublisher,
    PluginRepository, PluginServiceDependencies, PluginServiceError,
    PluginServicePaths, PluginSourceStorePort, ERR_RECONCILE_REQUIRED,
    ERR_RUNTIME, ERR_STALE,
};
use nomifun_plugin_platform::{OwnerMutationCoordinator, PluginOwnerMutationScope};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
struct FakeState {
    library_revision: u64,
    artifacts: Vec<PluginArtifactRow>,
    projects: Vec<PluginProjectRow>,
    mounts: Vec<PluginMountRow>,
    candidates: Vec<PluginReadyCandidateRow>,
    receipts: Vec<PluginCandidateTestReceiptRow>,
    credentials: BTreeMap<String, PluginCredentialBindingSnapshot>,
    operations: Vec<ProductOperationRow>,
}

#[derive(Default)]
struct FakeRepository {
    state: Mutex<FakeState>,
}

impl FakeRepository {
    async fn insert_project(&self, row: PluginProjectRow) {
        self.state.lock().await.projects.push(row);
    }

    async fn insert_mount(&self, row: PluginMountRow) {
        self.state.lock().await.mounts.push(row);
    }

    async fn insert_artifact(&self, row: PluginArtifactRow) {
        self.state.lock().await.artifacts.push(row);
    }

    async fn insert_candidate(&self, row: PluginReadyCandidateRow) {
        let mut state = self.state.lock().await;
        if let Some(project) = state
            .projects
            .iter_mut()
            .find(|project| project.project_id == row.project_id)
        {
            project.ready_candidate_id = Some(row.candidate_id.clone());
        }
        state.candidates.push(row);
    }

    async fn snapshot(&self) -> PluginInventory {
        let state = self.state.lock().await;
        PluginInventory {
            library_revision: state.library_revision,
            artifacts: state.artifacts.clone(),
            projects: state.projects.clone(),
            mounts: state.mounts.clone(),
            candidates: state.candidates.clone(),
            receipts: state.receipts.clone(),
            operations: state.operations.clone(),
        }
    }
}

#[async_trait]
impl PluginRepository for FakeRepository {
    async fn inventory(
        &self,
        owner_user_id: &str,
    ) -> Result<PluginInventory, PluginServiceError> {
        let state = self.state.lock().await;
        let projects = state
            .projects
            .iter()
            .filter(|project| project.owner_user_id == owner_user_id)
            .cloned()
            .collect::<Vec<_>>();
        let project_ids = projects
            .iter()
            .map(|project| project.project_id.clone())
            .collect::<BTreeSet<_>>();
        Ok(PluginInventory {
            library_revision: state.library_revision,
            artifacts: state.artifacts.clone(),
            projects,
            mounts: state.mounts.clone(),
            candidates: state
                .candidates
                .iter()
                .filter(|candidate| project_ids.contains(&candidate.project_id))
                .cloned()
                .collect(),
            receipts: state.receipts.clone(),
            operations: state.operations.clone(),
        })
    }

    async fn get_project(
        &self,
        project_id: &str,
    ) -> Result<Option<PluginProjectRow>, PluginServiceError> {
        Ok(self
            .state
            .lock()
            .await
            .projects
            .iter()
            .find(|project| project.project_id == project_id)
            .cloned())
    }

    async fn get_project_for_mount(
        &self,
        mount_id: &str,
    ) -> Result<Option<PluginProjectRow>, PluginServiceError> {
        Ok(self
            .state
            .lock()
            .await
            .projects
            .iter()
            .find(|project| project.linked_mount_id.as_deref() == Some(mount_id))
            .cloned())
    }

    async fn mount_owner_user_id(
        &self,
        mount_id: &str,
    ) -> Result<Option<String>, PluginServiceError> {
        let state = self.state.lock().await;
        if !state.mounts.iter().any(|mount| mount.mount_id == mount_id) {
            return Ok(None);
        }
        Ok(state
            .projects
            .iter()
            .find(|project| project.linked_mount_id.as_deref() == Some(mount_id))
            .map(|project| project.owner_user_id.clone())
            .or_else(|| Some("user-1".into())))
    }

    async fn get_mount(
        &self,
        mount_id: &str,
    ) -> Result<Option<PluginMountRow>, PluginServiceError> {
        Ok(self
            .state
            .lock()
            .await
            .mounts
            .iter()
            .find(|mount| mount.mount_id == mount_id)
            .cloned())
    }

    async fn get_candidate(
        &self,
        project_id: &str,
    ) -> Result<Option<PluginReadyCandidateRow>, PluginServiceError> {
        let state = self.state.lock().await;
        let ready_id = state
            .projects
            .iter()
            .find(|project| project.project_id == project_id)
            .and_then(|project| project.ready_candidate_id.as_deref());
        Ok(ready_id.and_then(|ready_id| {
            state
                .candidates
                .iter()
                .find(|candidate| candidate.candidate_id == ready_id)
                .cloned()
        }))
    }

    async fn get_artifact(
        &self,
        artifact_digest: &str,
    ) -> Result<Option<PluginArtifactRow>, PluginServiceError> {
        Ok(self
            .state
            .lock()
            .await
            .artifacts
            .iter()
            .find(|artifact| artifact.artifact_digest == artifact_digest)
            .cloned())
    }

    async fn get_test_receipt(
        &self,
        candidate_id: &str,
    ) -> Result<Option<PluginCandidateTestReceiptRow>, PluginServiceError> {
        Ok(self
            .state
            .lock()
            .await
            .receipts
            .iter()
            .find(|receipt| receipt.candidate_id == candidate_id)
            .cloned())
    }

    async fn create_project(
        &self,
        params: &CreatePluginProjectParams,
    ) -> Result<PluginProjectRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let row = PluginProjectRow {
            id: state.projects.len() as i64 + 1,
            project_id: params.project_id.clone(),
            owner_user_id: params.owner_user_id.clone(),
            package_id: params.package_id.clone(),
            display_name: params.display_name.clone(),
            description: params.description.clone(),
            managed_source_path: params.managed_source_path.clone(),
            source_head_digest: params.source_head_digest.clone(),
            dependency_lock_digest: params.dependency_lock_digest.clone(),
            build_generation: params.initial_build_generation,
            linked_mount_id: None,
            ready_candidate_id: None,
            created_at: params.created_at,
            updated_at: params.created_at,
        };
        state.projects.push(row.clone());
        state.library_revision += 1;
        Ok(row)
    }

    async fn delete_project_cas(
        &self,
        params: &DeletePluginProjectParams,
    ) -> Result<bool, PluginServiceError> {
        let mut state = self.state.lock().await;
        let Some(index) = state.projects.iter().position(|project| {
            project.project_id == params.project_id
                && project.owner_user_id == params.owner_user_id
                && project.updated_at == params.expected_updated_at
                && project.build_generation == params.expected_generation
        }) else {
            return Err(PluginServiceError::stale("project delete CAS"));
        };
        let candidate_id = state.projects[index].ready_candidate_id.clone();
        if candidate_id != params.expected_ready_candidate_id {
            return Err(PluginServiceError::stale("project candidate delete CAS"));
        }
        state.projects.remove(index);
        let removed_candidate_ids = state
            .candidates
            .iter()
            .filter(|candidate| candidate.project_id == params.project_id)
            .map(|candidate| candidate.candidate_id.clone())
            .collect::<BTreeSet<_>>();
        state.candidates.retain(|candidate| {
            candidate.project_id != params.project_id
        });
        state
            .receipts
            .retain(|receipt| !removed_candidate_ids.contains(&receipt.candidate_id));
        state.library_revision += 1;
        Ok(true)
    }

    async fn link_project(
        &self,
        params: &LinkPluginProjectParams,
    ) -> Result<PluginProjectRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let mount = state
            .mounts
            .iter()
            .find(|mount| mount.mount_id == params.mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision as u64 != params.expected_mount_revision
            || mount.current_artifact_digest.as_deref()
                != Some(params.expected_target_digest.as_str())
        {
            return Err(PluginServiceError::stale("linked mount CAS"));
        }
        let project = state
            .projects
            .iter_mut()
            .find(|project| project.project_id == params.project_id)
            .ok_or_else(|| PluginServiceError::not_found("project"))?;
        if project.updated_at as u64 != params.expected_project_revision {
            return Err(PluginServiceError::stale("linked project CAS"));
        }
        if project.owner_user_id != params.owner_user_id {
            return Err(PluginServiceError::forbidden("project owner"));
        }
        project.linked_mount_id = Some(params.mount_id.clone());
        project.updated_at = params.updated_at;
        Ok(project.clone())
    }

    async fn put_artifact(
        &self,
        params: &CreatePluginArtifactParams,
    ) -> Result<PluginArtifactRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        if let Some(existing) = state
            .artifacts
            .iter()
            .find(|artifact| artifact.artifact_digest == params.artifact_digest)
        {
            return Ok(existing.clone());
        }
        let row = PluginArtifactRow {
            id: state.artifacts.len() as i64 + 1,
            artifact_id: params.artifact_id.clone(),
            artifact_digest: params.artifact_digest.clone(),
            package_id: params.package_id.clone(),
            package_version: params.package_version.clone(),
            manifest_digest: params.manifest_digest.clone(),
            manifest_json: serde_json::to_string(&params.manifest).unwrap(),
            managed_path: params.managed_path.clone(),
            created_at: params.created_at,
        };
        state.artifacts.push(row.clone());
        Ok(row)
    }

    async fn start_operation(
        &self,
        params: &StartProductOperationParams,
    ) -> Result<ProductOperationRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        if !state
            .projects
            .iter()
            .any(|project| project.project_id == params.owner_id)
        {
            return Err(PluginServiceError::conflict(
                "operation owner must exist before start",
            ));
        }
        let row = ProductOperationRow {
            id: state.operations.len() as i64 + 1,
            operation_id: params.operation_id.clone(),
            kind: params.kind.as_str().into(),
            owner_kind: params.owner_kind.clone(),
            owner_id: params.owner_id.clone(),
            state: "running".into(),
            progress_percent: params.progress_percent.map(i64::from),
            last_error_code: None,
            bounded_log_tail_json: serde_json::to_string(&params.bounded_log_tail).unwrap(),
            started_at_ms: params.started_at_ms,
            finished_at_ms: None,
        };
        state.operations.push(row.clone());
        Ok(row)
    }

    async fn finish_operation(
        &self,
        params: &FinishProductOperationParams,
    ) -> Result<ProductOperationRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let row = state
            .operations
            .iter_mut()
            .find(|operation| operation.operation_id == params.operation_id)
            .ok_or_else(|| PluginServiceError::not_found("operation"))?;
        row.state = params.state.as_str().into();
        row.progress_percent = params.progress_percent.map(i64::from);
        row.last_error_code = params.last_error_code.clone();
        row.bounded_log_tail_json = serde_json::to_string(&params.bounded_log_tail).unwrap();
        row.finished_at_ms = Some(params.finished_at_ms);
        Ok(row.clone())
    }

    async fn record_candidate(
        &self,
        params: &RecordPluginReadyCandidateParams,
    ) -> Result<PluginReadyCandidateRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let artifact = state
            .artifacts
            .iter()
            .find(|artifact| artifact.artifact_digest == params.artifact_digest)
            .cloned()
            .ok_or_else(|| PluginServiceError::not_found("artifact"))?;
        let next_candidate_id = state.candidates.len() as i64 + 1;
        let project = state
            .projects
            .iter_mut()
            .find(|project| project.project_id == params.project_id)
            .ok_or_else(|| PluginServiceError::not_found("project"))?;
        if project.build_generation != params.expected_generation {
            return Err(PluginServiceError::stale("candidate generation"));
        }
        let row = PluginReadyCandidateRow {
            id: next_candidate_id,
            candidate_id: params.candidate_id.clone(),
            project_id: params.project_id.clone(),
            candidate_digest: params.candidate_digest.clone(),
            origin_kind: params.origin.as_str().into(),
            artifact_id: params.artifact_id.clone(),
            artifact_digest: params.artifact_digest.clone(),
            target_package_id: artifact.package_id,
            target_package_version: artifact.package_version,
            target_manifest_digest: artifact.manifest_digest,
            base_target_digest: params.base_target_digest.clone(),
            source_snapshot_digest: params.source_snapshot_digest.clone(),
            dependency_lock_digest: params.dependency_lock_digest.clone(),
            contract_diff_json: serde_json::to_string(&params.contract_diff).unwrap(),
            origin_operation_id: params.origin_operation_id.clone(),
            build_generation: params.expected_generation,
            created_at: params.created_at,
        };
        project.ready_candidate_id = Some(row.candidate_id.clone());
        state
            .candidates
            .retain(|candidate| candidate.project_id != row.project_id);
        state.candidates.push(row.clone());
        Ok(row)
    }

    async fn record_test_receipt(
        &self,
        params: &RecordPluginCandidateTestReceiptParams,
    ) -> Result<PluginCandidateTestReceiptRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let row = PluginCandidateTestReceiptRow {
            id: state.receipts.len() as i64 + 1,
            receipt_id: params.receipt_id.clone(),
            candidate_id: params.candidate_id.clone(),
            candidate_digest: params.candidate_digest.clone(),
            artifact_id: params.artifact_id.clone(),
            artifact_digest: params.artifact_digest.clone(),
            receipt_digest: params.receipt_digest.clone(),
            runtime_fingerprint_digest: params.runtime_fingerprint_digest.clone(),
            receipt_json: serde_json::to_string(&params.receipt).unwrap(),
            tested_at: params.tested_at,
        };
        state.receipts.push(row.clone());
        Ok(row)
    }

    async fn apply_candidate(
        &self,
        params: &ApplyPluginCandidateParams,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let candidate = state
            .candidates
            .iter()
            .find(|candidate| candidate.candidate_id == params.candidate_id)
            .cloned()
            .ok_or_else(|| PluginServiceError::not_found("candidate"))?;
        let project_index = state
            .projects
            .iter()
            .position(|project| project.project_id == params.project_id)
            .ok_or_else(|| PluginServiceError::not_found("project"))?;
        if state.projects[project_index].build_generation != params.expected_project_generation
            || state.projects[project_index].ready_candidate_id.as_deref()
                != Some(candidate.candidate_id.as_str())
        {
            return Err(PluginServiceError::stale("apply project CAS"));
        }
        let mount_index =
            if let Some(mount_id) = state.projects[project_index].linked_mount_id.as_deref() {
                state
                    .mounts
                    .iter()
                    .position(|mount| mount.mount_id == mount_id)
                    .ok_or_else(|| PluginServiceError::not_found("mount"))?
            } else {
                let mount_id = params
                    .new_mount_id
                    .clone()
                    .ok_or_else(|| PluginServiceError::invalid("new mount id"))?;
                state.mounts.push(mount_row(
                    &mount_id,
                    &candidate.target_package_id,
                    None,
                    0,
                ));
                state.mounts.len() - 1
            };
        let mount = &mut state.mounts[mount_index];
        if mount.revision != params.expected_mount_revision.unwrap_or_default()
            || mount.current_artifact_digest != params.expected_current_artifact_digest
            || candidate.base_target_digest != mount.current_artifact_digest
        {
            return Err(PluginServiceError::stale("apply mount CAS"));
        }
        mount.previous_artifact_digest = mount.current_artifact_digest.take();
        mount.current_artifact_digest = Some(candidate.artifact_digest.clone());
        mount.revision += 1;
        mount.enabled = true;
        mount.retained = false;
        mount.config_schema_digest = Some(params.config_schema_digest.clone());
        mount.updated_at = params.applied_at;
        let result = mount.clone();
        state.projects[project_index].linked_mount_id = Some(result.mount_id.clone());
        state.projects[project_index].ready_candidate_id = None;
        state
            .candidates
            .retain(|existing| existing.candidate_id != candidate.candidate_id);
        Ok(result)
    }

    async fn restore_previous(
        &self,
        params: &RestorePluginMountParams,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let mount = state
            .mounts
            .iter_mut()
            .find(|mount| mount.mount_id == params.mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision != params.expected_revision
            || mount.current_artifact_digest.as_deref()
                != Some(params.expected_current_artifact_digest.as_str())
            || mount.previous_artifact_digest.as_deref()
                != Some(params.expected_previous_artifact_digest.as_str())
        {
            return Err(PluginServiceError::stale("restore CAS"));
        }
        std::mem::swap(
            &mut mount.current_artifact_digest,
            &mut mount.previous_artifact_digest,
        );
        mount.revision += 1;
        mount.updated_at = params.restored_at;
        Ok(mount.clone())
    }

    async fn uninstall_retain_data(
        &self,
        params: &UninstallPluginMountParams,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let mount = state
            .mounts
            .iter_mut()
            .find(|mount| mount.mount_id == params.mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision != params.expected_revision
            || mount.current_artifact_digest.as_deref()
                != Some(params.expected_current_artifact_digest.as_str())
        {
            return Err(PluginServiceError::stale("uninstall CAS"));
        }
        mount.current_artifact_digest = None;
        mount.previous_artifact_digest = None;
        mount.enabled = false;
        mount.retained = true;
        mount.revision += 1;
        mount.updated_at = params.uninstalled_at;
        Ok(mount.clone())
    }

    async fn mark_delete_pending(
        &self,
        mount_id: &str,
        expected_revision: i64,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let mount = state
            .mounts
            .iter_mut()
            .find(|mount| mount.mount_id == mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision != expected_revision || !mount.retained {
            return Err(PluginServiceError::stale("delete pending CAS"));
        }
        mount.delete_pending = true;
        mount.revision += 1;
        mount.updated_at = updated_at;
        Ok(mount.clone())
    }

    async fn complete_data_delete(&self, mount_id: &str) -> Result<bool, PluginServiceError> {
        let mut state = self.state.lock().await;
        let before = state.mounts.len();
        state
            .mounts
            .retain(|mount| mount.mount_id != mount_id || !mount.delete_pending);
        state.credentials.remove(mount_id);
        Ok(state.mounts.len() != before)
    }

    async fn update_config(
        &self,
        params: &UpdatePluginMountConfigParams,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let mount = state
            .mounts
            .iter_mut()
            .find(|mount| mount.mount_id == params.mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision != params.expected_mount_revision
            || mount.current_artifact_digest != params.expected_current_artifact_digest
            || mount.config_revision != params.expected_config_revision
            || mount.config_schema_digest != params.expected_config_schema_digest
        {
            return Err(PluginServiceError::stale("config CAS"));
        }
        mount.config_json = serde_json::to_string(&params.config).unwrap();
        mount.config_revision += 1;
        mount.config_schema_digest = Some(params.config_schema_digest.clone());
        Ok(mount.clone())
    }

    async fn replace_credentials(
        &self,
        params: &ReplacePluginCredentialBindingsParams,
    ) -> Result<PluginCredentialBindingSnapshot, PluginServiceError> {
        let mut state = self.state.lock().await;
        let mount = state
            .mounts
            .iter_mut()
            .find(|mount| mount.mount_id == params.mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision != params.expected_mount_revision
            || mount.current_artifact_digest != params.expected_current_artifact_digest
            || mount.credential_bindings_revision != params.expected_bindings_revision
        {
            return Err(PluginServiceError::stale("credential CAS"));
        }
        mount.credential_bindings_revision += 1;
        let bindings = params
            .bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| PluginCredentialBindingRow {
                id: index as i64 + 1,
                mount_id: params.mount_id.clone(),
                slot: binding.slot.clone(),
                credential_id: binding.credential_id.clone(),
                created_at: params.updated_at,
                updated_at: params.updated_at,
            })
            .collect::<Vec<_>>();
        let snapshot = PluginCredentialBindingSnapshot {
            mount_id: params.mount_id.clone(),
            mount_revision: mount.revision,
            current_artifact_digest: mount.current_artifact_digest.clone(),
            bindings_revision: mount.credential_bindings_revision,
            bindings,
        };
        state
            .credentials
            .insert(params.mount_id.clone(), snapshot.clone());
        Ok(snapshot)
    }

    async fn list_credentials(
        &self,
        params: &ListPluginCredentialBindingsParams,
    ) -> Result<PluginCredentialBindingSnapshot, PluginServiceError> {
        let state = self.state.lock().await;
        let mount = state
            .mounts
            .iter()
            .find(|mount| mount.mount_id == params.mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision != params.expected_mount_revision
            || mount.current_artifact_digest != params.expected_current_artifact_digest
        {
            return Err(PluginServiceError::stale("credential read CAS"));
        }
        Ok(state
            .credentials
            .get(&params.mount_id)
            .cloned()
            .unwrap_or(PluginCredentialBindingSnapshot {
                mount_id: mount.mount_id.clone(),
                mount_revision: mount.revision,
                current_artifact_digest: mount.current_artifact_digest.clone(),
                bindings_revision: mount.credential_bindings_revision,
                bindings: Vec::new(),
            }))
    }

    async fn set_enabled(
        &self,
        mount_id: &str,
        expected_revision: i64,
        expected_digest: &str,
        enabled: bool,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let mount = state
            .mounts
            .iter_mut()
            .find(|mount| mount.mount_id == mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision != expected_revision
            || mount.current_artifact_digest.as_deref() != Some(expected_digest)
        {
            return Err(PluginServiceError::stale("enabled CAS"));
        }
        mount.enabled = enabled;
        mount.revision += 1;
        mount.updated_at = updated_at;
        Ok(mount.clone())
    }

    async fn retry_mount(
        &self,
        mount_id: &str,
        expected_revision: i64,
        expected_digest: &str,
        updated_at: i64,
    ) -> Result<PluginMountRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let mount = state
            .mounts
            .iter_mut()
            .find(|mount| mount.mount_id == mount_id)
            .ok_or_else(|| PluginServiceError::not_found("mount"))?;
        if mount.revision != expected_revision
            || mount.current_artifact_digest.as_deref() != Some(expected_digest)
        {
            return Err(PluginServiceError::stale("retry CAS"));
        }
        mount.last_error = None;
        mount.revision += 1;
        mount.updated_at = updated_at;
        Ok(mount.clone())
    }

    async fn list_operations(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<ProductOperationRow>, PluginServiceError> {
        let state = self.state.lock().await;
        Ok(state
            .operations
            .iter()
            .filter(|operation| {
                state.projects.iter().any(|project| {
                    project.owner_user_id == owner_user_id
                        && ((operation.owner_kind == "plugin_project"
                            && operation.owner_id == project.project_id)
                            || (operation.owner_kind == "plugin_mount"
                                && project.linked_mount_id.as_deref()
                                    == Some(operation.owner_id.as_str())))
                })
            })
            .cloned()
            .collect())
    }

    async fn get_operation(
        &self,
        owner_user_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, PluginServiceError> {
        let state = self.state.lock().await;
        Ok(state
            .operations
            .iter()
            .find(|operation| {
                operation.operation_id == operation_id
                    && state.projects.iter().any(|project| {
                        project.owner_user_id == owner_user_id
                            && ((operation.owner_kind == "plugin_project"
                                && operation.owner_id == project.project_id)
                                || (operation.owner_kind == "plugin_mount"
                                    && project.linked_mount_id.as_deref()
                                        == Some(operation.owner_id.as_str())))
                    })
            })
            .cloned())
    }

    async fn cancel_operation(
        &self,
        owner_user_id: &str,
        operation_id: &str,
        expected_revision: u64,
        finished_at_ms: i64,
    ) -> Result<ProductOperationRow, PluginServiceError> {
        let mut state = self.state.lock().await;
        let operation_index = state
            .operations
            .iter()
            .position(|operation| operation.operation_id == operation_id)
            .ok_or_else(|| PluginServiceError::not_found("operation"))?;
        let owner_allowed = state.projects.iter().any(|project| {
            project.owner_user_id == owner_user_id
                && ((state.operations[operation_index].owner_kind == "plugin_project"
                    && state.operations[operation_index].owner_id == project.project_id)
                    || (state.operations[operation_index].owner_kind == "plugin_mount"
                        && project.linked_mount_id.as_deref()
                            == Some(state.operations[operation_index].owner_id.as_str())))
        });
        if !owner_allowed {
            return Err(PluginServiceError::not_found("operation"));
        }
        let operation = &mut state.operations[operation_index];
        if expected_revision != 1 || operation.state != "running" {
            return Err(PluginServiceError::stale("operation revision"));
        }
        operation.state = "canceled".into();
        operation.finished_at_ms = Some(finished_at_ms.max(operation.started_at_ms));
        Ok(operation.clone())
    }
}

struct QueueArtifactStore {
    artifacts: Mutex<VecDeque<ImportedPluginArtifact>>,
}

impl QueueArtifactStore {
    fn new(artifacts: Vec<ImportedPluginArtifact>) -> Self {
        Self {
            artifacts: Mutex::new(artifacts.into()),
        }
    }
}

#[async_trait]
impl PluginArtifactStorePort for QueueArtifactStore {
    async fn import_directory(
        &self,
        _source: &Path,
    ) -> Result<ImportedPluginArtifact, PluginServiceError> {
        self.import_zip(Path::new("unused")).await
    }

    async fn import_zip(
        &self,
        _source: &Path,
    ) -> Result<ImportedPluginArtifact, PluginServiceError> {
        self.artifacts
            .lock()
            .await
            .pop_front()
            .ok_or_else(|| PluginServiceError::not_found("queued artifact"))
    }

    async fn verify(&self, _artifact: &PluginArtifactRow) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[derive(Default)]
struct FakeHost;

#[async_trait]
impl PluginHostCoordinator for FakeHost {
    async fn commit_fence(
        &self,
        _mount_id: &str,
    ) -> Result<PluginHostCommitFence, PluginServiceError> {
        Ok(PluginHostCommitFence::NotResident)
    }

}

struct UnavailableRuntimeHost;

#[async_trait]
impl PluginHostCoordinator for UnavailableRuntimeHost {
    async fn runtime_available(&self) -> Result<bool, PluginServiceError> {
        Ok(false)
    }

    async fn commit_fence(
        &self,
        _mount_id: &str,
    ) -> Result<PluginHostCommitFence, PluginServiceError> {
        panic!("Enable must reject before requesting a Host fence")
    }
}

#[derive(Default)]
struct FakeDataStore {
    deleted: Mutex<Vec<String>>,
}

#[derive(Default)]
struct FakeSourceStore {
    deleted: Mutex<Vec<String>>,
    fail_delete: bool,
}

#[async_trait]
impl PluginSourceStorePort for FakeSourceStore {
    async fn create_project(
        &self,
        _owner_user_id: &str,
        project_id: &str,
        _request: &CreatePluginProjectRequest,
    ) -> Result<CreatedPluginSource, PluginServiceError> {
        Ok(CreatedPluginSource {
            managed_relative_path: format!(
                "sources/owner/projects/{project_id}/source"
            ),
            source_snapshot_digest: "a".repeat(64),
            dependency_lock_digest: "b".repeat(64),
        })
    }

    async fn delete_project(
        &self,
        _owner_user_id: &str,
        project_id: &str,
    ) -> Result<(), PluginServiceError> {
        if self.fail_delete {
            return Err(PluginServiceError::integration(
                "injected Source cleanup failure",
            ));
        }
        self.deleted.lock().await.push(project_id.to_owned());
        Ok(())
    }
}

#[derive(Default)]
struct FakeOperationCancellation;

#[async_trait]
impl PluginOperationCancellation for FakeOperationCancellation {
    async fn cancel(&self, _operation: &ProductOperationRow) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

struct TerminalizingOperationCancellation {
    repo: Arc<FakeRepository>,
    owner_user_id: String,
}

#[async_trait]
impl PluginOperationCancellation for TerminalizingOperationCancellation {
    async fn cancel(&self, operation: &ProductOperationRow) -> Result<(), PluginServiceError> {
        self.repo
            .cancel_operation(
                &self.owner_user_id,
                &operation.operation_id,
                1,
                operation.started_at_ms + 1,
            )
            .await?;
        Ok(())
    }
}

#[derive(Default)]
struct FakeRegistryPublisher;

#[async_trait]
impl PluginRegistryPublisher for FakeRegistryPublisher {
    async fn reconcile_mount(
        &self,
        _owner_user_id: &str,
        _mount: &PluginMountRow,
    ) -> Result<(), PluginServiceError> {
        Ok(())
    }
}

#[async_trait]
impl PluginMountDataStore for FakeDataStore {
    async fn delete_mount_data(
        &self,
        mount_id: &str,
        _managed_relative_path: &str,
    ) -> Result<(), PluginServiceError> {
        self.deleted.lock().await.push(mount_id.into());
        Ok(())
    }
}

struct FakeBuilder {
    output: Mutex<Option<BuildOutput>>,
}

#[async_trait]
impl PluginBuildExecutor for FakeBuilder {
    async fn build(
        &self,
        _operation_id: &str,
        _project: &PluginProjectRow,
        _request: &BuildPluginProjectRequest,
    ) -> Result<BuildOutput, PluginServiceError> {
        self.output
            .lock()
            .await
            .take()
            .ok_or_else(|| PluginServiceError::not_found("build output"))
    }
}

#[derive(Default)]
struct FakeTester;

#[async_trait]
impl PluginCandidateTestExecutor for FakeTester {
    async fn test(
        &self,
        _project: &PluginProjectRow,
        candidate: &PluginReadyCandidateRow,
        request: &TestPluginCandidateRequest,
    ) -> Result<CandidateTestOutput, PluginServiceError> {
        let source_lineage = match (
            candidate.source_snapshot_digest.as_deref(),
            candidate.dependency_lock_digest.as_deref(),
        ) {
            (Some(source), Some(lock)) => nomifun_agent_contracts::PluginSourceLineage::Managed {
                source_snapshot_digest: source.into(),
                dependency_lock_digest: lock.into(),
                build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            },
            _ => nomifun_agent_contracts::PluginSourceLineage::RuntimeOnly,
        };
        Ok(CandidateTestOutput {
            receipt: CandidateTestReceipt {
                receipt_id: CandidateTestReceiptId::from(Uuid::now_v7().to_string()),
                candidate_id: candidate.candidate_id.clone().into(),
                candidate_digest: candidate.candidate_digest.clone().into(),
                outcome: CandidateTestOutcome::Passed,
                runtime: runtime(),
                host_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
                host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                javascript_sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
                test_contract_version:
                    nomifun_agent_contracts::CANDIDATE_TEST_CONTRACT_VERSION.into(),
                source_lineage,
                credential_mode: CandidateTestCredentialMode::None,
                resolved_test_input_digest: request.resolved_test_input_digest.clone().into(),
                host_generation: 1,
                issued_at_ms: 10,
            },
        })
    }
}

fn service(
    repo: Arc<FakeRepository>,
    store: Arc<QueueArtifactStore>,
    builder: Arc<dyn PluginBuildExecutor>,
    _temp: &TempDir,
) -> PluginApplicationService {
    service_with_source_store(
        repo,
        store,
        builder,
        Arc::new(FakeSourceStore::default()),
    )
}

fn service_with_source_store(
    repo: Arc<FakeRepository>,
    store: Arc<QueueArtifactStore>,
    builder: Arc<dyn PluginBuildExecutor>,
    source_store: Arc<dyn PluginSourceStorePort>,
) -> PluginApplicationService {
    service_with_operation_cancellation(
        repo,
        store,
        builder,
        source_store,
        Arc::new(FakeOperationCancellation),
    )
}

fn service_with_operation_cancellation(
    repo: Arc<FakeRepository>,
    store: Arc<QueueArtifactStore>,
    builder: Arc<dyn PluginBuildExecutor>,
    source_store: Arc<dyn PluginSourceStorePort>,
    operation_cancellation: Arc<dyn PluginOperationCancellation>,
) -> PluginApplicationService {
    service_with_host(
        repo,
        store,
        builder,
        source_store,
        operation_cancellation,
        Arc::new(FakeHost),
    )
}

fn service_with_host(
    repo: Arc<FakeRepository>,
    store: Arc<QueueArtifactStore>,
    builder: Arc<dyn PluginBuildExecutor>,
    source_store: Arc<dyn PluginSourceStorePort>,
    operation_cancellation: Arc<dyn PluginOperationCancellation>,
    host: Arc<dyn PluginHostCoordinator>,
) -> PluginApplicationService {
    PluginApplicationService::new(PluginServiceDependencies {
        repository: repo,
        artifacts: store,
        host,
        registry: Arc::new(FakeRegistryPublisher),
        mutation_coordinator: Arc::new(OwnerMutationCoordinator::new()),
        builder,
        tester: Arc::new(FakeTester),
        operation_cancellation,
        source_store,
        data_store: Arc::new(FakeDataStore::default()),
        paths: PluginServicePaths {
            mount_data_relative_root: "plugin-mount-data".into(),
        },
    })
}

#[tokio::test]
async fn enable_without_runtime_fails_before_mount_mutation() {
    let repo = Arc::new(FakeRepository::default());
    let package = artifact(b"export const plugin = 1;\n", "1.0.0");
    let digest = package.artifact_digest.as_ref().to_owned();
    repo.insert_artifact(artifact_row(&package, "artifact")).await;
    let mut mount = mount_row(
        "mount-runtime",
        "example.csv",
        Some(digest.clone()),
        4,
    );
    mount.enabled = false;
    repo.insert_mount(mount).await;
    let service = service_with_host(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        Arc::new(FakeSourceStore::default()),
        Arc::new(FakeOperationCancellation),
        Arc::new(UnavailableRuntimeHost),
    );

    let error = service
        .set_enabled(
            "user-1",
            SetPluginEnabledRequest {
                mount_id: "mount-runtime".into(),
                expected_mount_revision: 4,
                expected_current_target_digest: digest,
                enabled: true,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), ERR_RUNTIME);
    let persisted = repo
        .get_mount("mount-runtime")
        .await
        .unwrap()
        .unwrap();
    assert!(!persisted.enabled);
    assert_eq!(persisted.revision, 4);
}

fn sha256(bytes: &[u8]) -> DigestHex {
    DigestHex::from(hex::encode(Sha256::digest(bytes)))
}

fn artifact(main: &[u8], version: &str) -> PluginPackageArtifactV1 {
    artifact_with_config_schema(
        main,
        version,
        StrictJsonValue(json!({"type":"object"})),
    )
}

fn artifact_with_config_schema(
    main: &[u8],
    version: &str,
    config_schema: StrictJsonValue,
) -> PluginPackageArtifactV1 {
    let package = ExactVersionRef {
        id: PackageId::from("example.csv"),
        version: VersionString::from(version),
    };
    let input_schema = StrictJsonValue(json!({
        "additionalProperties": false,
        "properties": {"path": {"type": "string"}},
        "required": ["path"],
        "type": "object"
    }));
    let output_schema = StrictJsonValue(json!({
        "additionalProperties": true,
        "type": "object"
    }));
    let input_ref = CanonicalSchemaRef::from(format!(
        "schema://example/input@1#{}",
        nomifun_agent_contracts::digest_payload(&input_schema.0)
            .unwrap()
            .as_ref()
    ));
    let output_ref = CanonicalSchemaRef::from(format!(
        "schema://example/output@1#{}",
        nomifun_agent_contracts::digest_payload(&output_schema.0)
            .unwrap()
            .as_ref()
    ));
    let capability = CapabilityManifest {
        id: CapabilityId::from("example.csv.read"),
        contribution_id: "capability:example.csv.read".into(),
        version: "1.0.0".into(),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: LocalizedMetadata {
            name: "CSV Read".into(),
            description: "Read CSV resources.".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: nomifun_agent_contracts::capability_surface_declarations(
            ["desktop"],
            [CapabilityConsumer::Agent],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({"type":"object"})),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from("example.csv.read.invoke"),
                input_schema: input_ref.clone(),
                output_schema: output_ref.clone(),
                effect_class: EffectClass::ReadLocal,
                presentation: nomifun_agent_contracts::ToolPresentationKind::FunctionTool,
            }],
            ..Default::default()
        },
    };
    PluginPackageArtifactV1::new(
        ArtifactId::from(Uuid::now_v7().to_string()),
        PluginPackageV1Manifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            build_profile: JavaScriptBuildProfile::PluginPackageV1,
            build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            package: PackageManifest {
                schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
                host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                package_id: package.id,
                package_version: package.version,
                display: LocalizedMetadata {
                    name: "CSV Plugin".into(),
                    description: "CSV capability package.".into(),
                    localized_names: BTreeMap::new(),
                    localized_descriptions: BTreeMap::new(),
                },
                package_dependencies: Vec::new(),
                requires_runtime_features: Vec::new(),
                config_schema,
                provides_services: Vec::new(),
                requires_services: Vec::new(),
                entrypoint: JavaScriptEntrypointMetadata {
                    normalized_relative_path: "main.mjs".into(),
                    module_digest: sha256(main),
                    host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                    sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
                }
                .into(),
                contributions: PackageContributions {
                    capabilities: vec![capability],
                    ..Default::default()
            },
        },
        schemas: BTreeMap::from([
            (input_ref, input_schema),
            (output_ref, output_schema),
        ]),
        supported_targets: BTreeSet::from([RuntimeTarget::from(
                "x86_64-pc-windows-msvc",
            )]),
            minimum_node_major: MINIMUM_NODE_MAJOR,
            dependency_lock_digest: DigestHex::from("b".repeat(64)),
            credential_slots: vec![CredentialSlotDeclaration {
                slot_key: CredentialSlotKey::from("api_key"),
                kind: CredentialSlotKind::SecretText,
                display_name: "API key".into(),
                required: false,
            }],
        },
        vec![ArtifactFileDigest {
            normalized_relative_path: "main.mjs".into(),
            digest: sha256(main),
            size_bytes: main.len() as u64,
        }],
    )
    .unwrap()
}

fn imported(artifact: PluginPackageArtifactV1, root: &Path) -> ImportedPluginArtifact {
    ImportedPluginArtifact {
        artifact,
        managed_relative_path: "plugin-artifacts/artifact".into(),
        package_root: root.join("artifact/package"),
        already_present: false,
    }
}

fn artifact_row(value: &PluginPackageArtifactV1, path: &str) -> PluginArtifactRow {
    PluginArtifactRow {
        id: 1,
        artifact_id: value.artifact_id.as_ref().to_owned(),
        artifact_digest: value.artifact_digest.as_ref().to_owned(),
        package_id: value.manifest.payload.package.package_id.as_ref().to_owned(),
        package_version: value
            .manifest
            .payload
            .package
            .package_version
            .as_ref()
            .to_owned(),
        manifest_digest: value.manifest.payload_digest.as_ref().to_owned(),
        manifest_json: serde_json::to_string(&value.manifest.payload).unwrap(),
        managed_path: path.into(),
        created_at: 1,
    }
}

fn project_row(
    project_id: &str,
    owner: &str,
    source: Option<&str>,
    linked_mount_id: Option<&str>,
) -> PluginProjectRow {
    PluginProjectRow {
        id: 1,
        project_id: project_id.into(),
        owner_user_id: owner.into(),
        package_id: "example.csv".into(),
        display_name: "CSV Tools".into(),
        description: "Read CSV files.".into(),
        managed_source_path: source.map(str::to_owned),
        source_head_digest: source.map(|_| "c".repeat(64)),
        dependency_lock_digest: source.map(|_| "d".repeat(64)),
        build_generation: if source.is_some() { 1 } else { 0 },
        linked_mount_id: linked_mount_id.map(str::to_owned),
        ready_candidate_id: None,
        created_at: 1,
        updated_at: 7,
    }
}

fn mount_row(
    mount_id: &str,
    package_id: &str,
    current: Option<String>,
    revision: i64,
) -> PluginMountRow {
    PluginMountRow {
        id: 1,
        mount_id: mount_id.into(),
        package_id: package_id.into(),
        current_artifact_digest: current,
        previous_artifact_digest: None,
        current_revision_id: None,
        previous_revision_id: None,
        enabled: true,
        retained: false,
        delete_pending: false,
        revision,
        config_json: "{}".into(),
        config_schema_digest: Some(config_schema_digest()),
        config_revision: 1,
        credential_bindings_revision: 0,
        data_dir_path: "C:\\data\\plugin".into(),
        last_error: None,
        created_at: 1,
        updated_at: 2,
    }
}

fn candidate_row(
    project_id: &str,
    artifact: &PluginPackageArtifactV1,
    base: Option<String>,
    generation: i64,
) -> PluginReadyCandidateRow {
    let diff = contract_diff();
    PluginReadyCandidateRow {
        id: 1,
        candidate_id: Uuid::now_v7().to_string(),
        project_id: project_id.into(),
        candidate_digest: "e".repeat(64),
        origin_kind: "build".into(),
        artifact_id: artifact.artifact_id.as_ref().to_owned(),
        artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
        target_package_id: artifact
            .manifest
            .payload
            .package
            .package_id
            .as_ref()
            .to_owned(),
        target_package_version: artifact
            .manifest
            .payload
            .package
            .package_version
            .as_ref()
            .to_owned(),
        target_manifest_digest: artifact.manifest.payload_digest.as_ref().to_owned(),
        base_target_digest: base,
        source_snapshot_digest: Some("c".repeat(64)),
        dependency_lock_digest: Some("d".repeat(64)),
        contract_diff_json: serde_json::to_string(&diff).unwrap(),
        origin_operation_id: Uuid::now_v7().to_string(),
        build_generation: generation,
        created_at: 8,
    }
}

fn contract_diff() -> PluginContractDiff {
    PluginContractDiff {
        compatibility: PluginCompatibility::Compatible,
        changes: BTreeSet::from([PluginContractChangeKind::ArtifactBytes]),
        affected_consumer_locks: Vec::new(),
    }
}

fn config_schema_digest() -> String {
    nomifun_agent_contracts::digest_payload(&StrictJsonValue(json!({"type":"object"})))
        .unwrap()
        .as_ref()
        .to_owned()
}

fn runtime() -> NodeRuntimeFingerprint {
    NodeRuntimeFingerprint {
        runtime_installation_id: "node-24".into(),
        source_kind: NodeRuntimeSourceKind::Managed,
        node_version: "24.8.0".into(),
        node_major: nomifun_agent_contracts::RECOMMENDED_NODE_LTS_MAJOR,
        runtime_target: "x86_64-pc-windows-msvc".into(),
        executable_digest: "f".repeat(64).into(),
        javascript_host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
        javascript_sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
    }
}

fn no_builder() -> Arc<dyn PluginBuildExecutor> {
    Arc::new(nomifun_plugin_service::UnconfiguredPluginBuildExecutor)
}

#[tokio::test]
async fn create_project_persists_the_exact_scaffold_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let service = service(
        repo,
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        &temp,
    );
    let detail = service
        .create_project(CreateProjectInput {
            owner_user_id: "user-1".into(),
            request: CreatePluginProjectRequest {
                expected_library_revision: 0,
                package_id: "example.csv".into(),
                package_version: "0.1.0".into(),
                display_name: "CSV Tools".into(),
                description: "Read CSV files.".into(),
                language: nomifun_api_types::PluginProjectLanguageDto::TypeScript,
                linked_mount_id: None,
                expected_linked_mount_revision: None,
                expected_linked_target_digest: None,
            },
        })
        .await
        .unwrap();
    assert_eq!(
        detail.summary.source_state,
        PluginProjectSourceStateDto::Editable
    );
    assert_eq!(detail.summary.display_name, "CSV Tools");
    assert_eq!(
        detail.summary.description.as_deref(),
        Some("Read CSV files.")
    );
    assert_eq!(detail.source_snapshot_digest.unwrap(), "a".repeat(64));
    assert_eq!(detail.dependency_lock_digest.unwrap(), "b".repeat(64));
}

#[tokio::test]
async fn project_delete_removes_source_and_project_but_not_through_delete_data() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let source_store = Arc::new(FakeSourceStore::default());
    let service = service_with_source_store(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        source_store.clone(),
    );
    let detail = service
        .create_project(CreateProjectInput {
            owner_user_id: "user-1".into(),
            request: CreatePluginProjectRequest {
                expected_library_revision: 0,
                package_id: "example.delete".into(),
                package_version: "0.1.0".into(),
                display_name: "Delete Me".into(),
                description: "Project deletion fixture.".into(),
                language: nomifun_api_types::PluginProjectLanguageDto::JavaScript,
                linked_mount_id: None,
                expected_linked_mount_revision: None,
                expected_linked_target_digest: None,
            },
        })
        .await
        .unwrap();
    let project_id = detail.summary.project_id.clone();
    assert!(
        service
            .delete_project(
                "user-1",
                DeletePluginProjectRequest {
                    project_id: project_id.clone(),
                    expected_project_revision: detail.summary.project_revision,
                    expected_build_generation: detail.summary.build_generation,
                    expected_ready_candidate_id: None,
                    expected_ready_candidate_digest: None,
                },
            )
            .await
            .unwrap()
    );
    assert!(repo.get_project(&project_id).await.unwrap().is_none());
    assert_eq!(source_store.deleted.lock().await.as_slice(), &[project_id]);
    drop(temp);
}

#[tokio::test]
async fn project_delete_reports_reconcile_required_after_authoritative_db_commit() {
    let repo = Arc::new(FakeRepository::default());
    let source_store = Arc::new(FakeSourceStore {
        deleted: Mutex::new(Vec::new()),
        fail_delete: true,
    });
    let service = service_with_source_store(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        source_store,
    );
    let detail = service
        .create_project(CreateProjectInput {
            owner_user_id: "user-1".into(),
            request: CreatePluginProjectRequest {
                expected_library_revision: 0,
                package_id: "example.reconcile".into(),
                package_version: "0.1.0".into(),
                display_name: "Reconcile".into(),
                description: "Source cleanup failure fixture.".into(),
                language: nomifun_api_types::PluginProjectLanguageDto::TypeScript,
                linked_mount_id: None,
                expected_linked_mount_revision: None,
                expected_linked_target_digest: None,
            },
        })
        .await
        .unwrap();
    let project_id = detail.summary.project_id.clone();
    let error = service
        .delete_project(
            "user-1",
            DeletePluginProjectRequest {
                project_id: project_id.clone(),
                expected_project_revision: detail.summary.project_revision,
                expected_build_generation: detail.summary.build_generation,
                expected_ready_candidate_id: None,
                expected_ready_candidate_digest: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), ERR_RECONCILE_REQUIRED);
    assert!(repo.get_project(&project_id).await.unwrap().is_none());
}

#[tokio::test]
async fn source_less_import_creates_read_only_candidate_and_terminal_operation() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let package = artifact(b"export const version = 1;\n", "1.0.0");
    let digest = package.artifact_digest.as_ref().to_owned();
    let service = service(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(vec![imported(
            package,
            temp.path(),
        )])),
        no_builder(),
        &temp,
    );
    let detail = service
        .import_prebuilt(
            "user-1",
            ImportPluginRequest {
                expected_library_revision: 0,
                import_kind: PluginImportKindDto::PrebuiltArtifact,
                source_path: "fixture.zip".into(),
                expected_bundle_or_artifact_digest: digest,
                target_project_id: None,
                expected_project_revision: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        detail.summary.source_state,
        PluginProjectSourceStateDto::RuntimeOnly
    );
    assert_eq!(detail.ready.unwrap().origin, PluginCandidateOriginDto::Import);
    assert_eq!(repo.snapshot().await.operations[0].state, "succeeded");
}

#[tokio::test]
async fn same_version_different_digest_remains_two_immutable_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let first = artifact(b"export const version = 1;\n", "1.0.0");
    let second = artifact(b"export const version = 2;\n", "1.0.0");
    let first_digest = first.artifact_digest.as_ref().to_owned();
    let second_digest = second.artifact_digest.as_ref().to_owned();
    let service = service(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(vec![
            imported(first, temp.path()),
            imported(second, temp.path()),
        ])),
        no_builder(),
        &temp,
    );
    for (revision, digest) in [(0, first_digest.clone()), (1, second_digest.clone())] {
        service
            .import_prebuilt(
                "user-1",
                ImportPluginRequest {
                    expected_library_revision: revision,
                    import_kind: PluginImportKindDto::PrebuiltArtifact,
                    source_path: "fixture.zip".into(),
                    expected_bundle_or_artifact_digest: digest,
                    target_project_id: None,
                    expected_project_revision: None,
                },
            )
            .await
            .unwrap();
    }
    let snapshot = repo.snapshot().await;
    assert_eq!(snapshot.artifacts.len(), 2);
    assert_ne!(first_digest, second_digest);
    assert!(snapshot
        .artifacts
        .iter()
        .all(|artifact| artifact.package_version == "1.0.0"));
}

#[tokio::test]
async fn configure_rejects_stale_cas_and_exposes_only_credential_reference() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let package = artifact(b"export const plugin = 1;\n", "1.0.0");
    let digest = package.artifact_digest.as_ref().to_owned();
    repo.insert_artifact(artifact_row(&package, "artifact")).await;
    repo.insert_mount(mount_row("mount-1", "example.csv", Some(digest.clone()), 1))
        .await;
    let service = service(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        &temp,
    );
    let mut bindings = BTreeMap::new();
    bindings.insert("api_key".into(), Some("credential://stepfun".into()));
    let stale = service
        .configure(ConfigureInput {
            owner_user_id: "user-1".into(),
            request: ConfigurePluginRequest {
                mount_id: "mount-1".into(),
                expected_mount_revision: 1,
                expected_current_target_digest: digest.clone(),
                expected_config_revision: 0,
                expected_schema_digest: config_schema_digest(),
                values: json!({"mode":"fast"}),
                credential_bindings: bindings.clone(),
                expected_credential_bindings_revision: 0,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(stale.code(), ERR_STALE);

    let detail = service
        .configure(ConfigureInput {
            owner_user_id: "user-1".into(),
            request: ConfigurePluginRequest {
                mount_id: "mount-1".into(),
                expected_mount_revision: 1,
                expected_current_target_digest: digest,
                expected_config_revision: 1,
                expected_schema_digest: config_schema_digest(),
                values: json!({"mode":"fast"}),
                credential_bindings: bindings,
                expected_credential_bindings_revision: 0,
            },
        })
        .await
        .unwrap();
    let wire = serde_json::to_string(&detail).unwrap();
    assert!(wire.contains("credential://stepfun"));
    assert!(!wire.contains("actual-secret-value"));
    assert_eq!(detail.credential_slots[0].credential_id.as_deref(), Some("credential://stepfun"));
}

#[tokio::test]
async fn uninstall_retains_data_until_explicit_delete_data() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let package = artifact(b"export const plugin = 1;\n", "1.0.0");
    let digest = package.artifact_digest.as_ref().to_owned();
    repo.insert_artifact(artifact_row(&package, "artifact")).await;
    repo.insert_mount(mount_row("mount-1", "example.csv", Some(digest.clone()), 1))
        .await;
    let service = service(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        &temp,
    );
    let retained = service
        .uninstall(
            "user-1",
            UninstallPluginRequest {
                mount_id: "mount-1".into(),
                expected_mount_revision: 1,
                expected_current_target_digest: digest,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        retained.summary.lifecycle,
        PluginLifecycleDto::UninstalledDataRetained
    );
    assert!(repo.get_mount("mount-1").await.unwrap().is_some());
    assert!(
        service
            .delete_data(
                "user-1",
                DeletePluginDataRequest {
                    mount_id: "mount-1".into(),
                    expected_mount_revision: 2,
                    expected_lifecycle: PluginLifecycleDto::UninstalledDataRetained,
                    expected_data_revision: 2,
                },
            )
            .await
            .unwrap()
    );
    assert!(repo.get_mount("mount-1").await.unwrap().is_none());
}

#[tokio::test]
async fn candidate_stale_apply_then_restore_use_exact_digest_cas() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let old = artifact(b"export const plugin = 1;\n", "1.0.0");
    let new = artifact(b"export const plugin = 2;\n", "1.1.0");
    let old_digest = old.artifact_digest.as_ref().to_owned();
    let new_digest = new.artifact_digest.as_ref().to_owned();
    let project = project_row("project-1", "user-1", Some("C:\\source"), Some("mount-1"));
    repo.insert_project(project.clone()).await;
    repo.insert_artifact(artifact_row(&old, "old")).await;
    repo.insert_artifact(artifact_row(&new, "new")).await;
    repo.insert_mount(mount_row(
        "mount-1",
        "example.csv",
        Some(old_digest.clone()),
        1,
    ))
    .await;
    let candidate = candidate_row(
        "project-1",
        &new,
        Some(old_digest.clone()),
        project.build_generation,
    );
    let candidate_id = candidate.candidate_id.clone();
    let candidate_digest = candidate.candidate_digest.clone();
    repo.insert_candidate(candidate.clone()).await;
    repo.record_test_receipt(&RecordPluginCandidateTestReceiptParams {
        receipt_id: Uuid::now_v7().to_string(),
        candidate_id: candidate_id.clone(),
        candidate_digest: candidate_digest.clone(),
        artifact_id: candidate.artifact_id.clone(),
        artifact_digest: candidate.artifact_digest.clone(),
        receipt_digest: "a".repeat(64),
        runtime_fingerprint_digest: "b".repeat(64),
        receipt: json!({"outcome":"passed"}),
        tested_at: 10,
    })
    .await
    .unwrap();
    let service = service(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        &temp,
    );
    let stale = service
        .apply_candidate(
            "user-1",
            ApplyPluginCandidateRequest {
                project_id: "project-1".into(),
                expected_project_revision: 7,
                expected_build_generation: 1,
                candidate_id: candidate_id.clone(),
                expected_candidate_digest: "0".repeat(64),
                target: ApplyPluginTargetDto::ExistingMount {
                    mount_id: "mount-1".into(),
                    expected_mount_revision: 1,
                    expected_current_target_digest: old_digest.clone(),
                },
                allow_breaking: false,
                acknowledge_test_warning: false,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(stale.code(), ERR_STALE);

    let applied = service
        .apply_candidate(
            "user-1",
            ApplyPluginCandidateRequest {
                project_id: "project-1".into(),
                expected_project_revision: 7,
                expected_build_generation: 1,
                candidate_id,
                expected_candidate_digest: candidate_digest,
                target: ApplyPluginTargetDto::ExistingMount {
                    mount_id: "mount-1".into(),
                    expected_mount_revision: 1,
                    expected_current_target_digest: old_digest.clone(),
                },
                allow_breaking: false,
                acknowledge_test_warning: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        applied.summary.current.unwrap().artifact_digest,
        new_digest
    );
    let restored = service
        .restore(
            "user-1",
            RestorePluginPreviousRequest {
                mount_id: "mount-1".into(),
                expected_mount_revision: 2,
                expected_current_target_digest: new.artifact_digest.as_ref().to_owned(),
                expected_previous_target_digest: old_digest.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        restored.summary.current.unwrap().artifact_digest,
        old_digest
    );
}

#[tokio::test]
async fn build_and_candidate_test_bind_exact_generation_and_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let project = project_row("project-1", "user-1", Some("C:\\source"), None);
    repo.insert_project(project.clone()).await;
    let built = artifact(b"export const built = 1;\n", "1.0.0");
    let builder = Arc::new(FakeBuilder {
        output: Mutex::new(Some(BuildOutput {
            artifact: built,
            managed_relative_path: "plugin-artifacts/built".into(),
            source_snapshot_digest: "c".repeat(64),
            dependency_lock_digest: "d".repeat(64),
        })),
    });
    let service = service(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        builder,
        &temp,
    );
    let built_detail = service
        .build(
            "user-1",
            BuildPluginProjectRequest {
                project_id: "project-1".into(),
                expected_project_revision: 7,
                expected_build_generation: 1,
                expected_source_snapshot_digest: "c".repeat(64),
                expected_dependency_lock_digest: "d".repeat(64),
            },
        )
        .await
        .unwrap();
    let ready = built_detail.ready.unwrap();
    let tested = service
        .test_candidate(
            "user-1",
            TestPluginCandidateRequest {
                project_id: "project-1".into(),
                expected_project_revision: 7,
                expected_build_generation: 1,
                candidate_id: ready.candidate.candidate_id,
                expected_candidate_digest: ready.candidate.candidate_digest,
                expected_config_revision: 0,
                expected_credential_bindings_revision: 0,
                resolved_test_input_digest: "9".repeat(64),
            },
        )
        .await
        .unwrap();
    let tested_ready = tested.ready.unwrap();
    assert_eq!(
        tested_ready.test.status,
        nomifun_api_types::PluginCandidateTestStatusDto::Passed
    );
    assert_eq!(
        tested_ready
            .test
            .runtime
            .as_ref()
            .map(|runtime| runtime.node_version.as_str()),
        Some("24.8.0")
    );
    assert_eq!(
        tested_ready.test.resolved_test_input_digest.as_deref(),
        Some("9".repeat(64).as_str())
    );
    assert_eq!(repo.snapshot().await.receipts.len(), 1);
}

#[tokio::test]
async fn build_diff_is_derived_from_the_exact_linked_mount_contract() {
    for (built, expected_compatibility, expected_change) in [
        (
            artifact(b"export const built = 2;\n", "1.0.0"),
            nomifun_api_types::PluginCompatibilityDto::Compatible,
            None,
        ),
        (
            artifact_with_config_schema(
                b"export const built = 3;\n",
                "1.0.0",
                StrictJsonValue(json!({
                    "type": "object",
                    "properties": {"mode": {"type": "string"}}
                })),
            ),
            nomifun_api_types::PluginCompatibilityDto::Breaking,
            Some("configschema"),
        ),
    ] {
        let repo = Arc::new(FakeRepository::default());
        let current = artifact(b"export const built = 1;\n", "1.0.0");
        let current_digest = current.artifact_digest.as_ref().to_owned();
        repo.insert_project(project_row(
            "project-1",
            "user-1",
            Some("C:\\source"),
            Some("mount-1"),
        ))
        .await;
        repo.insert_artifact(artifact_row(&current, "current")).await;
        repo.insert_mount(mount_row(
            "mount-1",
            "example.csv",
            Some(current_digest.clone()),
            1,
        ))
        .await;
        let builder = Arc::new(FakeBuilder {
            output: Mutex::new(Some(BuildOutput {
                artifact: built,
                managed_relative_path: "plugin-artifacts/built".into(),
                source_snapshot_digest: "c".repeat(64),
                dependency_lock_digest: "d".repeat(64),
            })),
        });
        let service = service(
            Arc::clone(&repo),
            Arc::new(QueueArtifactStore::new(Vec::new())),
            builder,
            &tempfile::tempdir().unwrap(),
        );

        let detail = service
            .build(
                "user-1",
                BuildPluginProjectRequest {
                    project_id: "project-1".into(),
                    expected_project_revision: 7,
                    expected_build_generation: 1,
                    expected_source_snapshot_digest: "c".repeat(64),
                    expected_dependency_lock_digest: "d".repeat(64),
                },
            )
            .await
            .unwrap();
        let ready = detail.ready.expect("Build publishes one Ready Candidate");
        assert_eq!(ready.base_target_digest.as_deref(), Some(current_digest.as_str()));
        assert_eq!(ready.impact.compatibility, expected_compatibility);
        if let Some(change) = expected_change {
            assert!(ready.impact.changed_contracts.iter().any(|value| value == change));
        } else {
            assert_eq!(ready.impact.changed_contracts, vec!["artifactbytes"]);
        }
    }
}

#[tokio::test]
async fn operation_cancel_uses_the_product_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    repo.insert_project(project_row("project-1", "user-1", None, None))
        .await;
    let operation = repo
        .start_operation(&StartProductOperationParams {
            operation_id: "operation-1".into(),
            kind: nomifun_db::ProductOperationKind::Import,
            owner_kind: "plugin_project".into(),
            owner_id: "project-1".into(),
            progress_percent: Some(1),
            bounded_log_tail: Vec::new(),
            started_at_ms: 1,
        })
        .await
        .unwrap();
    let service = service(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        &temp,
    );
    assert_eq!(service.list_operations("user-1").await.unwrap().len(), 1);
    assert_eq!(
        service
            .get_project("user-1", "project-1")
            .await
            .unwrap()
            .active_operation
            .as_ref()
            .map(|operation| operation.operation_id.as_str()),
        Some("operation-1")
    );
    assert_eq!(
        service
            .get_operation("user-1", &operation.operation_id)
            .await
            .unwrap()
            .summary
            .operation_revision,
        1
    );
    let canceled = service
        .cancel_operation("user-1", &operation.operation_id, 1)
        .await
        .unwrap();
    assert_eq!(
        canceled.state,
        nomifun_api_types::DurableOperationStateDto::Canceled
    );
}

#[tokio::test]
async fn operation_cancel_accepts_a_builder_that_already_persisted_canceled() {
    let repo = Arc::new(FakeRepository::default());
    repo.insert_project(project_row("project-1", "user-1", None, None))
        .await;
    let operation = repo
        .start_operation(&StartProductOperationParams {
            operation_id: "operation-cancel-race".into(),
            kind: nomifun_db::ProductOperationKind::Build,
            owner_kind: "plugin_project".into(),
            owner_id: "project-1".into(),
            progress_percent: Some(1),
            bounded_log_tail: Vec::new(),
            started_at_ms: 1,
        })
        .await
        .unwrap();
    let service = service_with_operation_cancellation(
        Arc::clone(&repo),
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        Arc::new(FakeSourceStore::default()),
        Arc::new(TerminalizingOperationCancellation {
            repo,
            owner_user_id: "user-1".into(),
        }),
    );

    let canceled = service
        .cancel_operation("user-1", &operation.operation_id, 1)
        .await
        .unwrap();
    assert_eq!(
        canceled.state,
        nomifun_api_types::DurableOperationStateDto::Canceled
    );
    assert_eq!(canceled.operation_revision, 2);
}

#[tokio::test]
async fn auto_apply_requires_every_explicit_eligibility_predicate() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Arc::new(FakeRepository::default());
    let service = service(
        repo,
        Arc::new(QueueArtifactStore::new(Vec::new())),
        no_builder(),
        &temp,
    );
    let request = ApplyPluginCandidateRequest {
        project_id: "missing".into(),
        expected_project_revision: 1,
        expected_build_generation: 1,
        candidate_id: "missing".into(),
        expected_candidate_digest: "0".repeat(64),
        target: ApplyPluginTargetDto::InitialInstall {
            expected_library_revision: 0,
        },
        allow_breaking: false,
        acknowledge_test_warning: false,
    };
    let eligibility = PluginAutoApplyEligibility {
        standing_authorization_matches: false,
        linked_mount_matches: false,
        base_target_matches: false,
        managed_source_matches_project_head: false,
        matching_local_test_passed: false,
        contribution_contracts_unchanged: false,
        config_schema_unchanged: false,
        credential_slots_unchanged: false,
        resource_effect_contracts_unchanged: false,
        runtime_requirement_unchanged: false,
        dependency_lock_unchanged: false,
        host_sdk_contract_unchanged: false,
        supported_targets_unchanged: false,
        static_validation_passed: false,
        no_unknown_facts: false,
    };
    assert!(
        service
            .auto_apply_candidate("user-1", request, 1, eligibility)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn owner_mutation_coordinator_serializes_the_real_mount_scope() {
    let coordinator = OwnerMutationCoordinator::new();
    let scope = PluginOwnerMutationScope::mount("mount-1".into()).unwrap();
    let first = coordinator.acquire(&scope).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            coordinator.acquire(&scope),
        )
        .await
        .is_err()
    );
    drop(first);
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        coordinator.acquire(&scope),
    )
    .await
    .unwrap()
    .unwrap();
}
