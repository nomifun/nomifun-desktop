use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use nomifun_agent_contracts::{
    canonical_json_bytes, digest_bytes, digest_payload, ArtifactId, DigestHex,
    JavaScriptBuildProfile, LocalizedMetadata, MiniAppBridgeKvRequest,
    MiniAppBridgeRequest, MiniAppBridgeSession, MiniAppBridgeSessionId,
    MiniAppBridgeTarget, MiniAppId, MiniAppKvResponse,
    MiniAppNonUiReleaseFingerprint, MiniAppProjectId,
    MiniAppPublishAuthorization,
    MiniAppPublishRequest as MiniAppPublishContract,
    MiniAppPointerExpectation, MiniAppReadyOrigin, MiniAppReadyRelease,
    MiniAppReadyReleaseRef, MiniAppReleaseId, MiniAppReleasePointerState,
    MiniAppReleaseRef, MiniAppResourceContract, MiniAppSourceLineage, OperationId,
    PackageContributions, PackageId, PackageRef, StrictJsonValue,
    MiniAppSurfaceSessionId, MiniAppUiOnlyAutoPublishAuthorization,
    MiniAppUiOnlyAutoPublishProof, MiniAppUserAuthorizationId, VersionString,
    MiniAppBridgeTransport, MINIAPP_BRIDGE_CONTRACT_VERSION,
    MINIAPP_RELEASE_PROFILE_VERSION,
};
use nomifun_api_types::{
    BuildMiniAppRequest, CredentialBindingStatusDto, CredentialSlotBindingDto,
    CreateMiniAppProjectRequest, DurableOperationKindDto, DurableOperationOwnerDto,
    DurableOperationStateDto, DurableOperationSummaryDto, MiniAppKindDto,
    MiniAppLibraryResponseDto, MiniAppLifecycleDto, MiniAppProjectSourceStateDto,
    MiniAppPublishModeDto, MiniAppReadyReleaseDto, MiniAppReleasePointersDto,
    MiniAppReleaseRefDto, MiniAppReleaseTestDto, MiniAppServiceHealthDto,
    MiniAppSummaryDto, MiniAppSurfaceLaunchDescriptorDto, MiniAppTestStatusDto,
    MiniAppWorkshopDto, PluginConfigSchemaDto, PluginConfigStateDto,
    PublishMiniAppRequest as PublishMiniAppRequestDto,
    RollbackMiniAppRequest as RollbackMiniAppRequestDto, SetMiniAppEnabledRequest,
    SetMiniAppPublishModeRequest,
};
use nomifun_db::{
    CancelMiniAppM1BuildOperationParams, CloseMiniAppM1SurfaceSessionParams,
    CreateMiniAppM1Params, CreateMiniAppM1WithSourceParams,
    ExecuteMiniAppM1SurfaceKvParams,
    FinishMiniAppM1BuildAndRecordReadyParams, FinishMiniAppM1BuildOperationParams,
    IMiniAppM1Repository, MiniAppM1AutoPublishGuard, MiniAppM1Kind,
    MiniAppM1ManagedSourceLineage, MiniAppM1Snapshot, MiniAppM1SurfaceKvOperation,
    MiniAppM1SurfaceKvResult, MiniAppProductRow,
    MiniAppReleaseArtifactRow, MiniAppReleaseRow, MiniAppSurfaceSessionRow,
    OpenMiniAppM1SurfaceSessionParams,
    ProductOperationRow, ProductOperationState, PublishMiniAppM1ReadyParams,
    ResolveMiniAppM1SurfaceSessionParams, RollbackMiniAppM1PreviousParams,
    SetMiniAppM1AutoPublishParams,
    CommitMiniAppM1LifecycleParams, StartMiniAppM1BuildOperationParams,
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    MiniAppDependencyLockV1, MiniAppReleaseFileBytes, MiniAppReleasePublishRequest,
    issue_surface_capability, surface_capability_digest, MiniAppReleaseArtifactIdentity,
    MiniAppReleaseStore,
    MiniAppSourceFile, MiniAppSourceScope, MiniAppSourceSnapshot, MiniAppSourceStore,
    MiniAppStaticBundleBuilder, MiniAppStaticBundleFile, MiniAppStaticBundleInput,
    MiniAppStoredRelease, materialize_surface_entrypoint,
};

#[derive(Debug, Error)]
pub enum MiniAppM1ApplicationError {
    #[error("MiniApp input is invalid: {0}")]
    Invalid(String),
    #[error("MiniApp was not found")]
    NotFound,
    #[error("MiniApp database failed: {0}")]
    Database(#[from] nomifun_db::DbError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppSurfaceAsset {
    pub normalized_relative_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone)]
struct MiniAppM1Stores {
    source: Arc<MiniAppSourceStore>,
    release: Arc<MiniAppReleaseStore>,
}

#[derive(Clone)]
pub struct MiniAppM1ApplicationService {
    repository: Arc<dyn IMiniAppM1Repository>,
    stores: MiniAppM1Stores,
}

impl std::fmt::Debug for MiniAppM1ApplicationService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MiniAppM1ApplicationService")
            .finish_non_exhaustive()
    }
}

impl MiniAppM1ApplicationService {
    pub fn new_with_root(
        repository: Arc<dyn IMiniAppM1Repository>,
        root: impl AsRef<Path>,
    ) -> Result<Self, MiniAppM1ApplicationError> {
        let root = root.as_ref().to_path_buf();
        let source = Arc::new(
            MiniAppSourceStore::new(root.join("source"))
                .map_err(|error| store_error("Source Store", error))?,
        );
        let release = Arc::new(
            MiniAppReleaseStore::new(root.join("release"))
                .map_err(|error| store_error("Release Store", error))?,
        );
        Self::new_with_stores(repository, source, release)
    }

    pub fn new_with_stores(
        repository: Arc<dyn IMiniAppM1Repository>,
        source: Arc<MiniAppSourceStore>,
        release: Arc<MiniAppReleaseStore>,
    ) -> Result<Self, MiniAppM1ApplicationError> {
        source
            .cleanup_failed_staging()
            .map_err(|error| store_error("Source Store cleanup", error))?;
        release
            .cleanup_failed_staging()
            .map_err(|error| store_error("Release Store cleanup", error))?;
        Ok(Self {
            repository,
            stores: MiniAppM1Stores { source, release },
        })
    }

