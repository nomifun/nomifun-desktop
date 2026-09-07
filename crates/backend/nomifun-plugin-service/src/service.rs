use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_agent_contracts::{
    AffectedConsumerKind, CandidateTestReceipt, CapabilityConsumer,
    PluginAutoApplyEligibility,
    PluginCompatibility, PluginContractChangeKind, PluginContractDiff, PluginHostCommitFence,
    PluginMountId, PluginPackageV1Manifest, PluginProjectId, PluginReadyCandidate,
    PluginSourceLineage, PluginTargetRef,
    PLUGIN_PACKAGE_PROFILE_VERSION, digest_payload,
};
use nomifun_api_types::{
    ApplyPluginCandidateRequest, ApplyPluginTargetDto, ConfigurePluginRequest,
    CredentialBindingStatusDto, CredentialSlotBindingDto, DurableOperationDetailDto,
    DurableOperationKindDto, DurableOperationOwnerDto, DurableOperationStateDto,
    DurableOperationSummaryDto, JavascriptRuntimeRefDto,
    PluginAffectedConsumerDto, PluginApplyModeDto, PluginCandidateImpactDto,
    PluginCandidateOriginDto, PluginCandidateRefDto, PluginCandidateTestDto,
    PluginCandidateTestStatusDto, PluginCapabilityContributionDto, PluginCompatibilityDto,
    PluginConfigSchemaDto, PluginConfigStateDto, PluginConsumerAvailabilityDto,
    PluginConsumerAvailabilityStatusDto, PluginConsumerSurfaceDto, PluginDetailDto,
    PluginLibraryResponseDto, PluginLifecycleDto, PluginProjectDetailDto,
    PluginProjectSourceStateDto, PluginProjectSummaryDto, PluginReadyCandidateDto,
    PluginSummaryDto, PluginTargetRefDto,
};
use nomifun_db::{
    ApplyPluginCandidateParams, CreatePluginArtifactParams, CreatePluginProjectParams,
    DeletePluginProjectParams,
    FinishProductOperationParams, ListPluginCredentialBindingsParams, PluginArtifactRow,
    PluginCandidateOrigin as DbCandidateOrigin, PluginCandidateTestReceiptRow,
    PluginCredentialBindingInput, PluginCredentialBindingSnapshot, PluginMountRow,
    PluginProjectRow, PluginReadyCandidateRow, ProductOperationKind, ProductOperationRow,
    ProductOperationState, RecordPluginCandidateTestReceiptParams,
    RecordPluginReadyCandidateParams, ReplacePluginCredentialBindingsParams,
    RestorePluginMountParams, StartProductOperationParams, UninstallPluginMountParams,
    UpdatePluginMountConfigParams,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use nomifun_plugin_platform::{
    OwnerMutationCoordinator, OwnerMutationGuard, PluginOwnerMutationScope,
};

use crate::PluginServiceError;
use crate::repository::{
    PluginArtifactStorePort, PluginBuildExecutor, PluginCandidateTestExecutor,
    PluginHostCoordinator, PluginMountDataStore, PluginOperationCancellation,
    PluginRegistryPublisher, PluginRepository, PluginSourceStorePort,
};
use crate::types::{
    ApplyAuthorization, BuildRequest, ConfigureInput, CreateProjectInput, DeleteDataRequest,
    EnableRequest, ImportRequest, LinkPluginProjectParams, PluginInventory,
    PluginServicePaths, RetryRequest, RestoreRequest, TestRequest, UninstallRequest,
};

pub struct PluginApplicationService {
    repository: Arc<dyn PluginRepository>,
    artifacts: Arc<dyn PluginArtifactStorePort>,
    host: Arc<dyn PluginHostCoordinator>,
    registry: Arc<dyn PluginRegistryPublisher>,
    mutation_coordinator: Arc<OwnerMutationCoordinator>,
    builder: Arc<dyn PluginBuildExecutor>,
    tester: Arc<dyn PluginCandidateTestExecutor>,
    operation_cancellation: Arc<dyn PluginOperationCancellation>,
    source_store: Arc<dyn PluginSourceStorePort>,
    data_store: Arc<dyn PluginMountDataStore>,
    paths: PluginServicePaths,
}

pub struct PluginServiceDependencies {
    pub repository: Arc<dyn PluginRepository>,
    pub artifacts: Arc<dyn PluginArtifactStorePort>,
    pub host: Arc<dyn PluginHostCoordinator>,
    pub registry: Arc<dyn PluginRegistryPublisher>,
    pub mutation_coordinator: Arc<OwnerMutationCoordinator>,
    pub builder: Arc<dyn PluginBuildExecutor>,
    pub tester: Arc<dyn PluginCandidateTestExecutor>,
    pub operation_cancellation: Arc<dyn PluginOperationCancellation>,
    pub source_store: Arc<dyn PluginSourceStorePort>,
    pub data_store: Arc<dyn PluginMountDataStore>,
    pub paths: PluginServicePaths,
}

impl PluginApplicationService {
    pub fn new(dependencies: PluginServiceDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            artifacts: dependencies.artifacts,
            host: dependencies.host,
            registry: dependencies.registry,
            mutation_coordinator: dependencies.mutation_coordinator,
            builder: dependencies.builder,
            tester: dependencies.tester,
            operation_cancellation: dependencies.operation_cancellation,
            source_store: dependencies.source_store,
            data_store: dependencies.data_store,
            paths: dependencies.paths,
        }
    }

    async fn reconcile_committed_mount(
        &self,
        owner_user_id: &str,
        mount: &PluginMountRow,
    ) -> Result<(), PluginServiceError> {
        self.registry
            .reconcile_mount(owner_user_id, mount)
            .await
            .map_err(|error| {
                PluginServiceError::reconcile_required(format!(
                    "Mount {} commit is authoritative; Registry reconciliation failed: {error}",
                    mount.mount_id
                ))
            })
    }

    async fn owned_project(
        &self,
        owner_user_id: &str,
        project_id: &str,
    ) -> Result<PluginProjectRow, PluginServiceError> {
        let project = self
            .repository
            .get_project(project_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("project {project_id}")))?;
        require_project_owner(&project, owner_user_id)?;
        Ok(project)
    }

    async fn owned_mount(
        &self,
        owner_user_id: &str,
        mount_id: &str,
    ) -> Result<(PluginMountRow, Option<PluginProjectRow>), PluginServiceError> {
        let mount = self
            .repository
            .get_mount(mount_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("mount {mount_id}")))?;
        let actual_owner = self
            .repository
            .mount_owner_user_id(mount_id)
            .await?
            .ok_or_else(|| {
                PluginServiceError::integration(format!(
                    "Mount {mount_id} has no durable owner"
                ))
            })?;
        if actual_owner != owner_user_id {
            return Err(PluginServiceError::forbidden(
                "Mount belongs to another owner",
            ));
        }
        let project = self.repository.get_project_for_mount(mount_id).await?;
        if let Some(project) = &project {
            require_project_owner(project, owner_user_id)?;
        }
        Ok((mount, project))
    }

    async fn verified_artifact(
        &self,
        artifact_digest: &str,
    ) -> Result<Option<PluginArtifactRow>, PluginServiceError> {
        let artifact = self.repository.get_artifact(artifact_digest).await?;
        if let Some(artifact) = &artifact {
            self.artifacts.verify(artifact).await?;
        }
        Ok(artifact)
    }

    async fn project_guard(
        &self,
        owner_user_id: &str,
        project_id: &str,
    ) -> Result<OwnerMutationGuard, PluginServiceError> {
        let project = self.owned_project(owner_user_id, project_id).await?;
        let scope = match project.linked_mount_id {
            Some(mount_id) => PluginOwnerMutationScope::linked(
                PluginProjectId::from(project.project_id),
                PluginMountId::from(mount_id),
            )?,
            None => PluginOwnerMutationScope::project(PluginProjectId::from(project.project_id))?,
        };
        Ok(self.mutation_coordinator.acquire(&scope).await?)
    }

    async fn mount_guard(
        &self,
        owner_user_id: &str,
        mount_id: &str,
    ) -> Result<OwnerMutationGuard, PluginServiceError> {
        let (_, project) = self.owned_mount(owner_user_id, mount_id).await?;
        let scope = match project {
            Some(project) => PluginOwnerMutationScope::linked(
                PluginProjectId::from(project.project_id),
                PluginMountId::from(mount_id.to_owned()),
            )?,
            None => PluginOwnerMutationScope::mount(PluginMountId::from(mount_id.to_owned()))?,
        };
        Ok(self.mutation_coordinator.acquire(&scope).await?)
    }

    pub async fn list_library(
        &self,
        owner_user_id: &str,
    ) -> Result<PluginLibraryResponseDto, PluginServiceError> {
        let inventory = self.repository.inventory(owner_user_id).await?;
        Ok(project_library(&inventory))
    }

    pub async fn get_project(
        &self,
        owner_user_id: &str,
        project_id: &str,
    ) -> Result<PluginProjectDetailDto, PluginServiceError> {
        let project = self.owned_project(owner_user_id, project_id).await?;
        let candidate = self.repository.get_candidate(project_id).await?;
        let receipt = match candidate.as_ref() {
            Some(candidate) => self
                .repository
                .get_test_receipt(&candidate.candidate_id)
                .await?,
            None => None,
        };
        let operation = self
            .repository
            .list_operations(owner_user_id)
            .await?
            .into_iter()
            .find(|operation| {
                operation.owner_kind == "plugin_project"
                    && operation.owner_id == project.project_id
                    && operation.state == "running"
            });
        project_detail_with_state(
            &project,
            candidate.as_ref(),
            receipt.as_ref(),
            operation.as_ref(),
        )
    }

    pub async fn delete_project(
        &self,
        owner_user_id: &str,
        request: nomifun_api_types::DeletePluginProjectRequest,
    ) -> Result<bool, PluginServiceError> {
        let _guard = self.project_guard(owner_user_id, &request.project_id).await?;
        let project = self.owned_project(owner_user_id, &request.project_id).await?;
        require_project_request_fresh(
            &project,
            request.expected_project_revision,
            request.expected_build_generation,
        )?;
        let candidate = self.repository.get_candidate(&project.project_id).await?;
        match (
            candidate.as_ref(),
            request.expected_ready_candidate_id.as_deref(),
            request.expected_ready_candidate_digest.as_deref(),
        ) {
            (None, None, None) => {}
            (Some(candidate), Some(expected_id), Some(expected_digest))
                if candidate.candidate_id == expected_id
                    && candidate.candidate_digest == expected_digest => {}
            _ => {
                return Err(PluginServiceError::stale(
                    "Ready Candidate changed before Project deletion",
                ));
            }
        }

        let deleted = self
            .repository
            .delete_project_cas(&DeletePluginProjectParams {
                project_id: project.project_id.clone(),
                owner_user_id: owner_user_id.to_owned(),
                expected_updated_at: project.updated_at,
                expected_generation: project.build_generation,
                expected_ready_candidate_id: request.expected_ready_candidate_id,
                expected_ready_candidate_digest: request.expected_ready_candidate_digest,
            })
            .await?;
        if deleted
            && project.managed_source_path.is_some()
            && let Err(error) = self
                .source_store
                .delete_project(owner_user_id, &project.project_id)
                .await
        {
            return Err(PluginServiceError::reconcile_required(format!(
                "Plugin Project deletion is committed; Source cleanup must be retried: {error}"
            )));
        }
        Ok(deleted)
    }

    pub async fn get_mount(
        &self,
        owner_user_id: &str,
        mount_id: &str,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        let (mount, project) = self.owned_mount(owner_user_id, mount_id).await?;
        let current_artifact = match mount.current_artifact_digest.as_deref() {
            Some(digest) => self.verified_artifact(digest).await?,
            None => None,
        };
        let previous_artifact = match mount.previous_artifact_digest.as_deref() {
            Some(digest) => self.verified_artifact(digest).await?,
            None => None,
        };
        let bindings = self
            .repository
            .list_credentials(&ListPluginCredentialBindingsParams {
                mount_id: mount.mount_id.clone(),
                expected_mount_revision: mount.revision,
                expected_current_artifact_digest: mount.current_artifact_digest.clone(),
            })
            .await?;
        let mut detail = mount_detail(
            &mount,
            current_artifact.as_ref(),
            previous_artifact.as_ref(),
            &bindings,
        )?;
        detail.summary.linked_project_id = project.map(|project| project.project_id);
        Ok(detail)
    }

    pub async fn create_project(
        &self,
        input: CreateProjectInput,
    ) -> Result<PluginProjectDetailDto, PluginServiceError> {
        let project_id = Uuid::now_v7().to_string();
        let scope = match input.request.linked_mount_id.as_deref() {
            Some(mount_id) => PluginOwnerMutationScope::linked(
                PluginProjectId::from(project_id.clone()),
                PluginMountId::from(mount_id.to_owned()),
            )?,
            None => PluginOwnerMutationScope::project(PluginProjectId::from(project_id.clone()))?,
        };
        let _guard = self.mutation_coordinator.acquire(&scope).await?;
        let inventory = self.repository.inventory(&input.owner_user_id).await?;
        if inventory.library_revision != input.request.expected_library_revision {
            return Err(PluginServiceError::stale("library revision changed"));
        }
        let link = if let Some(mount_id) = input.request.linked_mount_id.as_deref() {
            let expected_mount_revision = input
                .request
                .expected_linked_mount_revision
                .ok_or_else(|| PluginServiceError::invalid("linked Mount revision is required"))?;
            let expected_target_digest = input
                .request
                .expected_linked_target_digest
                .as_deref()
                .ok_or_else(|| {
                    PluginServiceError::invalid("linked Mount target digest is required")
                })?;
            let (mount, _) = self.owned_mount(&input.owner_user_id, mount_id).await?;
            if mount.package_id != input.request.package_id {
                return Err(PluginServiceError::conflict(
                    "project and linked Mount package identities differ",
                ));
            }
            require_mount_cas(&mount, expected_mount_revision, expected_target_digest)?;
            Some((
                mount_id.to_owned(),
                expected_mount_revision,
                expected_target_digest.to_owned(),
            ))
        } else {
            None
        };
        let source = self
            .source_store
            .create_project(
                &input.owner_user_id,
                &project_id,
                &input.request,
            )
            .await?;
        let created = self
            .repository
            .create_project(&CreatePluginProjectParams {
                project_id: project_id.clone(),
                owner_user_id: input.owner_user_id.clone(),
                package_id: input.request.package_id.clone(),
                display_name: input.request.display_name.clone(),
                description: input.request.description.clone(),
                managed_source_path: Some(source.managed_relative_path),
                source_head_digest: Some(source.source_snapshot_digest),
                dependency_lock_digest: Some(source.dependency_lock_digest),
                initial_build_generation: 1,
                created_at: now_ms(),
            })
            .await;
        let mut project = match created {
            Ok(project) => project,
            Err(error) => {
                if let Err(cleanup) = self
                    .source_store
                    .delete_project(&input.owner_user_id, &project_id)
                    .await
                {
                    return Err(PluginServiceError::integration(format!(
                        "Plugin Project database creation failed with {error}; Source cleanup failed with {cleanup}"
                    )));
                }
                return Err(error);
            }
        };
        if let Some((mount_id, expected_mount_revision, expected_target_digest)) = link {
            project = self
                .repository
                .link_project(&LinkPluginProjectParams {
                    owner_user_id: input.owner_user_id.clone(),
                    project_id: project.project_id.clone(),
                    expected_project_revision: project.updated_at as u64,
                    mount_id,
                    expected_mount_revision,
                    expected_target_digest,
                    updated_at: now_ms(),
                })
                .await?;
        }
        project_detail_with_state(&project, None, None, None)
    }

    pub async fn import_prebuilt(
        &self,
        owner_user_id: &str,
        request: ImportRequest,
    ) -> Result<PluginProjectDetailDto, PluginServiceError> {
        let reserved_project_id = request
            .target_project_id
            .is_none()
            .then(|| Uuid::now_v7().to_string());
        let _guard = match request.target_project_id.as_deref() {
            Some(project_id) => self.project_guard(owner_user_id, project_id).await?,
            None => {
                let scope = PluginOwnerMutationScope::project(PluginProjectId::from(
                    reserved_project_id
                        .as_ref()
                        .expect("new import reserves a project identity")
                        .clone(),
                ))?;
                self.mutation_coordinator.acquire(&scope).await?
            }
        };
        self.import_prebuilt_locked(owner_user_id, request, reserved_project_id)
            .await
    }

    async fn import_prebuilt_locked(
        &self,
        owner_user_id: &str,
        request: ImportRequest,
        reserved_project_id: Option<String>,
    ) -> Result<PluginProjectDetailDto, PluginServiceError> {
        if request.import_kind != nomifun_api_types::PluginImportKindDto::PrebuiltArtifact {
            return Err(PluginServiceError::invalid(
                "this boundary accepts only prebuilt directory or zip imports",
            ));
        }
        let source = std::path::PathBuf::from(&request.source_path);
        let imported = if source.is_dir() {
            self.artifacts.import_directory(&source).await?
        } else {
            self.artifacts.import_zip(&source).await?
        };
        if imported.artifact.artifact_digest.as_ref() != request.expected_bundle_or_artifact_digest
        {
            return Err(PluginServiceError::stale(
                "imported artifact digest differs from the approved digest",
            ));
        }

        let package = &imported.artifact.manifest.payload.package;
        let project = if let Some(project_id) = request.target_project_id.as_deref() {
            let project = self
                .repository
                .get_project(project_id)
                .await?
                .ok_or_else(|| PluginServiceError::not_found(format!("project {project_id}")))?;
            require_project_owner(&project, owner_user_id)?;
            if request.expected_project_revision != Some(project.updated_at as u64) {
                return Err(PluginServiceError::stale("project revision changed"));
            }
            if project.package_id != package.package_id.as_ref() {
                return Err(PluginServiceError::conflict(
                    "import package identity differs from the target project",
                ));
            }
            project
        } else {
            let inventory = self.repository.inventory(owner_user_id).await?;
            if inventory.library_revision != request.expected_library_revision {
                return Err(PluginServiceError::stale("library revision changed"));
            }
            let project_id = reserved_project_id.ok_or_else(|| {
                PluginServiceError::integration("new import project identity was not reserved")
            })?;
            self.repository
                .create_project(&CreatePluginProjectParams {
                    project_id,
                    owner_user_id: owner_user_id.to_owned(),
                    package_id: package.package_id.as_ref().to_owned(),
                    display_name: package.display.name.clone(),
                    description: package.display.description.clone(),
                    managed_source_path: None,
                    source_head_digest: None,
                    dependency_lock_digest: None,
                    initial_build_generation: 0,
                    created_at: now_ms(),
                })
                .await?
        };

        let operation_id = Uuid::now_v7().to_string();
        self.repository
            .start_operation(&StartProductOperationParams {
                operation_id: operation_id.clone(),
                kind: ProductOperationKind::Import,
                owner_kind: "plugin_project".into(),
                owner_id: project.project_id.clone(),
                progress_percent: Some(10),
                bounded_log_tail: vec!["prebuilt package accepted".into()],
                started_at_ms: now_ms(),
            })
            .await?;
        let artifact = self
            .repository
            .put_artifact(&CreatePluginArtifactParams {
                artifact_id: imported.artifact.artifact_id.as_ref().to_owned(),
                artifact_digest: imported.artifact.artifact_digest.as_ref().to_owned(),
                package_id: package.package_id.as_ref().to_owned(),
                package_version: package.package_version.as_ref().to_owned(),
                manifest_digest: imported.artifact.manifest.payload_digest.as_ref().to_owned(),
                manifest: serde_json::to_value(&imported.artifact.manifest.payload)
                    .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
                managed_path: imported.managed_relative_path.clone(),
                created_at: now_ms(),
            })
            .await?;
        self.artifacts.verify(&artifact).await?;
        self.repository
            .finish_operation(&FinishProductOperationParams {
                operation_id: operation_id.clone(),
                state: ProductOperationState::Succeeded,
                progress_percent: Some(100),
                last_error_code: None,
                bounded_log_tail: vec!["read-only candidate ready".into()],
                finished_at_ms: now_ms(),
            })
            .await?;

        let base_target_digest = match project.linked_mount_id.as_deref() {
            Some(mount_id) => self
                .repository
                .get_mount(mount_id)
                .await?
                .and_then(|mount| mount.current_artifact_digest),
            None => None,
        };
        let contract_diff = PluginContractDiff {
            compatibility: PluginCompatibility::Compatible,
            changes: [PluginContractChangeKind::ArtifactBytes]
                .into_iter()
                .collect(),
            affected_consumer_locks: Vec::new(),
        };
        let candidate_digest = digest_json(&json!({
            "project_id": project.project_id,
            "project_build_generation": project.build_generation,
            "artifact_digest": imported.artifact.artifact_digest,
            "base_target_digest": base_target_digest,
            "origin_operation_id": operation_id,
        }));
        self.repository
            .record_candidate(&RecordPluginReadyCandidateParams {
                candidate_id: Uuid::now_v7().to_string(),
                project_id: project.project_id.clone(),
                candidate_digest,
                origin: DbCandidateOrigin::Import,
                artifact_id: imported.artifact.artifact_id.as_ref().to_owned(),
                artifact_digest: imported.artifact.artifact_digest.as_ref().to_owned(),
                base_target_digest,
                source_snapshot_digest: None,
                dependency_lock_digest: None,
                contract_diff: serde_json::to_value(contract_diff)
                    .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
                origin_operation_id: operation_id,
                expected_generation: project.build_generation,
                created_at: now_ms(),
            })
            .await?;
        self.get_project(owner_user_id, &project.project_id).await
    }

    pub async fn configure(
        &self,
        input: ConfigureInput,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        let _guard = self
            .mount_guard(&input.owner_user_id, &input.request.mount_id)
            .await?;
        self.configure_locked(&input.owner_user_id, input.request)
            .await
    }

    async fn configure_locked(
        &self,
        owner_user_id: &str,
        request: ConfigurePluginRequest,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        let (mount, _) = self.owned_mount(owner_user_id, &request.mount_id).await?;
        require_mount_cas(
            &mount,
            request.expected_mount_revision,
            &request.expected_current_target_digest,
        )?;
        if mount.config_revision as u64 != request.expected_config_revision
            || mount.config_schema_digest.as_deref()
                != Some(request.expected_schema_digest.as_str())
        {
            return Err(PluginServiceError::stale(
                "config revision or schema digest changed",
            ));
        }
        if !request.values.is_object() {
            return Err(PluginServiceError::invalid(
                "plugin config values must be an object",
            ));
        }
        let current_bindings = self
            .repository
            .list_credentials(&ListPluginCredentialBindingsParams {
                mount_id: mount.mount_id.clone(),
                expected_mount_revision: mount.revision,
                expected_current_artifact_digest: mount.current_artifact_digest.clone(),
            })
            .await?;
        if current_bindings.bindings_revision as u64
            != request.expected_credential_bindings_revision
        {
            return Err(PluginServiceError::stale(
                "credential binding revision changed",
            ));
        }
        require_commit_fence(self.host.commit_fence(&request.mount_id).await?)?;
        let bindings = request
            .credential_bindings
            .into_iter()
            .filter_map(|(slot, credential_id)| {
                credential_id.map(|credential_id| PluginCredentialBindingInput {
                    slot,
                    credential_id,
                })
            })
            .collect::<Vec<_>>();

        let mount = self
            .repository
            .update_config(&UpdatePluginMountConfigParams {
                mount_id: request.mount_id,
                expected_mount_revision: request.expected_mount_revision as i64,
                expected_current_artifact_digest: Some(
                    request.expected_current_target_digest.clone(),
                ),
                expected_config_revision: request.expected_config_revision as i64,
                expected_config_schema_digest: Some(request.expected_schema_digest.clone()),
                config_schema_digest: request.expected_schema_digest,
                config: request.values,
                updated_at: now_ms(),
            })
            .await?;
        self.repository
            .replace_credentials(&ReplacePluginCredentialBindingsParams {
                mount_id: mount.mount_id.clone(),
                expected_mount_revision: mount.revision,
                expected_current_artifact_digest: mount.current_artifact_digest.clone(),
                expected_bindings_revision: request.expected_credential_bindings_revision as i64,
                bindings,
                updated_at: now_ms(),
            })
            .await?;
        let mount = self
            .repository
            .get_mount(&mount.mount_id)
            .await?
            .ok_or_else(|| {
                PluginServiceError::reconcile_required(
                    "configured Mount disappeared before Registry reconciliation",
                )
            })?;
        self.reconcile_committed_mount(owner_user_id, &mount).await?;
        self.get_mount(owner_user_id, &mount.mount_id).await
    }

    pub async fn set_enabled(
        &self,
        owner_user_id: &str,
        request: EnableRequest,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        let _guard = self.mount_guard(owner_user_id, &request.mount_id).await?;
        self.owned_mount(owner_user_id, &request.mount_id).await?;
        require_commit_fence(self.host.commit_fence(&request.mount_id).await?)?;
        let mount = self
            .repository
            .set_enabled(
                &request.mount_id,
                request.expected_mount_revision as i64,
                &request.expected_current_target_digest,
                request.enabled,
                now_ms(),
            )
            .await?;
        self.reconcile_committed_mount(owner_user_id, &mount).await?;
        self.get_mount(owner_user_id, &mount.mount_id).await
    }

    pub async fn retry(
        &self,
        owner_user_id: &str,
        request: RetryRequest,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        let _guard = self.mount_guard(owner_user_id, &request.mount_id).await?;
        self.owned_mount(owner_user_id, &request.mount_id).await?;
        require_commit_fence(self.host.commit_fence(&request.mount_id).await?)?;
        let mount = self
            .repository
            .retry_mount(
                &request.mount_id,
                request.expected_mount_revision as i64,
                &request.expected_current_target_digest,
                now_ms(),
            )
            .await?;
        self.reconcile_committed_mount(owner_user_id, &mount).await?;
        self.get_mount(owner_user_id, &mount.mount_id).await
    }

    pub async fn restore(
        &self,
        owner_user_id: &str,
        request: RestoreRequest,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        let _guard = self.mount_guard(owner_user_id, &request.mount_id).await?;
        self.owned_mount(owner_user_id, &request.mount_id).await?;
        require_commit_fence(self.host.commit_fence(&request.mount_id).await?)?;
        let mount = self
            .repository
            .restore_previous(&RestorePluginMountParams {
                mount_id: request.mount_id,
                expected_revision: request.expected_mount_revision as i64,
                expected_current_artifact_digest: request.expected_current_target_digest,
                expected_previous_artifact_digest: request.expected_previous_target_digest,
                restored_at: now_ms(),
            })
            .await?;
        self.reconcile_committed_mount(owner_user_id, &mount).await?;
        self.get_mount(owner_user_id, &mount.mount_id).await
    }

    pub async fn uninstall(
        &self,
        owner_user_id: &str,
        request: UninstallRequest,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        let _guard = self.mount_guard(owner_user_id, &request.mount_id).await?;
        self.owned_mount(owner_user_id, &request.mount_id).await?;
        require_commit_fence(self.host.commit_fence(&request.mount_id).await?)?;
        let mount = self
            .repository
            .uninstall_retain_data(&UninstallPluginMountParams {
                mount_id: request.mount_id,
                expected_revision: request.expected_mount_revision as i64,
                expected_current_artifact_digest: request.expected_current_target_digest,
                uninstalled_at: now_ms(),
            })
            .await?;
        self.reconcile_committed_mount(owner_user_id, &mount).await?;
        self.get_mount(owner_user_id, &mount.mount_id).await
    }

    pub async fn delete_data(
        &self,
        owner_user_id: &str,
        request: DeleteDataRequest,
    ) -> Result<bool, PluginServiceError> {
        let _guard = self.mount_guard(owner_user_id, &request.mount_id).await?;
        async {
            if request.expected_lifecycle != PluginLifecycleDto::UninstalledDataRetained
                || request.expected_mount_revision != request.expected_data_revision
            {
                return Err(PluginServiceError::stale(
                    "delete-data requires the exact retained lifecycle revision",
                ));
            }
            let (mount, _) = self.owned_mount(owner_user_id, &request.mount_id).await?;
            if mount.revision as u64 != request.expected_mount_revision
                || !mount.retained
                || mount.current_artifact_digest.is_some()
            {
                return Err(PluginServiceError::stale(
                    "Mount is no longer retained and uninstalled",
                ));
            }
            require_commit_fence(self.host.commit_fence(&request.mount_id).await?)?;
            let pending = self
                .repository
                .mark_delete_pending(
                    &request.mount_id,
                    request.expected_data_revision as i64,
                    now_ms(),
                )
                .await?;
            self.data_store
                .delete_mount_data(
                    &pending.mount_id,
                    &pending.data_dir_path,
                )
                .await?;
            self.repository.complete_data_delete(&pending.mount_id).await
        }
        .await
    }

    pub async fn build(
        &self,
        owner_user_id: &str,
        request: BuildRequest,
    ) -> Result<PluginProjectDetailDto, PluginServiceError> {
        let _guard = self.project_guard(owner_user_id, &request.project_id).await?;
        self.build_locked(owner_user_id, request).await
    }

    async fn build_locked(
        &self,
        owner_user_id: &str,
        request: BuildRequest,
    ) -> Result<PluginProjectDetailDto, PluginServiceError> {
        let project = self
            .repository
            .get_project(&request.project_id)
            .await?
            .ok_or_else(|| {
                PluginServiceError::not_found(format!("project {}", request.project_id))
            })?;
        require_project_owner(&project, owner_user_id)?;
        require_project_request_fresh(
            &project,
            request.expected_project_revision,
            request.expected_build_generation,
        )?;
        if project.managed_source_path.is_none()
            || project.source_head_digest.as_deref()
                != Some(request.expected_source_snapshot_digest.as_str())
            || project.dependency_lock_digest.as_deref()
                != Some(request.expected_dependency_lock_digest.as_str())
        {
            return Err(PluginServiceError::stale(
                "managed source or dependency lock changed",
            ));
        }

        let operation_id = Uuid::now_v7().to_string();
        self.repository
            .start_operation(&StartProductOperationParams {
                operation_id: operation_id.clone(),
                kind: ProductOperationKind::Build,
                owner_kind: "plugin_project".into(),
                owner_id: project.project_id.clone(),
                progress_percent: Some(0),
                bounded_log_tail: vec!["build started".into()],
                started_at_ms: now_ms(),
            })
            .await?;
        let output = match self
            .builder
            .build(&operation_id, &project, &request)
            .await
        {
            Ok(output) => output,
            Err(error) => {
                self.finish_build_error(owner_user_id, &operation_id, &error)
                    .await?;
                return Err(error);
            }
        };
        let prepared = async {
            output
                .artifact
                .validate()
                .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
            let (base_target_digest, contract_diff) = self
                .build_contract_diff(owner_user_id, &project, &output.artifact.manifest.payload)
                .await?;
            let artifact = self
                .repository
                .put_artifact(&CreatePluginArtifactParams {
                    artifact_id: output.artifact.artifact_id.as_ref().to_owned(),
                    artifact_digest: output.artifact.artifact_digest.as_ref().to_owned(),
                    package_id: output
                        .artifact
                        .manifest
                        .payload
                        .package
                        .package_id
                        .as_ref()
                        .to_owned(),
                    package_version: output
                        .artifact
                        .manifest
                        .payload
                        .package
                        .package_version
                        .as_ref()
                        .to_owned(),
                    manifest_digest: output.artifact.manifest.payload_digest.as_ref().to_owned(),
                    manifest: serde_json::to_value(&output.artifact.manifest.payload)
                        .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
                    managed_path: output.managed_relative_path.clone(),
                    created_at: now_ms(),
                })
                .await?;
            self.artifacts.verify(&artifact).await?;
            Ok::<_, PluginServiceError>((base_target_digest, contract_diff))
        }
        .await;
        let (base_target_digest, contract_diff) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.finish_build_error(owner_user_id, &operation_id, &error)
                    .await?;
                return Err(error);
            }
        };
        self.repository
            .finish_operation(&FinishProductOperationParams {
                operation_id: operation_id.clone(),
                state: ProductOperationState::Succeeded,
                progress_percent: Some(100),
                last_error_code: None,
                bounded_log_tail: vec!["candidate built".into()],
                finished_at_ms: now_ms(),
            })
            .await?;
        let candidate_digest = digest_json(&json!({
            "project_id": project.project_id,
            "generation": project.build_generation,
            "artifact_digest": output.artifact.artifact_digest,
            "base_target_digest": base_target_digest,
            "contract_diff": contract_diff,
        }));
        self.repository
            .record_candidate(&RecordPluginReadyCandidateParams {
                candidate_id: Uuid::now_v7().to_string(),
                project_id: project.project_id.clone(),
                candidate_digest,
                origin: DbCandidateOrigin::Build,
                artifact_id: output.artifact.artifact_id.as_ref().to_owned(),
                artifact_digest: output.artifact.artifact_digest.as_ref().to_owned(),
                base_target_digest,
                source_snapshot_digest: Some(output.source_snapshot_digest),
                dependency_lock_digest: Some(output.dependency_lock_digest),
                contract_diff: serde_json::to_value(contract_diff)
                    .map_err(|error| PluginServiceError::invalid(error.to_string()))?,
                origin_operation_id: operation_id,
                expected_generation: project.build_generation,
                created_at: now_ms(),
            })
            .await?;
        self.get_project(owner_user_id, &project.project_id).await
    }

    async fn finish_build_error(
        &self,
        owner_user_id: &str,
        operation_id: &str,
        error: &PluginServiceError,
    ) -> Result<(), PluginServiceError> {
        let canceled = error.code() == crate::ERR_OPERATION_CANCELED;
        let terminal = FinishProductOperationParams {
            operation_id: operation_id.to_owned(),
            state: if canceled {
                ProductOperationState::Canceled
            } else {
                ProductOperationState::Failed
            },
            progress_percent: None,
            last_error_code: (!canceled).then(|| error.code().into()),
            bounded_log_tail: vec![if canceled {
                "build canceled".into()
            } else {
                "build failed".into()
            }],
            finished_at_ms: now_ms(),
        };
        if let Err(finish_error) = self.repository.finish_operation(&terminal).await {
            let already_canceled = canceled
                && self
                    .repository
                    .get_operation(owner_user_id, operation_id)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|operation| {
                        operation.state == ProductOperationState::Canceled.as_str()
                    });
            if !already_canceled {
                return Err(finish_error);
            }
        }
        Ok(())
    }

    async fn build_contract_diff(
        &self,
        owner_user_id: &str,
        project: &PluginProjectRow,
        candidate: &PluginPackageV1Manifest,
    ) -> Result<(Option<String>, PluginContractDiff), PluginServiceError> {
        let Some(mount_id) = project.linked_mount_id.as_deref() else {
            return Ok((None, plugin_contract_diff(None, candidate)?));
        };
        let (mount, _) = self.owned_mount(owner_user_id, mount_id).await?;
        if mount.package_id != project.package_id
            || candidate.package.package_id.as_ref() != project.package_id
        {
            return Err(PluginServiceError::conflict(
                "linked Project, Mount, and built Package identities differ",
            ));
        }
        let base_target_digest = mount.current_artifact_digest.clone().ok_or_else(|| {
            PluginServiceError::stale("linked Mount has no current Artifact")
        })?;
        let current = self
            .verified_artifact(&base_target_digest)
            .await?
            .ok_or_else(|| PluginServiceError::not_found("linked Mount current Artifact"))?;
        let current = artifact_manifest(&current)?;
        Ok((
            Some(base_target_digest),
            plugin_contract_diff(Some(&current), candidate)?,
        ))
    }

    pub async fn test_candidate(
        &self,
        owner_user_id: &str,
        request: TestRequest,
    ) -> Result<PluginProjectDetailDto, PluginServiceError> {
        let _guard = self.project_guard(owner_user_id, &request.project_id).await?;
        self.test_candidate_locked(owner_user_id, request).await
    }

    async fn test_candidate_locked(
        &self,
        owner_user_id: &str,
        request: TestRequest,
    ) -> Result<PluginProjectDetailDto, PluginServiceError> {
        let project = self
            .repository
            .get_project(&request.project_id)
            .await?
            .ok_or_else(|| {
                PluginServiceError::not_found(format!("project {}", request.project_id))
            })?;
        require_project_owner(&project, owner_user_id)?;
        require_project_request_fresh(
            &project,
            request.expected_project_revision,
            request.expected_build_generation,
        )?;
        let candidate = self
            .repository
            .get_candidate(&project.project_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found("ready candidate"))?;
        if candidate.candidate_id != request.candidate_id
            || candidate.candidate_digest != request.expected_candidate_digest
        {
            return Err(PluginServiceError::stale(
                "candidate changed before test",
            ));
        }
        if let Some(mount_id) = project.linked_mount_id.as_deref() {
            let mount = self
                .repository
                .get_mount(mount_id)
                .await?
                .ok_or_else(|| PluginServiceError::not_found(format!("mount {mount_id}")))?;
            let bindings = self
                .repository
                .list_credentials(&ListPluginCredentialBindingsParams {
                    mount_id: mount.mount_id.clone(),
                    expected_mount_revision: mount.revision,
                    expected_current_artifact_digest: mount.current_artifact_digest.clone(),
                })
                .await?;
            if mount.config_revision as u64 != request.expected_config_revision
                || bindings.bindings_revision as u64
                    != request.expected_credential_bindings_revision
            {
                return Err(PluginServiceError::stale(
                    "candidate test config or credential revision changed",
                ));
            }
        } else if request.expected_config_revision != 0
            || request.expected_credential_bindings_revision != 0
        {
            return Err(PluginServiceError::stale(
                "first-install candidate test must use empty config and credential revisions",
            ));
        }
        let output = self.tester.test(&project, &candidate, &request).await?;
        let contract_candidate = candidate_contract(&candidate)?;
        output
            .receipt
            .validate_for(&contract_candidate)
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        if output.receipt.resolved_test_input_digest.as_ref()
            != request.resolved_test_input_digest
        {
            return Err(PluginServiceError::stale(
                "test input digest changed",
            ));
        }
        let receipt_json = serde_json::to_value(&output.receipt)
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        let receipt_digest = digest_json(&receipt_json);
        let runtime_digest = digest_payload(&output.receipt.runtime)
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        self.repository
            .record_test_receipt(&RecordPluginCandidateTestReceiptParams {
                receipt_id: output.receipt.receipt_id.as_ref().to_owned(),
                candidate_id: candidate.candidate_id,
                candidate_digest: candidate.candidate_digest,
                artifact_id: candidate.artifact_id,
                artifact_digest: candidate.artifact_digest,
                receipt_digest,
                runtime_fingerprint_digest: runtime_digest.as_ref().to_owned(),
                receipt: receipt_json,
                tested_at: output.receipt.issued_at_ms,
            })
            .await?;
        self.get_project(owner_user_id, &project.project_id).await
    }

    pub async fn apply_candidate(
        &self,
        owner_user_id: &str,
        request: ApplyPluginCandidateRequest,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        self.apply_candidate_authorized(owner_user_id, request, ApplyAuthorization::Manual)
            .await
    }

    pub async fn auto_apply_candidate(
        &self,
        owner_user_id: &str,
        request: ApplyPluginCandidateRequest,
        authorization_revision: u64,
        eligibility: PluginAutoApplyEligibility,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        self.apply_candidate_authorized(
            owner_user_id,
            request,
            ApplyAuthorization::StandingAuto {
                authorization_revision,
                eligibility,
            },
        )
        .await
    }

    async fn apply_candidate_authorized(
        &self,
        owner_user_id: &str,
        request: ApplyPluginCandidateRequest,
        authorization: ApplyAuthorization,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        let scope = match &request.target {
            ApplyPluginTargetDto::InitialInstall { .. } => PluginOwnerMutationScope::project(
                PluginProjectId::from(request.project_id.clone()),
            )?,
            ApplyPluginTargetDto::ExistingMount { mount_id, .. } => {
                PluginOwnerMutationScope::linked(
                    PluginProjectId::from(request.project_id.clone()),
                    PluginMountId::from(mount_id.clone()),
                )?
            }
        };
        let _guard = self.mutation_coordinator.acquire(&scope).await?;
        self.apply_candidate_locked(owner_user_id, request, authorization)
            .await
    }

    async fn apply_candidate_locked(
        &self,
        owner_user_id: &str,
        request: ApplyPluginCandidateRequest,
        authorization: ApplyAuthorization,
    ) -> Result<PluginDetailDto, PluginServiceError> {
        if matches!(
            (&authorization, &request.target),
            (
                ApplyAuthorization::StandingAuto { .. },
                ApplyPluginTargetDto::InitialInstall { .. }
            )
        ) {
            return Err(PluginServiceError::invalid(
                "initial installation is always manual",
            ));
        }
        let project = self
            .repository
            .get_project(&request.project_id)
            .await?
            .ok_or_else(|| {
                PluginServiceError::not_found(format!("project {}", request.project_id))
            })?;
        require_project_owner(&project, owner_user_id)?;
        require_project_request_fresh(
            &project,
            request.expected_project_revision,
            request.expected_build_generation,
        )?;
        let candidate = self
            .repository
            .get_candidate(&project.project_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found("ready candidate"))?;
        if candidate.candidate_id != request.candidate_id
            || candidate.candidate_digest != request.expected_candidate_digest
        {
            return Err(PluginServiceError::stale(
                "candidate is no longer the exact ready candidate",
            ));
        }
        let compatibility = candidate_compatibility(&candidate)?;
        match &authorization {
            ApplyAuthorization::Manual => {
                if compatibility == PluginCompatibility::Breaking && !request.allow_breaking {
                    return Err(PluginServiceError::invalid(
                        "breaking candidate requires allow_breaking",
                    ));
                }
            }
            ApplyAuthorization::StandingAuto {
                authorization_revision,
                eligibility,
            } => {
                if *authorization_revision == 0
                    || !eligibility.is_eligible()
                    || compatibility != PluginCompatibility::Compatible
                    || matches!(request.target, ApplyPluginTargetDto::InitialInstall { .. })
                {
                    return Err(PluginServiceError::invalid(
                        "auto apply requires explicit authorization and every exact eligibility predicate",
                    ));
                }
            }
        }
        let receipt = self
            .repository
            .get_test_receipt(&candidate.candidate_id)
            .await?;
        let passed = receipt
            .as_ref()
            .and_then(|row| serde_json::from_str::<Value>(&row.receipt_json).ok())
            .and_then(|value| value.get("outcome").cloned())
            == Some(Value::String("passed".into()));
        if !passed && !request.acknowledge_test_warning {
            return Err(PluginServiceError::invalid(
                "candidate requires a matching passed test or explicit warning acknowledgement",
            ));
        }

        let artifact = self
            .verified_artifact(&candidate.artifact_digest)
            .await?
            .ok_or_else(|| PluginServiceError::not_found("candidate artifact"))?;
        let manifest = artifact_manifest(&artifact)?;
        let config_schema_digest = digest_payload(&manifest.package.config_schema)
            .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
        let (expected_mount_revision, expected_current, new_mount_id, new_data_dir_path) =
            match request.target {
                ApplyPluginTargetDto::InitialInstall {
                    expected_library_revision,
                } => {
                    if !matches!(authorization, ApplyAuthorization::Manual)
                        || candidate.base_target_digest.is_some()
                    {
                        return Err(PluginServiceError::invalid(
                            "initial installation is manual and has no base target",
                        ));
                    }
                    let inventory = self.repository.inventory(owner_user_id).await?;
                    if inventory.library_revision != expected_library_revision {
                        return Err(PluginServiceError::stale(
                            "library revision changed before initial installation",
                        ));
                    }
                    let mount_id = Uuid::now_v7().to_string();
                    (
                        Some(0),
                        None,
                        Some(mount_id.clone()),
                        Some(managed_relative_child(
                            &self.paths.mount_data_relative_root,
                            &mount_id,
                        )?),
                    )
                }
                ApplyPluginTargetDto::ExistingMount {
                    mount_id,
                    expected_mount_revision,
                    expected_current_target_digest,
                } => {
                    require_commit_fence(self.host.commit_fence(&mount_id).await?)?;
                    if project.linked_mount_id.as_deref() != Some(mount_id.as_str())
                        || candidate.base_target_digest.as_deref()
                            != Some(expected_current_target_digest.as_str())
                    {
                        return Err(PluginServiceError::stale(
                            "candidate base or linked mount changed",
                        ));
                    }
                    (
                        Some(expected_mount_revision as i64),
                        Some(expected_current_target_digest),
                        None,
                        None,
                    )
                }
            };
        let mount = self
            .repository
            .apply_candidate(&ApplyPluginCandidateParams {
                project_id: project.project_id,
                candidate_id: candidate.candidate_id,
                expected_project_generation: request.expected_build_generation as i64,
                expected_mount_revision,
                expected_current_artifact_digest: expected_current,
                new_mount_id,
                new_data_dir_path,
                config_schema_digest: config_schema_digest.as_ref().to_owned(),
                initial_config: json!({}),
                applied_at: now_ms(),
            })
            .await?;
        self.reconcile_committed_mount(owner_user_id, &mount).await?;
        self.get_mount(owner_user_id, &mount.mount_id).await
    }

    pub async fn list_operations(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<DurableOperationSummaryDto>, PluginServiceError> {
        Ok(self
            .repository
            .list_operations(owner_user_id)
            .await?
            .iter()
            .map(operation_summary)
            .collect())
    }

    pub async fn get_operation(
        &self,
        owner_user_id: &str,
        operation_id: &str,
    ) -> Result<DurableOperationDetailDto, PluginServiceError> {
        let operation = self
            .repository
            .get_operation(owner_user_id, operation_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("operation {operation_id}")))?;
        Ok(operation_detail(&operation))
    }

    pub async fn cancel_operation(
        &self,
        owner_user_id: &str,
        operation_id: &str,
        expected_revision: u64,
    ) -> Result<DurableOperationSummaryDto, PluginServiceError> {
        let operation = self
            .repository
            .get_operation(owner_user_id, operation_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("operation {operation_id}")))?;
        if terminal_revision(&operation) != expected_revision || operation.state != "running" {
            return Err(PluginServiceError::stale(
                "operation state or revision changed",
            ));
        }
        self.operation_cancellation.cancel(&operation).await?;
        let current = self
            .repository
            .get_operation(owner_user_id, operation_id)
            .await?
            .ok_or_else(|| PluginServiceError::not_found(format!("operation {operation_id}")))?;
        if current.state == ProductOperationState::Canceled.as_str() {
            return Ok(operation_summary(&current));
        }
        if current.state != ProductOperationState::Running.as_str()
            || terminal_revision(&current) != expected_revision
        {
            return Err(PluginServiceError::stale(
                "operation completed while cancellation was being applied",
            ));
        }
        self.repository
            .cancel_operation(owner_user_id, operation_id, expected_revision, now_ms())
            .await
            .map(|operation| operation_summary(&operation))
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_millis() as i64
}

