use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use nomifun_agent_contracts::{
    canonical_json_bytes, digest_payload, ArtifactId, JavaScriptBuildProfile, LocalizedMetadata,
    MiniAppId, MiniAppProjectId, MiniAppReadyOrigin, MiniAppReadyRelease, MiniAppReleaseId,
    MiniAppReleaseRef, MiniAppResourceContract, MiniAppSourceLineage, OperationId, PackageId,
    PackageRef, StrictJsonValue, VersionString, MINIAPP_BRIDGE_CONTRACT_VERSION,
    MINIAPP_RELEASE_PROFILE_VERSION,
};
use nomifun_api_types::{
    BuildMiniAppRequest, CredentialBindingStatusDto, CredentialSlotBindingDto,
    CreateMiniAppProjectRequest, DurableOperationKindDto, DurableOperationOwnerDto,
    DurableOperationStateDto, DurableOperationSummaryDto, MiniAppKindDto,
    MiniAppLibraryResponseDto, MiniAppLifecycleDto, MiniAppProjectSourceStateDto,
    MiniAppReadyReleaseDto, MiniAppReleasePointersDto, MiniAppReleaseRefDto,
    MiniAppReleaseTestDto, MiniAppServiceHealthDto, MiniAppSummaryDto, MiniAppTestStatusDto,
    MiniAppWorkshopDto, PluginConfigSchemaDto, PluginConfigStateDto,
};
use nomifun_db::{
    CancelMiniAppM1BuildOperationParams, CreateMiniAppM1Params, CreateMiniAppM1WithSourceParams,
    FinishMiniAppM1BuildAndRecordReadyParams, FinishMiniAppM1BuildOperationParams,
    IMiniAppM1Repository, MiniAppM1Kind, MiniAppM1ManagedSourceLineage, MiniAppM1Snapshot,
    MiniAppProductRow, MiniAppReleaseArtifactRow, MiniAppReleaseRow, ProductOperationRow,
    ProductOperationState, StartMiniAppM1BuildOperationParams,
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    MiniAppDependencyLockV1, MiniAppReleaseFileBytes, MiniAppReleasePublishRequest,
    MiniAppReleaseStore, MiniAppSourceSnapshot, MiniAppSourceStore, MiniAppStaticBundleBuilder,
    MiniAppStaticBundleFile, MiniAppStaticBundleInput,
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
            Ok(snapshot) => workshop_from_snapshot(&snapshot, None),
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
            let bytes = source
                .file(&file.normalized_relative_path)
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(format!(
                        "captured Source file disappeared: {}",
                        file.normalized_relative_path
                    ))
                })?;
            Ok(MiniAppReleaseFileBytes::new(
                file.normalized_relative_path.clone(),
                bytes.to_vec(),
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
    if snapshot.ready_release.as_ref().is_some_and(|ready| {
        ready.project_id.as_deref() == Some(request.project_id.as_str())
            && ready.source_snapshot_digest.as_deref()
                == Some(request.expected_source_snapshot_digest.as_str())
            && ready.dependency_lock_digest.as_deref()
                == Some(request.expected_dependency_lock_digest.as_str())
            && ready.build_generation
                == Some(i64::try_from(request.expected_build_generation).unwrap_or(i64::MIN))
    }) {
        return Err(MiniAppM1ApplicationError::Invalid(
            "the current Ready Release already represents this exact Source generation".to_owned(),
        ));
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
                can_publish: false,
                can_auto_publish: false,
                blocking_reasons: vec!["MINIAPP_PUBLISH_NOT_AVAILABLE".to_owned()],
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