    pub async fn library(
        &self,
        owner_user_id: &str,
    ) -> Result<MiniAppLibraryResponseDto, MiniAppM1ApplicationError> {
        let library = self.repository.library(owner_user_id).await?;
        let mut miniapps = Vec::with_capacity(library.products.len());
        for product in &library.products {
            let snapshot = self
                .repository
                .get(owner_user_id, &product.miniapp_id)
                .await?
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(format!(
                        "MiniApp {} disappeared from its owner-scoped Library",
                        product.miniapp_id
                    ))
                })?;
            miniapps.push(summary_from_snapshot(&snapshot)?);
        }
        Ok(MiniAppLibraryResponseDto {
            library_revision: nonnegative_u64(
                library.library.revision,
                "MiniApp library revision",
            )?,
            miniapps,
        })
    }

    pub async fn create(
        &self,
        owner_user_id: &str,
        request: CreateMiniAppProjectRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        if request.kind != MiniAppKindDto::UiOnly {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Service MiniApp creation is deferred to M1-1".to_owned(),
            ));
        }
        let expected_library_revision = to_i64(
            request.expected_library_revision,
            "library revision",
        )?;
        let stores = &self.stores;
        let miniapp_id = Uuid::now_v7().to_string();
        let project_id = Uuid::now_v7().to_string();
        let source = stores
            .source
            .create_project(
                owner_user_id,
                &miniapp_id,
                &project_id,
                request.display_name.clone(),
            )
            .map_err(|error| store_error("Source Store", error))?;
        let create = CreateMiniAppM1Params {
            owner_user_id: owner_user_id.to_owned(),
            miniapp_id: miniapp_id.clone(),
            project_id: project_id.clone(),
            expected_library_revision,
            display_name: request.display_name,
            description: request.description,
            icon_asset_id: None,
            kind: MiniAppM1Kind::UiOnly,
            materialized_catalog_digest: nomifun_agent_contracts::digest_bytes(
                b"miniapp-m1-empty-catalog",
            )
            .as_ref()
            .to_owned(),
            config_schema_json: r#"{"type":"object"}"#.to_owned(),
            config_json: "{}".to_owned(),
            created_at: nomifun_common::now_ms(),
        };
        let source_lineage = MiniAppM1ManagedSourceLineage {
            managed_source_path: source.managed_relative_path,
            source_head_digest: source.source_snapshot_digest.as_ref().to_owned(),
            dependency_lock_digest: source.dependency_lock_digest.as_ref().to_owned(),
            build_profile_version: source.build_profile_version.as_ref().to_owned(),
            build_generation: to_i64(source.build_generation, "build generation")?,
        };
        let snapshot = match self
            .repository
            .create_with_source(&CreateMiniAppM1WithSourceParams {
                create,
                source: source_lineage,
            })
            .await
        {
            Ok(snapshot) => snapshot,
            Err(database_error) => {
                match stores.source.delete_project(
                    owner_user_id,
                    &miniapp_id,
                    &project_id,
                ) {
                    Ok(()) => return Err(database_error.into()),
                    Err(cleanup_error) => {
                        return Err(MiniAppM1ApplicationError::Invalid(format!(
                            "database create failed ({database_error}); Source cleanup also failed ({cleanup_error})"
                        )));
                    }
                }
            }
        };
        workshop_from_snapshot(&snapshot, None)
    }

    pub async fn build(
        &self,
        owner_user_id: &str,
        request: BuildMiniAppRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let stores = &self.stores;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        validate_build_request(&snapshot, &request)?;
        let source = read_exact_source(&stores.source, owner_user_id, &snapshot, &request)?;
        let source_lineage = managed_source_lineage(&snapshot)?;
        let operation_id = Uuid::now_v7().to_string();
        let started_at_ms = positive_now_ms();
        self.repository
            .start_build_operation(&StartMiniAppM1BuildOperationParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                project_id: request.project_id.clone(),
                operation_id: operation_id.clone(),
                expected_project_revision: to_i64(
                    request.expected_project_revision,
                    "project revision",
                )?,
                expected_source: source_lineage,
                bounded_log_tail: vec!["UI-only Build started".to_owned()],
                started_at_ms,
            })
            .await?;

        let prepared = match prepare_build_release(
            owner_user_id,
            &snapshot,
            &request,
            &source,
            &operation_id,
            started_at_ms,
            &stores.release,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                return Err(self
                    .finish_failed_build(
                        owner_user_id,
                        &request.miniapp_id,
                        &operation_id,
                        error,
                    )
                    .await);
            }
        };
        let operation = self
            .repository
            .get_build_operation(owner_user_id, &request.miniapp_id, &operation_id)
            .await?
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(format!(
                    "Build operation {operation_id} disappeared before Ready commit"
                ))
            })?;
        if operation.state != ProductOperationState::Running.as_str() {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "Build operation {operation_id} is already {}; Ready was not changed",
                operation.state
            )));
        }

        let completed = self
            .repository
            .finish_build_and_record_ready(&FinishMiniAppM1BuildAndRecordReadyParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                project_id: request.project_id.clone(),
                operation_id: operation_id.clone(),
                expected_product_revision: to_i64(
                    request.expected_product_revision,
                    "product revision",
                )?,
                expected_pointer_revision: snapshot.product.pointer_revision,
                expected_project_revision: to_i64(
                    request.expected_project_revision,
                    "project revision",
                )?,
                expected_build_generation: to_i64(
                    request.expected_build_generation,
                    "build generation",
                )?,
                artifact: prepared.artifact,
                release: prepared.release,
                bounded_log_tail: vec![
                    "UI-only Release admitted".to_owned(),
                    "Ready Release committed atomically".to_owned(),
                ],
                finished_at_ms: prepared.finished_at_ms,
            })
            .await;
        match completed {
            Ok(snapshot) => {
                if snapshot
                    .auto_publish_authorization
                    .as_ref()
                    .is_some_and(|authorization| authorization.enabled)
                    && snapshot.active_release.is_some()
                {
                    match self.auto_publish_ready(owner_user_id, snapshot.clone()).await {
                        Ok(snapshot) => return workshop_from_snapshot(&snapshot, None),
                        Err(error) => {
                            let observed = self
                                .repository
                                .get(owner_user_id, &request.miniapp_id)
                                .await?
                                .ok_or(MiniAppM1ApplicationError::NotFound)?;
                            tracing::warn!(
                                miniapp_id = %request.miniapp_id,
                                error = %error,
                                ready_release_id = ?observed.product.ready_release_id,
                                active_release_id = ?observed.product.active_release_id,
                                "strict UI-only auto Publish returned an error; reconciled persisted state"
                            );
                            return workshop_from_snapshot(&observed, None);
                        }
                    }
                }
                workshop_from_snapshot(&snapshot, None)
            }
            Err(database_error) => {
                let original = MiniAppM1ApplicationError::Database(database_error);
                Err(self
                    .finish_failed_build(
                        owner_user_id,
                        &request.miniapp_id,
                        &operation_id,
                        original,
                    )
                    .await)
            }
        }
    }

    pub async fn cancel_build(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        operation_id: &str,
        expected_operation_revision: u64,
    ) -> Result<DurableOperationSummaryDto, MiniAppM1ApplicationError> {
        let current = self
            .repository
            .get_build_operation(owner_user_id, miniapp_id, operation_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        let current_summary = operation_summary(&current)?;
        if current_summary.operation_revision != expected_operation_revision
            || current.state != ProductOperationState::Running.as_str()
        {
            return Err(operation_conflict(
                operation_id,
                &current.state,
                "operation state or revision changed",
            ));
        }
        match self
            .repository
            .cancel_build_operation(&CancelMiniAppM1BuildOperationParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: miniapp_id.to_owned(),
                operation_id: operation_id.to_owned(),
                bounded_log_tail: vec!["UI-only Build canceled".to_owned()],
                finished_at_ms: positive_now_ms().max(current.started_at_ms),
            })
            .await
        {
            Ok(operation) => operation_summary(&operation),
            Err(error) => {
                let observed = self
                    .repository
                    .get_build_operation(owner_user_id, miniapp_id, operation_id)
                    .await?;
                if let Some(observed) = observed
                    && observed.state == ProductOperationState::Canceled.as_str()
                {
                    return operation_summary(&observed);
                }
                Err(error.into())
            }
        }
    }

    pub async fn publish(
        &self,
        owner_user_id: &str,
        request: PublishMiniAppRequestDto,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        require_ui_only_release_mutation(&snapshot)?;
        require_no_running_build(&*self.repository, owner_user_id, &request.miniapp_id).await?;
        validate_publish_request(&snapshot, &request)?;

        let ready = snapshot
            .ready_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Ready Release".to_owned()))?;
        let stored = self.load_verified_release(owner_user_id, ready)?;
        if !stored.artifact.manifest.payload.is_ui_only() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "M1-0-02-B only publishes UI-only Releases".to_owned(),
            ));
        }

        let target = release_contract_ref(ready);
        let target_catalog_digest = materialized_catalog_digest(
            &request.miniapp_id,
            &target,
            &stored.artifact.manifest.payload.contributions,
        )?;
        let committed = self
            .repository
            .publish_ready_cas(&PublishMiniAppM1ReadyParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                expected_product_revision: to_i64(
                    request.expected_product_revision,
                    "product revision",
                )?,
                expected_pointer_revision: to_i64(
                    request.expected_pointer_revision,
                    "pointer revision",
                )?,
                expected_active_release_epoch: to_i64(
                    request.expected_active_release_epoch,
                    "active release epoch",
                )?,
                expected_ready_release_id: request.ready_release_id,
                expected_ready_release_digest: request.expected_ready_release_digest,
                expected_active_release_digest: request.expected_active_release_digest,
                target_catalog_digest: target_catalog_digest.as_ref().to_owned(),
                auto_publish_guard: None,
                updated_at: positive_now_ms(),
            })
            .await?;
        workshop_from_snapshot(&committed, None)
    }

    pub async fn rollback(
        &self,
        owner_user_id: &str,
        request: RollbackMiniAppRequestDto,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        require_ui_only_release_mutation(&snapshot)?;
        require_no_running_build(&*self.repository, owner_user_id, &request.miniapp_id).await?;
        validate_rollback_request(&snapshot, &request)?;

        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Active Release".to_owned()))?;
        let previous = snapshot
            .previous_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Previous Release".to_owned()))?;
        self.load_verified_release(owner_user_id, active)?;
        let target = self.load_verified_release(owner_user_id, previous)?;
        if !target.artifact.manifest.payload.is_ui_only() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "M1-0-02-B only rolls back UI-only Releases".to_owned(),
            ));
        }

        let rollback_target = release_contract_ref(previous);
        let target_catalog_digest = materialized_catalog_digest(
            &request.miniapp_id,
            &rollback_target,
            &target.artifact.manifest.payload.contributions,
        )?;
        let committed = self
            .repository
            .rollback_previous_cas(&RollbackMiniAppM1PreviousParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id,
                expected_product_revision: to_i64(
                    request.expected_product_revision,
                    "product revision",
                )?,
                expected_pointer_revision: to_i64(
                    request.expected_pointer_revision,
                    "pointer revision",
                )?,
                expected_active_release_epoch: to_i64(
                    request.expected_active_release_epoch,
                    "active release epoch",
                )?,
                expected_current_release_id: active.release_id.clone(),
                expected_current_release_digest: request.expected_current_release_digest,
                expected_previous_release_id: previous.release_id.clone(),
                expected_previous_release_digest: request.expected_previous_release_digest,
                target_catalog_digest: target_catalog_digest.as_ref().to_owned(),
                updated_at: positive_now_ms(),
            })
            .await?;
        workshop_from_snapshot(&committed, None)
    }

    pub async fn set_enabled(
        &self,
        owner_user_id: &str,
        request: SetMiniAppEnabledRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        require_ui_only_release_mutation(&snapshot)?;
        if snapshot.product.product_revision
            != to_i64(request.expected_product_revision, "product revision")?
            || snapshot.product.pointer_revision
                != to_i64(request.expected_pointer_revision, "pointer revision")?
            || snapshot.product.active_release_digest.as_deref()
                != request.expected_active_release_digest.as_deref()
        {
            return Err(MiniAppM1ApplicationError::Database(
                nomifun_db::DbError::Conflict(
                    "MiniApp lifecycle request is stale against the exact Product pointers"
                        .to_owned(),
                ),
            ));
        }
        if request.enabled && snapshot.product.active_release_id.is_none() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Enable requires an Active Release".to_owned(),
            ));
        }
        let expected_lifecycle = if request.enabled {
            "disabled"
        } else {
            "enabled"
        };
        if snapshot.product.lifecycle != expected_lifecycle {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "MiniApp is already {}",
                snapshot.product.lifecycle
            )));
        }
        let committed = self
            .repository
            .commit_lifecycle_cas(&CommitMiniAppM1LifecycleParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id,
                expected_product_revision: to_i64(
                    request.expected_product_revision,
                    "product revision",
                )?,
                expected_pointer_revision: to_i64(
                    request.expected_pointer_revision,
                    "pointer revision",
                )?,
                expected_active_release_digest: request.expected_active_release_digest,
                enabled: request.enabled,
                updated_at: positive_now_ms(),
            })
            .await?;
        workshop_from_snapshot(&committed, None)
    }

    pub async fn set_publish_mode(
        &self,
        owner_user_id: &str,
        request: SetMiniAppPublishModeRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        require_ui_only_release_mutation(&snapshot)?;
        if snapshot.product.product_revision
            != to_i64(request.expected_product_revision, "product revision")?
            || snapshot.product.pointer_revision
                != to_i64(request.expected_pointer_revision, "pointer revision")?
        {
            return Err(MiniAppM1ApplicationError::Database(
                nomifun_db::DbError::Conflict(
                    "MiniApp Publish mode request is stale against the exact Product pointers"
                        .to_owned(),
                ),
            ));
        }
        let enabled = request.mode == MiniAppPublishModeDto::AutoUiOnly;
        if enabled && snapshot.active_release.is_none() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "auto Publish can be enabled only after the first manual Publish".to_owned(),
            ));
        }
        let now = positive_now_ms();
        let (authorization_id, expected_authorization_revision, authorized_at) =
            match snapshot.auto_publish_authorization.as_ref() {
                Some(existing) => (
                    existing.authorization_id.clone(),
                    Some(existing.revision),
                    if enabled {
                        now
                    } else {
                        existing.user_authorized_at_ms
                    },
                ),
                None => (Uuid::now_v7().to_string(), None, now),
            };
        let committed = self
            .repository
            .set_auto_publish_cas(&SetMiniAppM1AutoPublishParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id,
                expected_product_revision: to_i64(
                    request.expected_product_revision,
                    "product revision",
                )?,
                expected_pointer_revision: to_i64(
                    request.expected_pointer_revision,
                    "pointer revision",
                )?,
                expected_authorization_revision,
                authorization_id,
                enabled,
                user_authorized_at_ms: authorized_at,
                updated_at: now,
            })
            .await?;
        let active_operation = latest_running_build(
            self.repository
                .list_build_operations(owner_user_id, &committed.product.miniapp_id)
                .await?,
        )?;
        workshop_from_snapshot(&committed, active_operation)
    }

    async fn auto_publish_ready(
        &self,
        owner_user_id: &str,
        snapshot: MiniAppM1Snapshot,
    ) -> Result<MiniAppM1Snapshot, MiniAppM1ApplicationError> {
        let authorization = snapshot
            .auto_publish_authorization
            .as_ref()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "auto Publish authorization disappeared before Build completion".to_owned(),
                )
            })?;
        if !authorization.enabled {
            return Ok(snapshot);
        }
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "first Publish must remain manual".to_owned(),
                )
            })?;
        let ready = snapshot
            .ready_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Ready Release".to_owned()))?;
        let current_stored = self.load_verified_release(owner_user_id, active)?;
        let target_stored = self.load_verified_release(owner_user_id, ready)?;
        let current_ref = release_contract_ref(active);
        let target_ref = release_contract_ref(ready);
        let current_non_ui = ui_only_non_ui_fingerprint(&current_stored)?;
        let target_non_ui = ui_only_non_ui_fingerprint(&target_stored)?;
        let current_project_id = active.project_id.as_deref();
        let target_project_id = ready.project_id.as_deref();
        if current_project_id.is_none() || current_project_id != target_project_id {
            return Ok(snapshot);
        }
        let project_id = current_project_id.unwrap_or_default();
        let current_source_digest = active.source_snapshot_digest.as_deref();
        let target_source_digest = ready.source_snapshot_digest.as_deref();
        if current_source_digest.is_none() || target_source_digest.is_none() {
            return Ok(snapshot);
        }
        let current_source = self
            .stores
            .source
            .read_revision_files(
                owner_user_id,
                &snapshot.product.miniapp_id,
                project_id,
                current_source_digest.unwrap_or_default(),
            )
            .map_err(|error| store_error("Active Source revision", error))?;
        let target_source = self
            .stores
            .source
            .read_revision_files(
                owner_user_id,
                &snapshot.product.miniapp_id,
                project_id,
                target_source_digest.unwrap_or_default(),
            )
            .map_err(|error| store_error("Ready Source revision", error))?;
        let changed_source_paths =
            changed_source_paths(&current_source, &target_source);
        let changed_output_paths =
            changed_output_paths(&current_stored, &target_stored);
        let no_unknown_changes =
            source_matches_artifact(&current_source, &current_stored)
                && source_matches_artifact(&target_source, &target_stored)
                && changed_source_paths == changed_output_paths;
        let project_head_matches_ready_source = snapshot
            .project
            .source_head_digest
            .as_deref()
            == ready.source_snapshot_digest.as_deref();
        if current_stored.artifact.manifest.payload.ui.ui_tree_digest
            == target_stored.artifact.manifest.payload.ui.ui_tree_digest
            || current_non_ui != target_non_ui
            || changed_source_paths.is_empty()
            || changed_output_paths.is_empty()
            || !project_head_matches_ready_source
            || !no_unknown_changes
        {
            return Ok(snapshot);
        }
        let proof = MiniAppUiOnlyAutoPublishProof {
            current_release: current_ref.clone(),
            target_release: target_ref.clone(),
            current_ui_tree_digest: current_stored
                .artifact
                .manifest
                .payload
                .ui
                .ui_tree_digest
                .clone(),
            target_ui_tree_digest: target_stored
                .artifact
                .manifest
                .payload
                .ui
                .ui_tree_digest
                .clone(),
            current_non_ui,
            target_non_ui,
            changed_source_paths,
            changed_output_paths,
            project_head_matches_ready_source,
            static_validation_passed: true,
            no_unknown_changes,
        };
        let contract_authorization = MiniAppUiOnlyAutoPublishAuthorization {
            authorization_id: MiniAppUserAuthorizationId::from(
                authorization.authorization_id.clone(),
            ),
            miniapp_id: MiniAppId::from(snapshot.product.miniapp_id.clone()),
            enabled: authorization.enabled,
            authorization_revision: u64::try_from(authorization.revision).map_err(|_| {
                MiniAppM1ApplicationError::Invalid(
                    "auto Publish authorization revision is negative".to_owned(),
                )
            })?,
            user_authorized_at_ms: authorization.user_authorized_at_ms,
        };
        let catalog_digest = materialized_catalog_digest(
            &snapshot.product.miniapp_id,
            &target_ref,
            &target_stored.artifact.manifest.payload.contributions,
        )?;
        let contract = MiniAppPublishContract {
            miniapp_id: MiniAppId::from(snapshot.product.miniapp_id.clone()),
            expected: pointer_expectation_from_snapshot(&snapshot)?,
            target_ready_release: target_ref,
            target_catalog_digest: catalog_digest.clone(),
            authorization: MiniAppPublishAuthorization::AutoUiOnly {
                authorization: contract_authorization,
                proof: Box::new(proof),
            },
        };
        let current_pointer = pointer_state_from_snapshot(&snapshot)?;
        if contract.next_state(&current_pointer).is_err() {
            return Ok(snapshot);
        }
        self.repository
            .publish_ready_cas(&PublishMiniAppM1ReadyParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: snapshot.product.miniapp_id.clone(),
                expected_product_revision: snapshot.product.product_revision,
                expected_pointer_revision: snapshot.product.pointer_revision,
                expected_active_release_epoch: snapshot.product.active_release_epoch,
                expected_ready_release_id: ready.release_id.clone(),
                expected_ready_release_digest: ready.release_digest.clone(),
                expected_active_release_digest: snapshot.product.active_release_digest.clone(),
                target_catalog_digest: catalog_digest.as_ref().to_owned(),
                auto_publish_guard: Some(MiniAppM1AutoPublishGuard {
                    authorization_id: authorization.authorization_id.clone(),
                    authorization_revision: authorization.revision,
                    project_id: snapshot.project.project_id.clone(),
                    project_revision: snapshot.project.project_revision,
                    source_head_digest: snapshot
                        .project
                        .source_head_digest
                        .clone()
                        .ok_or_else(|| {
                            MiniAppM1ApplicationError::Invalid(
                                "auto Publish requires an exact Project Source head".to_owned(),
                            )
                        })?,
                    dependency_lock_digest: snapshot
                        .project
                        .dependency_lock_digest
                        .clone()
                        .ok_or_else(|| {
                            MiniAppM1ApplicationError::Invalid(
                                "auto Publish requires an exact dependency lock".to_owned(),
                            )
                        })?,
                    build_profile_version: snapshot
                        .project
                        .build_profile_version
                        .clone()
                        .ok_or_else(|| {
                            MiniAppM1ApplicationError::Invalid(
                                "auto Publish requires an exact Build profile".to_owned(),
                            )
                        })?,
                    build_generation: snapshot.project.build_generation,
                }),
                updated_at: positive_now_ms(),
            })
            .await
            .map_err(Into::into)
    }

    pub async fn open_surface(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<MiniAppSurfaceLaunchDescriptorDto, MiniAppM1ApplicationError> {
        validate_request_identity(miniapp_id, "miniapp_id")?;
        let snapshot = self
            .repository
            .get(owner_user_id, miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if snapshot.product.lifecycle != "enabled" {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Surface is available only while enabled".to_owned(),
            ));
        }
        if snapshot.product.kind != MiniAppM1Kind::UiOnly.as_str() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Service Surface is deferred to M1-1".to_owned(),
            ));
        }
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Active Release".to_owned()))?;
        let stored = self.load_verified_release(owner_user_id, active)?;
        let entrypoint = stored.artifact.manifest.payload.ui.entrypoint.clone();
        if stored
            .files
            .iter()
            .all(|file| file.normalized_relative_path != entrypoint)
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Active Release is missing its UI entrypoint bytes".to_owned(),
            ));
        }
        let capability = issue_surface_capability()?;
        let capability_digest = surface_capability_digest(&capability)?;
        let session = self
            .repository
            .open_surface_session_cas(&OpenMiniAppM1SurfaceSessionParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: miniapp_id.to_owned(),
                surface_session_id: Uuid::now_v7().to_string(),
                capability_digest,
                expected_product_revision: snapshot.product.product_revision,
                expected_pointer_revision: snapshot.product.pointer_revision,
                expected_active_release_id: active.release_id.clone(),
                expected_active_release_digest: active.release_digest.clone(),
                expected_active_release_epoch: snapshot.product.active_release_epoch,
                issued_at_ms: positive_now_ms().max(snapshot.product.updated_at),
            })
            .await?;
        Ok(MiniAppSurfaceLaunchDescriptorDto {
            miniapp_id: miniapp_id.to_owned(),
            product_revision: positive_u64(
                snapshot.product.product_revision,
                "MiniApp product revision",
            )?,
            release_id: active.release_id.clone(),
            expected_release_digest: active.release_digest.clone(),
            active_release_epoch: positive_u64(
                snapshot.product.active_release_epoch,
                "MiniApp active release epoch",
            )?,
            surface_session_id: session.surface_session_id,
            surface_generation: positive_u64(
                session.generation,
                "MiniApp Surface generation",
            )?,
            surface_capability: capability,
            ui_entrypoint: entrypoint,
            kind: MiniAppKindDto::UiOnly,
        })
    }

    pub async fn close_surface(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        surface_session_id: &str,
        capability: &str,
    ) -> Result<bool, MiniAppM1ApplicationError> {
        validate_request_identity(miniapp_id, "miniapp_id")?;
        validate_request_identity(surface_session_id, "surface_session_id")?;
        let capability_digest = surface_capability_digest(capability)?;
        self.repository
            .close_surface_session_cas(&CloseMiniAppM1SurfaceSessionParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: miniapp_id.to_owned(),
                surface_session_id: surface_session_id.to_owned(),
                capability_digest,
            })
            .await
            .map_err(Into::into)
    }

    pub async fn surface_asset(
        &self,
        miniapp_id: &str,
        capability: &str,
        active_release_epoch: u64,
        expected_release_digest: &str,
        asset_path: &str,
    ) -> Result<MiniAppSurfaceAsset, MiniAppM1ApplicationError> {
        let session = self
            .resolve_surface_session(
                miniapp_id,
                capability,
                active_release_epoch,
                expected_release_digest,
            )
            .await?;
        let snapshot = self
            .repository
            .get(&session.owner_user_id, miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if snapshot.product.lifecycle != "enabled"
            || snapshot.product.kind != MiniAppM1Kind::UiOnly.as_str()
            || nonnegative_u64(
                snapshot.product.active_release_epoch,
                "MiniApp active release epoch",
            )? != active_release_epoch
            || snapshot.product.active_release_digest.as_deref()
                != Some(expected_release_digest)
        {
            return Err(MiniAppM1ApplicationError::NotFound);
        }
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if active.release_id != session.active_release_id {
            return Err(MiniAppM1ApplicationError::NotFound);
        }
        let stored = self.load_verified_release(&session.owner_user_id, active)?;
        let file = stored
            .files
            .into_iter()
            .find(|file| file.normalized_relative_path == asset_path)
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        let observed = self
            .resolve_surface_session(
                miniapp_id,
                capability,
                active_release_epoch,
                expected_release_digest,
            )
            .await?;
        if observed.surface_session_id != session.surface_session_id
            || observed.generation != session.generation
        {
            return Err(MiniAppM1ApplicationError::NotFound);
        }
        Ok(MiniAppSurfaceAsset {
            normalized_relative_path: file.normalized_relative_path,
            bytes: file.bytes,
        })
    }

    pub async fn surface_bridge_request(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        capability: &str,
        active_release_epoch: u64,
        expected_release_digest: &str,
        request: MiniAppBridgeRequest,
    ) -> Result<StrictJsonValue, MiniAppM1ApplicationError> {
        let surface_session = self
            .resolve_surface_session(
                miniapp_id,
                capability,
                active_release_epoch,
                expected_release_digest,
            )
            .await?;
        if surface_session.owner_user_id != owner_user_id {
            return Err(MiniAppM1ApplicationError::NotFound);
        }
        let snapshot = self
            .repository
            .get(owner_user_id, miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if snapshot.product.lifecycle != "enabled"
            || snapshot.product.kind != MiniAppM1Kind::UiOnly.as_str()
            || positive_u64(
                snapshot.product.active_release_epoch,
                "MiniApp active release epoch",
            )? != active_release_epoch
            || snapshot.product.active_release_digest.as_deref()
                != Some(expected_release_digest)
        {
            return Err(MiniAppM1ApplicationError::NotFound);
        }
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if active.release_id != surface_session.active_release_id {
            return Err(MiniAppM1ApplicationError::NotFound);
        }
        self.load_verified_release(owner_user_id, active)?;
        let pointer = pointer_state_from_snapshot(&snapshot)?;
        let session = MiniAppBridgeSession {
            bridge_contract_version: MINIAPP_BRIDGE_CONTRACT_VERSION.into(),
            bridge_session_id: MiniAppBridgeSessionId::from(
                surface_session.surface_session_id.clone(),
            ),
            surface_session_id: MiniAppSurfaceSessionId::from(
                surface_session.surface_session_id.clone(),
            ),
            miniapp_id: MiniAppId::from(miniapp_id),
            active_release: release_contract_ref(active),
            active_release_epoch,
            transport: MiniAppBridgeTransport::MessageChannelV1,
            service_run_key: None,
        };
        request
            .validate_for(&session, &pointer)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        match request.target {
            MiniAppBridgeTarget::HostKv { request } => {
                self.execute_surface_kv(
                    owner_user_id,
                    miniapp_id,
                    &surface_session,
                    request,
                )
                .await
            }
            MiniAppBridgeTarget::Service { .. } => Err(MiniAppM1ApplicationError::Invalid(
                "UI-only MiniApp Bridge cannot invoke a Service".to_owned(),
            )),
        }
    }

    pub async fn workshop(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let snapshot = self
            .repository
            .get(owner_user_id, miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        let active_operation = latest_running_build(
            self.repository
                .list_build_operations(owner_user_id, miniapp_id)
                .await?,
        )?;
        workshop_from_snapshot(&snapshot, active_operation)
    }

    fn load_verified_release(
        &self,
        owner_user_id: &str,
        release: &MiniAppReleaseRow,
    ) -> Result<MiniAppStoredRelease, MiniAppM1ApplicationError> {
        let project_id = release.project_id.as_deref().ok_or_else(|| {
            MiniAppM1ApplicationError::Invalid(
                "M1-0-02-B requires a managed Release with Project lineage".to_owned(),
            )
        })?;
        let scope = MiniAppSourceScope::new(owner_user_id, &release.miniapp_id, project_id)
            .map_err(|error| store_error("Release scope", error))?;
        let expected_artifact_identity = MiniAppReleaseArtifactIdentity {
            artifact_id: ArtifactId::from(release.artifact_id.clone()),
            artifact_digest: DigestHex::from(release.artifact_digest.clone()),
            manifest_digest: DigestHex::from(release.manifest_digest.clone()),
        };
        let stored = self
            .stores
            .release
            .load_exact(scope, &expected_artifact_identity)
            .map_err(|error| store_error("Release Store", error))?;
        if stored.artifact.artifact_id.as_ref() != release.artifact_id
            || stored.artifact.artifact_digest.as_ref() != release.artifact_digest
            || stored.artifact.manifest.payload_digest.as_ref() != release.manifest_digest
            || release.release_digest != release.artifact_digest
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Release Store bytes do not match the exact database Release".to_owned(),
            ));
        }
        let record: MiniAppReadyRelease =
            serde_json::from_str(&release.release_record_json).map_err(|error| {
                MiniAppM1ApplicationError::Invalid(format!(
                    "database Release record cannot be decoded: {error}"
                ))
            })?;
        record
            .validate_for_artifact(&stored.artifact)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let MiniAppSourceLineage::Managed {
            project_id: record_project_id,
            source_snapshot_digest,
            dependency_lock_digest,
            build_profile_version,
            build_generation,
        } = &record.source_lineage
        else {
            return Err(MiniAppM1ApplicationError::Invalid(
                "managed database Release has a runtime-only Release record".to_owned(),
            ));
        };
        if record.miniapp_id.as_ref() != release.miniapp_id
            || record.release.release_id.as_ref() != release.release_id
            || record.release.artifact_id.as_ref() != release.artifact_id
            || record.release.release_digest.as_ref() != release.release_digest
            || record.release.manifest_digest.as_ref() != release.manifest_digest
            || record.origin != MiniAppReadyOrigin::Build
            || record.origin_operation_id.as_ref() != release.origin_operation_id
            || record_project_id.as_ref() != project_id
            || source_snapshot_digest.as_ref()
                != release.source_snapshot_digest.as_deref().unwrap_or_default()
            || dependency_lock_digest.as_ref()
                != release.dependency_lock_digest.as_deref().unwrap_or_default()
            || build_profile_version.as_ref()
                != release.build_profile_version.as_deref().unwrap_or_default()
            || i64::try_from(*build_generation).ok() != release.build_generation
            || record.created_at_ms != release.created_at
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "database Release row does not match its canonical Release record".to_owned(),
            ));
        }
        Ok(stored)
    }

    async fn resolve_surface_session(
        &self,
        miniapp_id: &str,
        capability: &str,
        active_release_epoch: u64,
        expected_release_digest: &str,
    ) -> Result<MiniAppSurfaceSessionRow, MiniAppM1ApplicationError> {
        validate_request_identity(miniapp_id, "miniapp_id")?;
        validate_digest_string(expected_release_digest, "expected Release digest")?;
        let capability_digest = surface_capability_digest(capability)?;
        self.repository
            .resolve_surface_session(&ResolveMiniAppM1SurfaceSessionParams {
                miniapp_id: miniapp_id.to_owned(),
                capability_digest,
                expected_active_release_digest: expected_release_digest.to_owned(),
                expected_active_release_epoch: to_i64(
                    active_release_epoch,
                    "active release epoch",
                )?,
            })
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)
    }

    async fn execute_surface_kv(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        session: &MiniAppSurfaceSessionRow,
        request: MiniAppBridgeKvRequest,
    ) -> Result<StrictJsonValue, MiniAppM1ApplicationError> {
        const NAMESPACE: &str = "surface";
        let (key, operation) = match request {
            MiniAppBridgeKvRequest::Get { key } => (key, MiniAppM1SurfaceKvOperation::Get),
            MiniAppBridgeKvRequest::Set { key, value } => (
                key,
                MiniAppM1SurfaceKvOperation::Set { value: value.0 },
            ),
            MiniAppBridgeKvRequest::Delete { key } => {
                (key, MiniAppM1SurfaceKvOperation::Delete)
            }
            MiniAppBridgeKvRequest::CompareAndSwap {
                key,
                expected_revision,
                value,
            } => (
                key,
                MiniAppM1SurfaceKvOperation::CompareAndSwap {
                    expected_revision: expected_revision
                        .map(|revision| to_i64(revision, "MiniApp KV revision"))
                        .transpose()?,
                    value: value.map(|value| value.0),
                },
            ),
        };
        let response = match self
            .repository
            .execute_surface_kv(&ExecuteMiniAppM1SurfaceKvParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: miniapp_id.to_owned(),
                surface_session_id: session.surface_session_id.clone(),
                expected_surface_generation: session.generation,
                expected_capability_digest: session.capability_digest.clone(),
                expected_active_release_epoch: session.active_release_epoch,
                expected_active_release_digest: session.active_release_digest.clone(),
                namespace: NAMESPACE.to_owned(),
                key,
                operation,
                updated_at: positive_now_ms(),
            })
            .await?
        {
            MiniAppM1SurfaceKvResult::Value { value, revision } => MiniAppKvResponse::Value {
                value: value.map(StrictJsonValue),
                revision: revision
                    .map(|value| positive_u64(value, "MiniApp KV revision"))
                    .transpose()?,
            },
            MiniAppM1SurfaceKvResult::Written { revision } => {
                MiniAppKvResponse::Written {
                    revision: positive_u64(revision, "MiniApp KV revision")?,
                }
            }
            MiniAppM1SurfaceKvResult::Deleted { existed } => {
                MiniAppKvResponse::Deleted { existed }
            }
            MiniAppM1SurfaceKvResult::CompareAndSwap {
                applied,
                current_revision,
            } => MiniAppKvResponse::CompareAndSwap {
                applied,
                current_revision: current_revision
                    .map(|value| positive_u64(value, "MiniApp KV revision"))
                    .transpose()?,
            },
        };
        serde_json::to_value(response)
            .map(StrictJsonValue)
            .map_err(|error| {
                MiniAppM1ApplicationError::Invalid(format!(
                    "MiniApp KV response cannot be serialized: {error}"
                ))
            })
    }

    async fn finish_failed_build(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        operation_id: &str,
        original: MiniAppM1ApplicationError,
    ) -> MiniAppM1ApplicationError {
        let finished_at_ms = positive_now_ms();
        let result = self
            .repository
            .finish_build_operation(&FinishMiniAppM1BuildOperationParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: miniapp_id.to_owned(),
                operation_id: operation_id.to_owned(),
                state: ProductOperationState::Failed,
                progress_percent: 0,
                last_error_code: Some(build_error_code(&original).to_owned()),
                bounded_log_tail: vec![bounded_log_line(&original.to_string())],
                finished_at_ms,
            })
            .await;
        match result {
            Ok(_) => original,
            Err(finish_error) => MiniAppM1ApplicationError::Invalid(format!(
                "{original}; failed to record terminal Build state: {finish_error}"
            )),
        }
    }
}