fn managed_relative_child(base: &str, child: &str) -> Result<String, PluginServiceError> {
    let base_path = Path::new(base);
    if base.trim().is_empty()
        || child.trim().is_empty()
        || base_path.is_absolute()
        || base_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || child
            .chars()
            .any(|character| character == '/' || character == '\\')
    {
        return Err(PluginServiceError::invalid(
            "managed path must be a relative directory plus one stable child identity",
        ));
    }
    Ok(format!("{}/{}", base.trim_end_matches('/'), child))
}

fn digest_json(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).expect("JSON value is serializable");
    hex::encode(Sha256::digest(bytes))
}

fn require_project_owner(
    project: &PluginProjectRow,
    owner_user_id: &str,
) -> Result<(), PluginServiceError> {
    if project.owner_user_id == owner_user_id {
        Ok(())
    } else {
        Err(PluginServiceError::Coded {
            code: crate::ERR_FORBIDDEN,
            message: "project belongs to another owner".into(),
        })
    }
}

fn require_project_request_fresh(
    project: &PluginProjectRow,
    expected_project_revision: u64,
    expected_build_generation: u64,
) -> Result<(), PluginServiceError> {
    if project.updated_at as u64 == expected_project_revision
        && project.build_generation as u64 == expected_build_generation
    {
        Ok(())
    } else {
        Err(PluginServiceError::stale(
            "project revision or build generation changed",
        ))
    }
}

fn require_mount_cas(
    mount: &PluginMountRow,
    expected_revision: u64,
    expected_digest: &str,
) -> Result<(), PluginServiceError> {
    if mount.revision as u64 == expected_revision
        && mount.current_artifact_digest.as_deref() == Some(expected_digest)
    {
        Ok(())
    } else {
        Err(PluginServiceError::stale(
            "mount revision or target digest changed",
        ))
    }
}

fn require_commit_fence(fence: PluginHostCommitFence) -> Result<(), PluginServiceError> {
    match fence {
        PluginHostCommitFence::NotResident => Ok(()),
        PluginHostCommitFence::ResidentFenced {
            host_generation,
            fence_token_digest,
        } if host_generation > 0 && fence_token_digest.as_ref().len() == 64 => Ok(()),
        _ => Err(PluginServiceError::conflict(
            "Host did not provide a valid quiescent commit fence",
        )),
    }
}

fn artifact_manifest(
    artifact: &PluginArtifactRow,
) -> Result<PluginPackageV1Manifest, PluginServiceError> {
    serde_json::from_str(&artifact.manifest_json)
        .map_err(|error| PluginServiceError::invalid(format!("stored manifest: {error}")))
}

fn plugin_contract_diff(
    current: Option<&PluginPackageV1Manifest>,
    candidate: &PluginPackageV1Manifest,
) -> Result<PluginContractDiff, PluginServiceError> {
    let mut changes = BTreeSet::from([PluginContractChangeKind::ArtifactBytes]);
    let Some(current) = current else {
        return Ok(PluginContractDiff {
            compatibility: PluginCompatibility::Compatible,
            changes,
            affected_consumer_locks: Vec::new(),
        });
    };

    if current.dependency_lock_digest != candidate.dependency_lock_digest {
        changes.insert(PluginContractChangeKind::DependencyLock);
    }
    if contribution_identity_digest(current)? != contribution_identity_digest(candidate)? {
        changes.insert(PluginContractChangeKind::ContributionSet);
    }
    if current.package.contributions != candidate.package.contributions {
        changes.insert(PluginContractChangeKind::ContractDigest);
    }
    if resource_effect_digest(current)? != resource_effect_digest(candidate)? {
        changes.insert(PluginContractChangeKind::ResourceOrEffectContract);
    }
    if current.package.config_schema != candidate.package.config_schema {
        changes.insert(PluginContractChangeKind::ConfigSchema);
    }
    if current.credential_slots != candidate.credential_slots {
        changes.insert(PluginContractChangeKind::CredentialSlots);
    }
    if current.minimum_node_major != candidate.minimum_node_major
        || current.package.requires_runtime_features != candidate.package.requires_runtime_features
        || current.package.package_dependencies != candidate.package.package_dependencies
    {
        changes.insert(PluginContractChangeKind::RuntimeRequirement);
    }
    if current.supported_targets != candidate.supported_targets {
        changes.insert(PluginContractChangeKind::SupportedTargets);
    }
    if host_sdk_contract_digest(current)? != host_sdk_contract_digest(candidate)? {
        changes.insert(PluginContractChangeKind::HostSdkContract);
    }

    let compatibility = if changes.iter().all(|change| {
        matches!(
            change,
            PluginContractChangeKind::ArtifactBytes
                | PluginContractChangeKind::DependencyLock
        )
    }) {
        PluginCompatibility::Compatible
    } else {
        PluginCompatibility::Breaking
    };
    Ok(PluginContractDiff {
        compatibility,
        changes,
        affected_consumer_locks: Vec::new(),
    })
}