struct PreparedBuildRelease {
    artifact: MiniAppReleaseArtifactRow,
    release: MiniAppReleaseRow,
    finished_at_ms: i64,
}

fn prepare_build_release(
    owner_user_id: &str,
    snapshot: &MiniAppM1Snapshot,
    request: &BuildMiniAppRequest,
    source: &MiniAppSourceSnapshot,
    operation_id: &str,
    started_at_ms: i64,
    release_store: &MiniAppReleaseStore,
) -> Result<PreparedBuildRelease, MiniAppM1ApplicationError> {
    let lock: MiniAppDependencyLockV1 =
        serde_json::from_slice(&source.dependency_lock).map_err(|error| {
            MiniAppM1ApplicationError::Invalid(format!(
                "canonical dependency lock cannot be decoded: {error}"
            ))
        })?;
    if !lock.dependencies.is_empty() {
        return Err(MiniAppM1ApplicationError::Invalid(
            "M1-0-02-A accepts only the empty UI-only dependency lock".to_owned(),
        ));
    }

    let mut source_files = source
        .files
        .iter()
        .map(|file| {
            (
                file.normalized_relative_path.clone(),
                file.bytes.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let ui_index_html = source_files.remove("ui/index.html").ok_or_else(|| {
        MiniAppM1ApplicationError::Invalid(
            "UI-only Source must contain ui/index.html".to_owned(),
        )
    })?;
    let materialized_ui_index_html = materialize_surface_entrypoint(&ui_index_html)
        .map_err(|error| {
            MiniAppM1ApplicationError::Invalid(format!(
                "UI-only Surface Bridge bootstrap failed: {error}"
            ))
        })?;
    let ui_assets = source_files
        .into_iter()
        .map(|(path, bytes)| MiniAppStaticBundleFile::new(path, bytes))
        .collect();
    let config_schema: Value = serde_json::from_str(&snapshot.product.config_schema_json)
        .map_err(|error| {
            MiniAppM1ApplicationError::Invalid(format!(
                "MiniApp config schema is invalid: {error}"
            ))
        })?;
    let artifact = MiniAppStaticBundleBuilder::new()
        .build_ui_only(MiniAppStaticBundleInput {
            artifact_id: ArtifactId::from(Uuid::now_v7().to_string()),
            display: LocalizedMetadata {
                name: snapshot.product.display_name.clone(),
                description: snapshot
                    .product
                    .description
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| snapshot.product.display_name.clone()),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            ui_index_html,
            ui_assets,
            service: None,
            package_json: None,
            dependency_lock_digest: source.dependency_lock_digest.clone(),
            dependency_graph_digest: digest_payload(&lock.dependencies).map_err(|error| {
                MiniAppM1ApplicationError::Invalid(error.to_string())
            })?,
            config_schema: StrictJsonValue(config_schema),
            credential_slots: Vec::new(),
            resource_contract: MiniAppResourceContract::default(),
            schemas: BTreeMap::new(),
            bridge_contract_digest: digest_payload(&MINIAPP_BRIDGE_CONTRACT_VERSION)
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?,
            contribution_package: PackageRef {
                id: PackageId::from(format!("miniapp.{}", request.miniapp_id)),
                version: VersionString::from("1.0.0"),
            },
            contributions: Default::default(),
            migrations: Vec::new(),
        })
        .map_err(|error| {
            MiniAppM1ApplicationError::Invalid(format!("fixed UI-only Build failed: {error}"))
        })?;
    let file_bytes = artifact
        .files
        .iter()
        .map(|file| {
            let bytes = if file.normalized_relative_path == "ui/index.html" {
                materialized_ui_index_html.clone()
            } else {
                source
                    .file(&file.normalized_relative_path)
                    .ok_or_else(|| {
                        MiniAppM1ApplicationError::Invalid(format!(
                            "captured Source file disappeared: {}",
                            file.normalized_relative_path
                        ))
                    })?
                    .to_vec()
            };
            Ok(MiniAppReleaseFileBytes::new(
                file.normalized_relative_path.clone(),
                bytes,
            ))
        })
        .collect::<Result<Vec<_>, MiniAppM1ApplicationError>>()?;
    let scope = crate::MiniAppSourceScope::new(
        owner_user_id,
        &request.miniapp_id,
        &request.project_id,
    )
    .map_err(|error| store_error("Source scope", error))?;
    let published = release_store
        .publish(MiniAppReleasePublishRequest::ui_only(
            scope,
            source.source_snapshot_digest.clone(),
            source.dependency_lock_digest.clone(),
            request.expected_build_generation,
            artifact,
            file_bytes,
        ))
        .map_err(|error| store_error("Release Store", error))?;
    let artifact = published.stored.artifact;
    let finished_at_ms = positive_now_ms().max(started_at_ms);
    let release_id = Uuid::now_v7().to_string();
    let ready = MiniAppReadyRelease {
        miniapp_id: MiniAppId::from(request.miniapp_id.clone()),
        release: MiniAppReleaseRef {
            release_id: MiniAppReleaseId::from(release_id.clone()),
            artifact_id: artifact.artifact_id.clone(),
            release_digest: artifact.artifact_digest.clone(),
            manifest_digest: artifact.manifest.payload_digest.clone(),
        },
        origin_operation_id: OperationId::from(operation_id.to_owned()),
        origin: MiniAppReadyOrigin::Build,
        source_lineage: MiniAppSourceLineage::Managed {
            project_id: MiniAppProjectId::from(request.project_id.clone()),
            source_snapshot_digest: source.source_snapshot_digest.clone(),
            dependency_lock_digest: source.dependency_lock_digest.clone(),
            build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
            build_generation: request.expected_build_generation,
        },
        matching_service_test_receipt: None,
        created_at_ms: finished_at_ms,
    };
    ready
        .validate_for_artifact(&artifact)
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
    Ok(PreparedBuildRelease {
        artifact: MiniAppReleaseArtifactRow {
            id: 0,
            artifact_id: artifact.artifact_id.as_ref().to_owned(),
            owner_user_id: owner_user_id.to_owned(),
            artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
            manifest_digest: artifact.manifest.payload_digest.as_ref().to_owned(),
            artifact_record_json: canonical_json_string(&artifact)?,
            managed_path: published.stored.managed_relative_path,
            created_at: finished_at_ms,
        },
        release: MiniAppReleaseRow {
            id: 0,
            release_id,
            miniapp_id: request.miniapp_id.clone(),
            owner_user_id: owner_user_id.to_owned(),
            artifact_id: artifact.artifact_id.as_ref().to_owned(),
            artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
            manifest_digest: artifact.manifest.payload_digest.as_ref().to_owned(),
            release_digest: artifact.artifact_digest.as_ref().to_owned(),
            origin_kind: "build".to_owned(),
            origin_operation_id: operation_id.to_owned(),
            source_kind: "managed".to_owned(),
            project_id: Some(request.project_id.clone()),
            source_snapshot_digest: Some(source.source_snapshot_digest.as_ref().to_owned()),
            dependency_lock_digest: Some(source.dependency_lock_digest.as_ref().to_owned()),
            build_profile_version: Some(MINIAPP_RELEASE_PROFILE_VERSION.to_owned()),
            build_generation: Some(to_i64(
                request.expected_build_generation,
                "build generation",
            )?),
            release_record_json: canonical_json_string(&ready)?,
            created_at: finished_at_ms,
        },
        finished_at_ms,
    })
}

fn validate_build_request(
    snapshot: &MiniAppM1Snapshot,
    request: &BuildMiniAppRequest,
) -> Result<(), MiniAppM1ApplicationError> {
    validate_digest_string(
        &request.expected_source_snapshot_digest,
        "expected source snapshot digest",
    )?;
    validate_digest_string(
        &request.expected_dependency_lock_digest,
        "expected dependency lock digest",
    )?;
    if snapshot.product.miniapp_id != request.miniapp_id
        || snapshot.project.project_id != request.project_id
    {
        return Err(MiniAppM1ApplicationError::Invalid(
            "Build identity does not match the owner-scoped Product/Project".to_owned(),
        ));
    }
    if snapshot.product.kind != MiniAppM1Kind::UiOnly.as_str() {
        return Err(MiniAppM1ApplicationError::Invalid(
            "M1-0-02-A only builds UI-only MiniApps".to_owned(),
        ));
    }
    if matches!(snapshot.product.lifecycle.as_str(), "trashed" | "deleting") {
        return Err(MiniAppM1ApplicationError::Invalid(
            "trashed or deleting MiniApps cannot be built".to_owned(),
        ));
    }
    if snapshot.product.product_revision
        != to_i64(request.expected_product_revision, "product revision")?
        || snapshot.project.project_revision
            != to_i64(request.expected_project_revision, "project revision")?
        || snapshot.project.build_generation
            != to_i64(request.expected_build_generation, "build generation")?
        || snapshot.project.source_state != "editable"
        || snapshot.project.source_head_digest.as_deref()
            != Some(request.expected_source_snapshot_digest.as_str())
        || snapshot.project.dependency_lock_digest.as_deref()
            != Some(request.expected_dependency_lock_digest.as_str())
        || snapshot.project.build_profile_version.as_deref()
            != Some(MINIAPP_RELEASE_PROFILE_VERSION)
    {
        return Err(MiniAppM1ApplicationError::Invalid(
            "Build request is stale against the exact Product/Project/Source head".to_owned(),
        ));
    }
    for (label, release) in [
        ("Ready", snapshot.ready_release.as_ref()),
        ("Active", snapshot.active_release.as_ref()),
        ("Previous", snapshot.previous_release.as_ref()),
    ] {
        if release.is_some_and(|release| {
            release.project_id.as_deref() == Some(request.project_id.as_str())
                && release.source_snapshot_digest.as_deref()
                    == Some(request.expected_source_snapshot_digest.as_str())
                && release.dependency_lock_digest.as_deref()
                    == Some(request.expected_dependency_lock_digest.as_str())
                && release.build_generation
                    == Some(i64::try_from(request.expected_build_generation).unwrap_or(i64::MIN))
        }) {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "the current {label} Release already represents this exact Source generation"
            )));
        }
    }
    Ok(())
}

fn read_exact_source(
    store: &MiniAppSourceStore,
    owner_user_id: &str,
    snapshot: &MiniAppM1Snapshot,
    request: &BuildMiniAppRequest,
) -> Result<MiniAppSourceSnapshot, MiniAppM1ApplicationError> {
    let source = store
        .read_snapshot(
            owner_user_id,
            &request.miniapp_id,
            &request.project_id,
            &request.expected_source_snapshot_digest,
        )
        .map_err(|error| store_error("Source Store", error))?;
    if source.dependency_lock_digest.as_ref() != request.expected_dependency_lock_digest
        || source.project.build_generation != request.expected_build_generation
        || source.project.build_profile != JavaScriptBuildProfile::MiniAppReleaseV1
        || source.project.build_profile_version.as_ref()
            != MINIAPP_RELEASE_PROFILE_VERSION
        || snapshot.project.managed_source_path.as_deref()
            != Some(source.project.managed_relative_path.as_str())
    {
        return Err(MiniAppM1ApplicationError::Invalid(
            "Source Store head does not match the exact DB Project lineage".to_owned(),
        ));
    }
    Ok(source)
}

fn managed_source_lineage(
    snapshot: &MiniAppM1Snapshot,
) -> Result<MiniAppM1ManagedSourceLineage, MiniAppM1ApplicationError> {
    Ok(MiniAppM1ManagedSourceLineage {
        managed_source_path: snapshot
            .project
            .managed_source_path
            .clone()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "UI-only Build requires a managed Source path".to_owned(),
                )
            })?,
        source_head_digest: snapshot
            .project
            .source_head_digest
            .clone()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "UI-only Build requires a Source digest".to_owned(),
                )
            })?,
        dependency_lock_digest: snapshot
            .project
            .dependency_lock_digest
            .clone()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "UI-only Build requires a dependency lock digest".to_owned(),
                )
            })?,
        build_profile_version: snapshot
            .project
            .build_profile_version
            .clone()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "UI-only Build requires a build profile".to_owned(),
                )
            })?,
        build_generation: snapshot.project.build_generation,
    })
}