fn contribution_identity_digest(
    manifest: &PluginPackageV1Manifest,
) -> Result<nomifun_agent_contracts::DigestHex, PluginServiceError> {
    let contributions = &manifest.package.contributions;
    digest_payload(&json!({
        "capabilities": contributions.capabilities.iter().map(|capability| json!({
            "id": capability.id,
            "contribution_id": capability.contribution_id,
            "version": capability.version,
            "kind": capability.kind,
        })).collect::<Vec<_>>(),
        "skills": contributions.skills.iter().map(|skill| json!({
            "id": skill.id,
            "version": skill.version,
        })).collect::<Vec<_>>(),
        "mcp_tools": contributions.mcp_tools.iter().map(|mapping| json!({
            "server_id": mapping.server_id,
            "canonical_tool_key": mapping.canonical_tool_key,
            "capability": mapping.capability,
        })).collect::<Vec<_>>(),
    }))
    .map_err(|error| PluginServiceError::invalid(error.to_string()))
}

fn resource_effect_digest(
    manifest: &PluginPackageV1Manifest,
) -> Result<nomifun_agent_contracts::DigestHex, PluginServiceError> {
    digest_payload(
        &manifest
            .package
            .contributions
            .capabilities
            .iter()
            .map(|capability| {
                json!({
                    "id": capability.id,
                    "kind": capability.kind,
                    "contributions": capability.contributions,
                })
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|error| PluginServiceError::invalid(error.to_string()))
}

fn host_sdk_contract_digest(
    manifest: &PluginPackageV1Manifest,
) -> Result<nomifun_agent_contracts::DigestHex, PluginServiceError> {
    let entrypoint = manifest.package.entrypoint.as_javascript().ok_or_else(|| {
        PluginServiceError::invalid("Plugin Package build requires a JavaScript entrypoint")
    })?;
    digest_payload(&json!({
        "schema_version": manifest.schema_version,
        "build_profile": manifest.build_profile,
        "build_profile_version": manifest.build_profile_version,
        "package_schema_version": manifest.package.schema_version,
        "host_contract_version": manifest.package.host_contract_version,
        "entrypoint_path": entrypoint.normalized_relative_path,
        "entrypoint_host_protocol_version": entrypoint.host_protocol_version,
        "entrypoint_sdk_contract_version": entrypoint.sdk_contract_version,
    }))
    .map_err(|error| PluginServiceError::invalid(error.to_string()))
}

fn candidate_compatibility(
    candidate: &PluginReadyCandidateRow,
) -> Result<PluginCompatibility, PluginServiceError> {
    let diff: PluginContractDiff = serde_json::from_str(&candidate.contract_diff_json)
        .map_err(|error| PluginServiceError::invalid(format!("stored contract diff: {error}")))?;
    Ok(diff.compatibility)
}

fn candidate_contract(
    candidate: &PluginReadyCandidateRow,
) -> Result<PluginReadyCandidate, PluginServiceError> {
    let contract_diff: PluginContractDiff = serde_json::from_str(&candidate.contract_diff_json)
        .map_err(|error| PluginServiceError::invalid(format!("stored contract diff: {error}")))?;
    let source_lineage = match (
        candidate.source_snapshot_digest.as_deref(),
        candidate.dependency_lock_digest.as_deref(),
    ) {
        (Some(source), Some(lock)) => PluginSourceLineage::Managed {
            source_snapshot_digest: source.into(),
            dependency_lock_digest: lock.into(),
            build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
        },
        (None, None) => PluginSourceLineage::RuntimeOnly,
        _ => {
            return Err(PluginServiceError::invalid(
                "stored candidate has incomplete source lineage",
            ));
        }
    };
    Ok(PluginReadyCandidate {
        candidate_id: candidate.candidate_id.clone().into(),
        candidate_digest: candidate.candidate_digest.clone().into(),
        project_id: candidate.project_id.clone().into(),
        project_build_generation: candidate.build_generation as u64,
        origin_operation_id: candidate.origin_operation_id.clone().into(),
        origin: if candidate.origin_kind == "build" {
            nomifun_agent_contracts::PluginCandidateOrigin::Build
        } else {
            nomifun_agent_contracts::PluginCandidateOrigin::Import
        },
        target: PluginTargetRef {
            artifact_id: candidate.artifact_id.clone().into(),
            artifact_digest: candidate.artifact_digest.clone().into(),
            manifest_digest: candidate.target_manifest_digest.clone().into(),
            package: nomifun_agent_contracts::PackageRef {
                id: candidate.target_package_id.clone().into(),
                version: candidate.target_package_version.clone().into(),
            },
        },
        base_target_digest: candidate.base_target_digest.clone().map(Into::into),
        source_lineage,
        contract_diff,
        matching_test_receipt: None,
    })
}

fn project_library(inventory: &PluginInventory) -> PluginLibraryResponseDto {
    let artifacts = inventory
        .artifacts
        .iter()
        .map(|artifact| (artifact.artifact_digest.as_str(), artifact))
        .collect::<BTreeMap<_, _>>();
    let plugins = inventory
        .mounts
        .iter()
        .map(|mount| {
            let current_artifact = mount
                .current_artifact_digest
                .as_deref()
                .and_then(|digest| artifacts.get(digest).copied())
                ;
            let current = current_artifact.map(target_dto);
            let previous = mount
                .previous_artifact_digest
                .as_deref()
                .and_then(|digest| artifacts.get(digest).copied())
                .map(target_dto);
            let linked_project = inventory.projects.iter().find(|project| {
                project.linked_mount_id.as_deref() == Some(mount.mount_id.as_str())
            });
            let package_display = current_artifact
                .and_then(|artifact| artifact_manifest(artifact).ok())
                .map(|manifest| manifest.package.display);
            PluginSummaryDto {
                mount_id: mount.mount_id.clone(),
                mount_revision: mount.revision as u64,
                display_name: linked_project
                    .map(|project| project.display_name.clone())
                    .or_else(|| package_display.as_ref().map(|display| display.name.clone()))
                    .unwrap_or_else(|| mount.package_id.clone()),
                description: linked_project
                    .and_then(|project| (!project.description.is_empty()).then(|| project.description.clone()))
                    .or_else(|| package_display.map(|display| display.description)),
                lifecycle: lifecycle(mount),
                current,
                previous,
                linked_project_id: linked_project.map(|project| project.project_id.clone()),
                contribution_count: 0,
                updated_at_ms: mount.updated_at,
            }
        })
        .collect();
    let projects = inventory
        .projects
        .iter()
        .map(|project| {
            let candidate = inventory
                .candidates
                .iter()
                .find(|candidate| candidate.project_id == project.project_id);
            project_summary(project, candidate)
        })
        .collect();
    PluginLibraryResponseDto {
        library_revision: inventory.library_revision,
        plugins,
        projects,
    }
}

fn project_summary(
    project: &PluginProjectRow,
    candidate: Option<&PluginReadyCandidateRow>,
) -> PluginProjectSummaryDto {
    let source_state = if project.managed_source_path.is_some() {
        if project.source_head_digest.is_some() {
            PluginProjectSourceStateDto::Editable
        } else {
            PluginProjectSourceStateDto::Empty
        }
    } else {
        PluginProjectSourceStateDto::RuntimeOnly
    };
    PluginProjectSummaryDto {
        project_id: project.project_id.clone(),
        project_revision: project.updated_at as u64,
        display_name: project.display_name.clone(),
        description: (!project.description.is_empty()).then(|| project.description.clone()),
        linked_mount_id: project.linked_mount_id.clone(),
        source_state,
        build_generation: project.build_generation as u64,
        ready_candidate: candidate.map(|candidate| PluginCandidateRefDto {
            candidate_id: candidate.candidate_id.clone(),
            candidate_digest: candidate.candidate_digest.clone(),
        }),
        apply_mode: PluginApplyModeDto::AskBeforeApply,
        updated_at_ms: project.updated_at,
    }
}

fn project_detail_with_state(
    project: &PluginProjectRow,
    candidate: Option<&PluginReadyCandidateRow>,
    receipt: Option<&PluginCandidateTestReceiptRow>,
    operation: Option<&ProductOperationRow>,
) -> Result<PluginProjectDetailDto, PluginServiceError> {
    Ok(PluginProjectDetailDto {
        summary: project_summary(project, candidate),
        source_snapshot_digest: project.source_head_digest.clone(),
        dependency_lock_digest: project.dependency_lock_digest.clone(),
        ready: candidate
            .map(|candidate| candidate_dto(candidate, receipt))
            .transpose()?,
        active_operation: operation.map(operation_summary),
    })
}

fn target_dto(artifact: &PluginArtifactRow) -> PluginTargetRefDto {
    PluginTargetRefDto {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        artifact_id: artifact.artifact_id.clone(),
        artifact_digest: artifact.artifact_digest.clone(),
        manifest_digest: artifact.manifest_digest.clone(),
    }
}

fn candidate_dto(
    candidate: &PluginReadyCandidateRow,
    receipt: Option<&PluginCandidateTestReceiptRow>,
) -> Result<PluginReadyCandidateDto, PluginServiceError> {
    let diff = serde_json::from_str::<PluginContractDiff>(&candidate.contract_diff_json).ok();
    let compatibility = match diff.as_ref().map(|diff| &diff.compatibility) {
        Some(PluginCompatibility::Compatible) => PluginCompatibilityDto::Compatible,
        Some(PluginCompatibility::Breaking) => PluginCompatibilityDto::Breaking,
        _ => PluginCompatibilityDto::Unknown,
    };
    let changed_contracts = diff
        .as_ref()
        .map(|diff| {
            diff.changes
                .iter()
                .map(|change| format!("{change:?}").to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let affected_consumers = diff
        .as_ref()
        .map(|diff| {
            diff.affected_consumer_locks
                .iter()
                .map(|lock| PluginAffectedConsumerDto {
                    surface: affected_consumer_surface(lock.consumer_kind),
                    consumer_id: lock.consumer_id.clone(),
                    contribution_id: lock.contribution_id.as_ref().to_owned(),
                    expected_contract_digest: lock.contract_digest.as_ref().to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    let receipt = receipt
        .map(|receipt| {
            serde_json::from_str::<CandidateTestReceipt>(&receipt.receipt_json)
                .map_err(|error| {
                    PluginServiceError::integration(format!(
                        "stored Candidate Test receipt is invalid: {error}"
                    ))
                })
        })
        .transpose()?;
    if let Some(receipt) = &receipt {
        receipt
            .validate_for(&candidate_contract(candidate)?)
            .map_err(|error| {
                PluginServiceError::integration(format!(
                    "stored Candidate Test receipt does not match its Candidate: {error}"
                ))
            })?;
    }
    Ok(PluginReadyCandidateDto {
        candidate: PluginCandidateRefDto {
            candidate_id: candidate.candidate_id.clone(),
            candidate_digest: candidate.candidate_digest.clone(),
        },
        origin: if candidate.origin_kind == "build" {
            PluginCandidateOriginDto::Build
        } else {
            PluginCandidateOriginDto::Import
        },
        target: PluginTargetRefDto {
            package_id: candidate.target_package_id.clone(),
            package_version: candidate.target_package_version.clone(),
            artifact_id: candidate.artifact_id.clone(),
            artifact_digest: candidate.artifact_digest.clone(),
            manifest_digest: candidate.target_manifest_digest.clone(),
        },
        project_build_generation: candidate.build_generation as u64,
        base_target_digest: candidate.base_target_digest.clone(),
        test: PluginCandidateTestDto {
            status: match receipt.as_ref().map(|receipt| receipt.outcome) {
                Some(nomifun_agent_contracts::CandidateTestOutcome::Passed) => {
                    PluginCandidateTestStatusDto::Passed
                }
                Some(nomifun_agent_contracts::CandidateTestOutcome::Failed) => {
                    PluginCandidateTestStatusDto::Failed
                }
                Some(nomifun_agent_contracts::CandidateTestOutcome::NeedsTestInput) => {
                    PluginCandidateTestStatusDto::NeedsTestInput
                }
                None => PluginCandidateTestStatusDto::NotRun,
            },
            receipt_id: receipt
                .as_ref()
                .map(|receipt| receipt.receipt_id.as_ref().to_owned()),
            candidate_id: candidate.candidate_id.clone(),
            candidate_digest: candidate.candidate_digest.clone(),
            runtime: receipt.as_ref().map(|receipt| JavascriptRuntimeRefDto {
                runtime_installation_id: receipt
                    .runtime
                    .runtime_installation_id
                    .as_ref()
                    .to_owned(),
                node_version: receipt.runtime.node_version.as_ref().to_owned(),
                runtime_target: receipt.runtime.runtime_target.as_ref().to_owned(),
                executable_digest: receipt.runtime.executable_digest.as_ref().to_owned(),
            }),
            resolved_test_input_digest: receipt
                .as_ref()
                .map(|receipt| receipt.resolved_test_input_digest.as_ref().to_owned()),
            issued_at_ms: receipt.as_ref().map(|receipt| receipt.issued_at_ms),
            error_code: None,
        },
        impact: PluginCandidateImpactDto {
            compatibility,
            changed_contracts,
            affected_consumers,
            can_apply: true,
            can_auto_apply: false,
            blocking_reasons: Vec::new(),
        },
    })
}

fn affected_consumer_surface(kind: AffectedConsumerKind) -> PluginConsumerSurfaceDto {
    match kind {
        AffectedConsumerKind::AgentPresetRevision | AffectedConsumerKind::AgentBinding => {
            PluginConsumerSurfaceDto::Agent
        }
        AffectedConsumerKind::GatewayOperation => PluginConsumerSurfaceDto::Gateway,
        AffectedConsumerKind::RemoteOperation => PluginConsumerSurfaceDto::Remote,
        AffectedConsumerKind::AutomationOperation => PluginConsumerSurfaceDto::Automation,
        AffectedConsumerKind::UiOperation => PluginConsumerSurfaceDto::Ui,
        AffectedConsumerKind::MiniappServiceOperation => PluginConsumerSurfaceDto::MiniappService,
    }
}

fn lifecycle(mount: &PluginMountRow) -> PluginLifecycleDto {
    if mount.delete_pending {
        PluginLifecycleDto::DeletePending
    } else if mount.retained {
        PluginLifecycleDto::UninstalledDataRetained
    } else if mount.current_artifact_digest.is_none() || mount.last_error.is_some() {
        PluginLifecycleDto::Error
    } else if mount.enabled {
        PluginLifecycleDto::Enabled
    } else {
        PluginLifecycleDto::Disabled
    }
}

fn mount_detail(
    mount: &PluginMountRow,
    current_artifact: Option<&PluginArtifactRow>,
    previous_artifact: Option<&PluginArtifactRow>,
    bindings: &PluginCredentialBindingSnapshot,
) -> Result<PluginDetailDto, PluginServiceError> {
    let manifest = current_artifact.map(artifact_manifest).transpose()?;
    let capabilities = match (current_artifact, manifest.as_ref()) {
        (Some(artifact), Some(manifest)) => manifest
            .package
            .contributions
            .capabilities
            .iter()
            .map(|capability| {
                let contract_digest = digest_payload(capability)
                    .map_err(|error| PluginServiceError::invalid(error.to_string()))?;
                Ok(PluginCapabilityContributionDto {
                    capability_id: capability.id.as_ref().to_owned(),
                    capability_version: capability.version.as_ref().to_owned(),
                    display_name: capability.display.name.clone(),
                    description: Some(capability.display.description.clone()),
                    provenance: nomifun_api_types::PluginContributionProvenanceDto {
                        mount_id: mount.mount_id.clone(),
                        mount_revision: mount.revision as u64,
                        artifact_id: artifact.artifact_id.clone(),
                        artifact_digest: artifact.artifact_digest.clone(),
                        manifest_digest: artifact.manifest_digest.clone(),
                        contribution_id: capability.contribution_id.as_ref().to_owned(),
                        contract_digest: contract_digest.as_ref().to_owned(),
                    },
                    consumer_availability: capability
                        .supported_consumers()
                        .map_err(PluginServiceError::invalid)?
                        .into_iter()
                        .map(|consumer| PluginConsumerAvailabilityDto {
                            surface: consumer_surface(consumer),
                            status: if mount.enabled {
                                PluginConsumerAvailabilityStatusDto::Active
                            } else {
                                PluginConsumerAvailabilityStatusDto::Disabled
                            },
                            reason_code: None,
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>, PluginServiceError>>()?,
        _ => Vec::new(),
    };
    let config_schema = manifest
        .as_ref()
        .map(|manifest| manifest.package.config_schema.0.clone())
        .unwrap_or_else(|| json!({}));
    let credential_slots = manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .credential_slots
                .iter()
                .map(|slot| {
                    let binding = bindings
                        .bindings
                        .iter()
                        .find(|binding| binding.slot == slot.slot_key.as_ref());
                    CredentialSlotBindingDto {
                        slot_key: slot.slot_key.as_ref().to_owned(),
                        display_name: slot.display_name.clone(),
                        required: slot.required,
                        status: if binding.is_some() {
                            CredentialBindingStatusDto::Bound
                        } else {
                            CredentialBindingStatusDto::Unbound
                        },
                        credential_id: binding.map(|binding| binding.credential_id.clone()),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(PluginDetailDto {
        summary: PluginSummaryDto {
            mount_id: mount.mount_id.clone(),
            mount_revision: mount.revision as u64,
            display_name: manifest
                .as_ref()
                .map(|manifest| manifest.package.display.name.clone())
                .unwrap_or_else(|| mount.package_id.clone()),
            description: manifest
                .as_ref()
                .map(|manifest| manifest.package.display.description.clone()),
            lifecycle: lifecycle(mount),
            current: current_artifact.map(target_dto),
            previous: previous_artifact.map(target_dto),
            linked_project_id: None,
            contribution_count: capabilities.len() as u32,
            updated_at_ms: mount.updated_at,
        },
        capabilities,
        config_schema: PluginConfigSchemaDto {
            schema_digest: mount.config_schema_digest.clone().unwrap_or_default(),
            schema: config_schema,
        },
        config: PluginConfigStateDto {
            config_revision: mount.config_revision as u64,
            schema_digest: mount.config_schema_digest.clone().unwrap_or_default(),
            values: serde_json::from_str(&mount.config_json).unwrap_or_else(|_| json!({})),
            valid: true,
            validation_errors: Vec::new(),
        },
        credential_bindings_revision: bindings.bindings_revision as u64,
        credential_slots,
        retained_data: mount.retained,
        last_error_code: mount.last_error.clone(),
    })
}

fn consumer_surface(consumer: CapabilityConsumer) -> PluginConsumerSurfaceDto {
    match consumer {
        CapabilityConsumer::Agent => PluginConsumerSurfaceDto::Agent,
        CapabilityConsumer::Gateway => PluginConsumerSurfaceDto::Gateway,
        CapabilityConsumer::Knowledge => PluginConsumerSurfaceDto::Knowledge,
        CapabilityConsumer::Remote => PluginConsumerSurfaceDto::Remote,
        CapabilityConsumer::Automation => PluginConsumerSurfaceDto::Automation,
        CapabilityConsumer::Ui => PluginConsumerSurfaceDto::Ui,
        CapabilityConsumer::MiniAppService => PluginConsumerSurfaceDto::MiniappService,
    }
}

fn operation_summary(row: &ProductOperationRow) -> DurableOperationSummaryDto {
    DurableOperationSummaryDto {
        operation_id: row.operation_id.clone(),
        operation_revision: terminal_revision(row),
        kind: match row.kind.as_str() {
            "build" => DurableOperationKindDto::Build,
            "import" => DurableOperationKindDto::Import,
            "export" => DurableOperationKindDto::Export,
            _ => DurableOperationKindDto::MiniappPermanentDelete,
        },
        owner: match row.owner_kind.as_str() {
            "plugin_mount" => DurableOperationOwnerDto::PluginMount {
                mount_id: row.owner_id.clone(),
            },
            "miniapp" => DurableOperationOwnerDto::Miniapp {
                miniapp_id: row.owner_id.clone(),
            },
            _ => DurableOperationOwnerDto::PluginProject {
                project_id: row.owner_id.clone(),
            },
        },
        state: match row.state.as_str() {
            "running" => DurableOperationStateDto::Running,
            "succeeded" => DurableOperationStateDto::Succeeded,
            "failed" => DurableOperationStateDto::Failed,
            _ => DurableOperationStateDto::Canceled,
        },
        cancelable: row.state == "running",
        progress_percent: row.progress_percent.map(|value| value as u8),
        started_at_ms: row.started_at_ms,
        completed_at_ms: row.finished_at_ms,
    }
}

fn terminal_revision(row: &ProductOperationRow) -> u64 {
    if row.finished_at_ms.is_some() { 2 } else { 1 }
}

fn operation_detail(row: &ProductOperationRow) -> DurableOperationDetailDto {
    DurableOperationDetailDto {
        summary: operation_summary(row),
        bounded_log_tail: serde_json::from_str(&row.bounded_log_tail_json).unwrap_or_default(),
        result_artifact_digests: BTreeMap::new(),
        last_error_code: row.last_error_code.clone(),
    }
}