async fn require_no_running_build(
    repository: &dyn IMiniAppM1Repository,
    owner_user_id: &str,
    miniapp_id: &str,
) -> Result<(), MiniAppM1ApplicationError> {
    if repository
        .list_build_operations(owner_user_id, miniapp_id)
        .await?
        .into_iter()
        .any(|operation| operation.state == ProductOperationState::Running.as_str())
    {
        return Err(MiniAppM1ApplicationError::Database(
            nomifun_db::DbError::Conflict(
                "MiniApp Release pointers cannot change while a Build is running".to_owned(),
            ),
        ));
    }
    Ok(())
}

fn require_ui_only_release_mutation(
    snapshot: &MiniAppM1Snapshot,
) -> Result<(), MiniAppM1ApplicationError> {
    if snapshot.product.kind != MiniAppM1Kind::UiOnly.as_str() {
        return Err(MiniAppM1ApplicationError::Invalid(
            "M1-0-02-B supports UI-only MiniApps; Service cutover belongs to M1-1".to_owned(),
        ));
    }
    if matches!(snapshot.product.lifecycle.as_str(), "trashed" | "deleting") {
        return Err(MiniAppM1ApplicationError::Invalid(
            "trashed or deleting MiniApps cannot change Release pointers".to_owned(),
        ));
    }
    Ok(())
}

fn validate_publish_request(
    snapshot: &MiniAppM1Snapshot,
    request: &PublishMiniAppRequestDto,
) -> Result<(), MiniAppM1ApplicationError> {
    validate_request_identity(&request.miniapp_id, "miniapp_id")?;
    validate_digest_string(
        &request.expected_ready_release_digest,
        "expected Ready Release digest",
    )?;
    if let Some(active) = &request.expected_active_release_digest {
        validate_digest_string(active, "expected Active Release digest")?;
    }
    if request.expected_service_test_receipt_id.is_some() || request.acknowledge_test_warning {
        return Err(MiniAppM1ApplicationError::Invalid(
            "UI-only Publish does not accept Service Test warnings or receipts".to_owned(),
        ));
    }
    let ready = snapshot
        .ready_release
        .as_ref()
        .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Ready Release".to_owned()))?;
    if snapshot.product.miniapp_id != request.miniapp_id
        || positive_u64(snapshot.product.product_revision, "MiniApp product revision")?
            != request.expected_product_revision
        || positive_u64(snapshot.product.pointer_revision, "MiniApp pointer revision")?
            != request.expected_pointer_revision
        || nonnegative_u64(
            snapshot.product.active_release_epoch,
            "MiniApp active release epoch",
        )? != request.expected_active_release_epoch
        || ready.release_id != request.ready_release_id
        || ready.release_digest != request.expected_ready_release_digest
        || snapshot.product.active_release_digest.as_deref()
            != request.expected_active_release_digest.as_deref()
    {
        return Err(MiniAppM1ApplicationError::Database(
            nomifun_db::DbError::Conflict(
                "MiniApp Publish request is stale against the exact Release pointers".to_owned(),
            ),
        ));
    }
    Ok(())
}

fn validate_rollback_request(
    snapshot: &MiniAppM1Snapshot,
    request: &RollbackMiniAppRequestDto,
) -> Result<(), MiniAppM1ApplicationError> {
    validate_request_identity(&request.miniapp_id, "miniapp_id")?;
    validate_digest_string(
        &request.expected_current_release_digest,
        "expected current Release digest",
    )?;
    validate_digest_string(
        &request.expected_previous_release_digest,
        "expected Previous Release digest",
    )?;
    let active = snapshot
        .active_release
        .as_ref()
        .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Active Release".to_owned()))?;
    let previous = snapshot
        .previous_release
        .as_ref()
        .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Previous Release".to_owned()))?;
    if snapshot.product.miniapp_id != request.miniapp_id
        || positive_u64(snapshot.product.product_revision, "MiniApp product revision")?
            != request.expected_product_revision
        || positive_u64(snapshot.product.pointer_revision, "MiniApp pointer revision")?
            != request.expected_pointer_revision
        || positive_u64(
            snapshot.product.active_release_epoch,
            "MiniApp active release epoch",
        )? != request.expected_active_release_epoch
        || active.release_digest != request.expected_current_release_digest
        || previous.release_id != request.previous_release_id
        || previous.release_digest != request.expected_previous_release_digest
    {
        return Err(MiniAppM1ApplicationError::Database(
            nomifun_db::DbError::Conflict(
                "MiniApp Rollback request is stale against the exact Release pointers".to_owned(),
            ),
        ));
    }
    Ok(())
}

fn validate_request_identity(
    value: &str,
    label: &str,
) -> Result<(), MiniAppM1ApplicationError> {
    nomifun_common::validate_uuidv7(value)
        .map(|_| ())
        .map_err(|error| {
            MiniAppM1ApplicationError::Invalid(format!(
                "{label} must be canonical UUIDv7: {error}"
            ))
        })
}

fn pointer_state_from_snapshot(
    snapshot: &MiniAppM1Snapshot,
) -> Result<MiniAppReleasePointerState, MiniAppM1ApplicationError> {
    validate_digest_string(
        &snapshot.product.materialized_catalog_digest,
        "materialized Catalog digest",
    )?;
    let state = MiniAppReleasePointerState {
        miniapp_id: MiniAppId::from(snapshot.product.miniapp_id.clone()),
        pointer_revision: positive_u64(
            snapshot.product.pointer_revision,
            "MiniApp pointer revision",
        )?,
        active_release_epoch: nonnegative_u64(
            snapshot.product.active_release_epoch,
            "MiniApp active release epoch",
        )?,
        ready_release: snapshot.ready_release.as_ref().map(|release| {
            MiniAppReadyReleaseRef {
                release_id: MiniAppReleaseId::from(release.release_id.clone()),
                release_digest: DigestHex::from(release.release_digest.clone()),
            }
        }),
        active_release: snapshot.active_release.as_ref().map(release_contract_ref),
        previous_release: snapshot
            .previous_release
            .as_ref()
            .map(release_contract_ref),
        materialized_catalog_digest: DigestHex::from(
            snapshot.product.materialized_catalog_digest.clone(),
        ),
    };
    state
        .validate()
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
    Ok(state)
}

fn pointer_expectation_from_snapshot(
    snapshot: &MiniAppM1Snapshot,
) -> Result<MiniAppPointerExpectation, MiniAppM1ApplicationError> {
    Ok(MiniAppPointerExpectation::from_state(
        &pointer_state_from_snapshot(snapshot)?,
    ))
}

fn ui_only_non_ui_fingerprint(
    release: &MiniAppStoredRelease,
) -> Result<MiniAppNonUiReleaseFingerprint, MiniAppM1ApplicationError> {
    let manifest = &release.artifact.manifest.payload;
    if !manifest.is_ui_only() {
        return Err(MiniAppM1ApplicationError::Invalid(
            "UI-only auto Publish proof received a Service Release".to_owned(),
        ));
    }
    Ok(MiniAppNonUiReleaseFingerprint {
        manifest_without_ui_digest: ui_only_non_ui_manifest_digest(release)?,
        service_run_key: None,
        migration_set_digest: manifest
            .migration_set_digest()
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?,
        contribution_set_digest: manifest
            .contribution_set_digest()
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?,
        bridge_contract_digest: manifest.bridge_contract_digest.clone(),
        config_schema_digest: manifest.config_schema_digest.clone(),
        credential_slots_digest: manifest.credential_slots_digest.clone(),
        resource_contract_digest: manifest.resource_contract_digest.clone(),
        runtime_requirements_digest: digest_bytes(b"miniapp-ui-only-no-runtime"),
        dependency_lock_digest: manifest.dependency_lock_digest.clone(),
    })
}

fn ui_only_non_ui_manifest_digest(
    release: &MiniAppStoredRelease,
) -> Result<DigestHex, MiniAppM1ApplicationError> {
    let mut manifest = release.artifact.manifest.payload.clone();
    manifest.ui.entrypoint_digest = digest_bytes(b"normalized-ui-entrypoint-content");
    manifest.ui.ui_tree_digest = digest_bytes(b"normalized-ui-tree-content");
    digest_payload(&manifest)
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))
}

fn changed_output_paths(
    current: &MiniAppStoredRelease,
    target: &MiniAppStoredRelease,
) -> BTreeSet<String> {
    let current = current
        .artifact
        .files
        .iter()
        .map(|file| {
            (
                file.normalized_relative_path.as_str(),
                file.digest.as_ref(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let target = target
        .artifact
        .files
        .iter()
        .map(|file| {
            (
                file.normalized_relative_path.as_str(),
                file.digest.as_ref(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    current
        .keys()
        .chain(target.keys())
        .filter(|path| current.get(**path) != target.get(**path))
        .map(|path| (*path).to_owned())
        .collect()
}

fn changed_source_paths(
    current: &[MiniAppSourceFile],
    target: &[MiniAppSourceFile],
) -> BTreeSet<String> {
    let current = current
        .iter()
        .map(|file| {
            (
                file.normalized_relative_path.as_str(),
                file.digest.as_ref(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let target = target
        .iter()
        .map(|file| {
            (
                file.normalized_relative_path.as_str(),
                file.digest.as_ref(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    current
        .keys()
        .chain(target.keys())
        .filter(|path| current.get(**path) != target.get(**path))
        .map(|path| (*path).to_owned())
        .collect()
}

fn source_matches_artifact(
    source: &[MiniAppSourceFile],
    release: &MiniAppStoredRelease,
) -> bool {
    let source = source
        .iter()
        .filter_map(|file| {
            if file.normalized_relative_path == "ui/index.html" {
                let bytes = materialize_surface_entrypoint(&file.bytes).ok()?;
                Some((
                    file.normalized_relative_path.as_str(),
                    (digest_bytes(&bytes), bytes.len() as u64),
                ))
            } else {
                Some((
                    file.normalized_relative_path.as_str(),
                    (file.digest.clone(), file.size_bytes),
                ))
            }
        })
        .collect::<BTreeMap<_, _>>();
    let artifact = release
        .artifact
        .files
        .iter()
        .map(|file| {
            (
                file.normalized_relative_path.as_str(),
                (file.digest.clone(), file.size_bytes),
            )
        })
        .collect::<BTreeMap<_, _>>();
    source.iter()
        .map(|(path, (digest, size))| (*path, (digest.as_ref(), *size)))
        .collect::<BTreeMap<_, _>>()
        == artifact
            .iter()
            .map(|(path, (digest, size))| (*path, (digest.as_ref(), *size)))
            .collect::<BTreeMap<_, _>>()
}

fn release_contract_ref(release: &MiniAppReleaseRow) -> MiniAppReleaseRef {
    MiniAppReleaseRef {
        release_id: MiniAppReleaseId::from(release.release_id.clone()),
        artifact_id: ArtifactId::from(release.artifact_id.clone()),
        release_digest: DigestHex::from(release.release_digest.clone()),
        manifest_digest: DigestHex::from(release.manifest_digest.clone()),
    }
}

#[derive(Serialize)]
struct MaterializedMiniAppCatalogDigest<'a> {
    miniapp_id: &'a str,
    active_release: &'a MiniAppReleaseRef,
    contributions: &'a PackageContributions,
}

fn materialized_catalog_digest(
    miniapp_id: &str,
    active_release: &MiniAppReleaseRef,
    contributions: &PackageContributions,
) -> Result<DigestHex, MiniAppM1ApplicationError> {
    digest_payload(&MaterializedMiniAppCatalogDigest {
        miniapp_id,
        active_release,
        contributions,
    })
    .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))
}

fn summary_from_product(
    product: &MiniAppProductRow,
) -> Result<MiniAppSummaryDto, MiniAppM1ApplicationError> {
    let kind = kind_dto(&product.kind)?;
    let lifecycle = lifecycle_dto(&product.lifecycle)?;
    let product_revision =
        positive_u64(product.product_revision, "MiniApp product revision")?;
    let pointer_revision =
        positive_u64(product.pointer_revision, "MiniApp pointer revision")?;
    let active_release_epoch = nonnegative_u64(
        product.active_release_epoch,
        "MiniApp active release epoch",
    )?;
    Ok(MiniAppSummaryDto {
        miniapp_id: product.miniapp_id.clone(),
        product_revision,
        display_name: product.display_name.clone(),
        description: product.description.clone(),
        icon_asset_id: product.icon_asset_id.clone(),
        kind,
        lifecycle,
        releases: MiniAppReleasePointersDto {
            pointer_revision,
            active_release_epoch,
            ready: None,
            active: None,
            previous: None,
        },
        service_health: if kind == MiniAppKindDto::UiOnly {
            MiniAppServiceHealthDto::NotApplicable
        } else {
            MiniAppServiceHealthDto::Stopped
        },
        surface_available: lifecycle == MiniAppLifecycleDto::Enabled
            && product.active_release_id.is_some(),
        updated_at_ms: product.updated_at,
    })
}

fn summary_from_snapshot(
    snapshot: &MiniAppM1Snapshot,
) -> Result<MiniAppSummaryDto, MiniAppM1ApplicationError> {
    let mut summary = summary_from_product(&snapshot.product)?;
    summary.releases.ready = snapshot
        .ready_release
        .as_ref()
        .map(release_ref_from_row);
    summary.releases.active = snapshot
        .active_release
        .as_ref()
        .map(release_ref_from_row);
    summary.releases.previous = snapshot
        .previous_release
        .as_ref()
        .map(release_ref_from_row);
    Ok(summary)
}

fn workshop_from_snapshot(
    snapshot: &MiniAppM1Snapshot,
    active_operation: Option<DurableOperationSummaryDto>,
) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
    let product = &snapshot.product;
    let project = &snapshot.project;
    let summary = summary_from_snapshot(snapshot)?;
    let source_state = match project.source_state.as_str() {
        "empty" => MiniAppProjectSourceStateDto::Empty,
        "editable" => MiniAppProjectSourceStateDto::Editable,
        "runtime_only" => MiniAppProjectSourceStateDto::RuntimeOnly,
        value => {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "unknown MiniApp source_state {value}"
            )));
        }
    };
    let ready = snapshot
        .ready_release
        .as_ref()
        .map(|release| -> Result<_, MiniAppM1ApplicationError> {
            let release_ref = release_ref_from_row(release);
            Ok(MiniAppReadyReleaseDto {
                release: release_ref.clone(),
                project_build_generation: match release.build_generation {
                    Some(value) => {
                        positive_u64(value, "MiniApp Ready Release build generation")?
                    }
                    None => 0,
                },
                created_at_ms: release.created_at,
                kind: summary.kind,
                service: None,
                test: MiniAppReleaseTestDto {
                    status: MiniAppTestStatusDto::NotRequired,
                    release_id: release.release_id.clone(),
                    expected_release_digest: release.release_digest.clone(),
                    receipt_id: None,
                    expected_service_run_key: None,
                    issued_at_ms: None,
                    error_code: None,
                },
                migration_count: 0,
                can_publish: true,
                can_auto_publish: snapshot
                    .auto_publish_authorization
                    .as_ref()
                    .is_some_and(|authorization| authorization.enabled)
                    && snapshot.active_release.is_some(),
                blocking_reasons: Vec::new(),
            })
        })
        .transpose()?;
    let schema: Value = serde_json::from_str(&product.config_schema_json)
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
    let values: Value = serde_json::from_str(&product.config_json)
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
    let config_schema_digest =
        nomifun_agent_contracts::digest_payload(&schema).map_err(|error| {
            MiniAppM1ApplicationError::Invalid(error.to_string())
        })?;
    Ok(MiniAppWorkshopDto {
        miniapp: summary,
        publish_mode: if snapshot
            .auto_publish_authorization
            .as_ref()
            .is_some_and(|authorization| authorization.enabled)
        {
            MiniAppPublishModeDto::AutoUiOnly
        } else {
            MiniAppPublishModeDto::Manual
        },
        project_id: project.project_id.clone(),
        project_revision: positive_u64(
            project.project_revision,
            "MiniApp Project revision",
        )?,
        source_state,
        build_generation: nonnegative_u64(
            project.build_generation,
            "MiniApp build generation",
        )?,
        source_snapshot_digest: project.source_head_digest.clone(),
        dependency_lock_digest: project.dependency_lock_digest.clone(),
        ready,
        config_schema: PluginConfigSchemaDto {
            schema_digest: config_schema_digest.as_ref().to_owned(),
            schema,
        },
        config: PluginConfigStateDto {
            config_revision: positive_u64(
                product.config_revision,
                "MiniApp config revision",
            )?,
            schema_digest: config_schema_digest.as_ref().to_owned(),
            values,
            valid: true,
            validation_errors: Vec::new(),
        },
        credential_bindings_revision: positive_u64(
            product.credential_bindings_revision,
            "MiniApp credential bindings revision",
        )?,
        credential_slots: snapshot
            .credential_bindings
            .iter()
            .map(|binding| CredentialSlotBindingDto {
                slot_key: binding.slot_key.clone(),
                display_name: binding.slot_key.clone(),
                required: true,
                status: CredentialBindingStatusDto::Bound,
                credential_id: Some(binding.credential_id.clone()),
            })
            .collect(),
        capabilities: Vec::new(),
        active_operation,
    })
}

fn latest_running_build(
    operations: Vec<ProductOperationRow>,
) -> Result<Option<DurableOperationSummaryDto>, MiniAppM1ApplicationError> {
    let mut running = operations.into_iter().filter(|operation| {
        operation.kind == "build"
            && operation.owner_kind == "miniapp"
            && operation.state == ProductOperationState::Running.as_str()
    });
    let Some(operation) = running.next() else {
        return Ok(None);
    };
    if running.next().is_some() {
        return Err(MiniAppM1ApplicationError::Invalid(
            "MiniApp has more than one running Build operation".to_owned(),
        ));
    }
    operation_summary(&operation).map(Some)
}

fn operation_summary(
    operation: &ProductOperationRow,
) -> Result<DurableOperationSummaryDto, MiniAppM1ApplicationError> {
    if operation.kind != "build" || operation.owner_kind != "miniapp" {
        return Err(MiniAppM1ApplicationError::Invalid(
            "MiniApp operation is not an owner-scoped Build".to_owned(),
        ));
    }
    let state = match operation.state.as_str() {
        "running" => DurableOperationStateDto::Running,
        "succeeded" => DurableOperationStateDto::Succeeded,
        "failed" => DurableOperationStateDto::Failed,
        "canceled" => DurableOperationStateDto::Canceled,
        value => {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "unknown MiniApp Build operation state {value}"
            )));
        }
    };
    let progress_percent = operation
        .progress_percent
        .map(|value| {
            u8::try_from(value).map_err(|_| {
                MiniAppM1ApplicationError::Invalid(
                    "MiniApp Build operation progress is outside u8 range".to_owned(),
                )
            })
        })
        .transpose()?;
    Ok(DurableOperationSummaryDto {
        operation_id: operation.operation_id.clone(),
        operation_revision: operation_revision(operation),
        kind: DurableOperationKindDto::Build,
        owner: DurableOperationOwnerDto::Miniapp {
            miniapp_id: operation.owner_id.clone(),
        },
        state,
        cancelable: operation.state == ProductOperationState::Running.as_str(),
        progress_percent,
        started_at_ms: operation.started_at_ms,
        completed_at_ms: operation.finished_at_ms,
    })
}

fn operation_revision(operation: &ProductOperationRow) -> u64 {
    if operation.finished_at_ms.is_some() {
        2
    } else {
        1
    }
}

fn operation_conflict(
    operation_id: &str,
    observed_state: &str,
    reason: &str,
) -> MiniAppM1ApplicationError {
    MiniAppM1ApplicationError::Database(nomifun_db::DbError::Conflict(format!(
        "MiniApp Build operation {operation_id} conflict: {reason}; observed state {observed_state}"
    )))
}

fn nonnegative_u64(
    value: i64,
    label: &str,
) -> Result<u64, MiniAppM1ApplicationError> {
    u64::try_from(value).map_err(|_| {
        MiniAppM1ApplicationError::Invalid(format!("{label} is negative"))
    })
}

fn positive_u64(
    value: i64,
    label: &str,
) -> Result<u64, MiniAppM1ApplicationError> {
    let value = nonnegative_u64(value, label)?;
    if value == 0 {
        Err(MiniAppM1ApplicationError::Invalid(format!(
            "{label} must be positive"
        )))
    } else {
        Ok(value)
    }
}

fn to_i64(value: u64, label: &str) -> Result<i64, MiniAppM1ApplicationError> {
    i64::try_from(value).map_err(|_| {
        MiniAppM1ApplicationError::Invalid(format!("{label} exceeds SQLite range"))
    })
}

fn validate_digest_string(
    value: &str,
    label: &str,
) -> Result<(), MiniAppM1ApplicationError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(MiniAppM1ApplicationError::Invalid(format!(
            "{label} must be a lowercase SHA-256 digest"
        )))
    }
}

fn kind_dto(value: &str) -> Result<MiniAppKindDto, MiniAppM1ApplicationError> {
    match value {
        "ui_only" => Ok(MiniAppKindDto::UiOnly),
        "service" => Ok(MiniAppKindDto::Service),
        value => Err(MiniAppM1ApplicationError::Invalid(format!(
            "unknown MiniApp kind {value}"
        ))),
    }
}

fn lifecycle_dto(
    value: &str,
) -> Result<MiniAppLifecycleDto, MiniAppM1ApplicationError> {
    match value {
        "enabled" => Ok(MiniAppLifecycleDto::Enabled),
        "disabled" => Ok(MiniAppLifecycleDto::Disabled),
        "trashed" => Ok(MiniAppLifecycleDto::Trashed),
        "deleting" => Ok(MiniAppLifecycleDto::Deleting),
        value => Err(MiniAppM1ApplicationError::Invalid(format!(
            "unknown MiniApp lifecycle {value}"
        ))),
    }
}

fn release_ref_from_row(
    release: &nomifun_db::MiniAppReleaseRow,
) -> MiniAppReleaseRefDto {
    MiniAppReleaseRefDto {
        release_id: release.release_id.clone(),
        artifact_id: release.artifact_id.clone(),
        release_digest: release.release_digest.clone(),
        manifest_digest: release.manifest_digest.clone(),
    }
}

fn canonical_json_string<T: Serialize>(
    value: &T,
) -> Result<String, MiniAppM1ApplicationError> {
    let bytes = canonical_json_bytes(value)
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
    String::from_utf8(bytes)
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))
}

fn positive_now_ms() -> i64 {
    nomifun_common::now_ms().max(1)
}

fn store_error(
    label: &str,
    error: impl std::fmt::Display,
) -> MiniAppM1ApplicationError {
    MiniAppM1ApplicationError::Invalid(format!("{label}: {error}"))
}

fn build_error_code(error: &MiniAppM1ApplicationError) -> &'static str {
    match error {
        MiniAppM1ApplicationError::NotFound => "MINIAPP_NOT_FOUND",
        MiniAppM1ApplicationError::Database(_) => "MINIAPP_DATABASE_ERROR",
        MiniAppM1ApplicationError::Invalid(message) if message.contains("Source Store") => {
            "MINIAPP_SOURCE_REJECTED"
        }
        MiniAppM1ApplicationError::Invalid(message) if message.contains("Release Store") => {
            "MINIAPP_RELEASE_REJECTED"
        }
        MiniAppM1ApplicationError::Invalid(_) => "MINIAPP_BUILD_REJECTED",
    }
}

fn bounded_log_line(value: &str) -> String {
    value.chars().take(4_096).collect()
}
