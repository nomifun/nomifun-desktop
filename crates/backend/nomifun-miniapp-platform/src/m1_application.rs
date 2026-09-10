use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    canonical_json_bytes, digest_bytes, digest_payload, ArtifactId, DigestHex,
    CapabilityCatalogMaterialization, CapabilityCatalogMaterializer,
    CapabilityCatalogPublication, CapabilityOwner, CapabilityProvenance,
    CapabilityConsumer, CapabilityReleaseState, CatalogAvailability,
    ContributionSourceKind, MiniAppCapabilityCatalogPublication,
    MiniAppCapabilityCatalogPublicationUpdate,
    MiniAppCapabilityCatalogSink,
    JavaScriptBuildProfile, LocalizedMetadata, MiniAppBridgeKvRequest,
    MiniAppBridgeCallId, MiniAppBridgeRequest, MiniAppBridgeSession,
    MiniAppBridgeSessionId,
    MiniAppBridgeTarget, MiniAppId, MiniAppKvResponse,
    MiniAppNonUiReleaseFingerprint, MiniAppProjectId,
    MiniAppPublishAuthorization,
    MiniAppPublishRequest as MiniAppPublishContract,
    MiniAppPointerExpectation, MiniAppReadyOrigin, MiniAppReadyRelease,
    MiniAppReadyReleaseRef, MiniAppReleaseId, MiniAppReleasePointerState,
    MiniAppReleaseRef, MiniAppResourceContract, MiniAppSourceLineage, OperationId,
    MiniAppServiceLifecycle, MiniAppServiceStorageDescriptor, MiniAppSourceBundle,
    MiniAppDatabaseHandleId,
    MiniAppServiceTestOutcome, MiniAppServiceTestReceipt, MiniAppServiceTestReceiptId,
    CredentialSlotDeclaration, PackageContributions, PackageId, PackageRef, StrictJsonValue,
    MiniAppSurfaceSessionId, MiniAppUiOnlyAutoPublishAuthorization,
    MiniAppUiOnlyAutoPublishProof, MiniAppUserAuthorizationId, VersionString,
    MiniAppBridgeTransport, MINIAPP_BRIDGE_CONTRACT_VERSION,
    MINIAPP_RELEASE_PROFILE_VERSION, MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
};
use nomifun_api_types::{
    BuildMiniAppRequest, CapabilityCatalogItemDto, CatalogMaterializationStateDto,
    CredentialBindingStatusDto, CredentialSlotBindingDto,
    CreateMiniAppProjectRequest, DurableOperationKindDto, DurableOperationOwnerDto,
    DurableOperationStateDto, DurableOperationSummaryDto, MiniAppKindDto,
    MiniAppLibraryResponseDto, MiniAppLifecycleDto, MiniAppProjectSourceStateDto,
    MiniAppPublishModeDto, MiniAppReadyReleaseDto, MiniAppReleasePointersDto,
    MiniAppReleaseRefDto, MiniAppReleaseTestDto, MiniAppServiceDescriptorDto,
    MiniAppServiceHealthDto, MiniAppServiceLifecycleDto, MiniAppSummaryDto,
    MiniAppSourceFileDto, MiniAppSurfaceLaunchDescriptorDto, MiniAppTestStatusDto,
    DeleteMiniAppRequest, ImportMiniAppArtifactRequest, ImportMiniAppShareRequest,
    MiniAppShareContentDto, MiniAppWorkshopDto, PluginConfigSchemaDto, PluginConfigStateDto,
    ExportMiniAppBackupRequest, ImportMiniAppBackupRequest,
    PublishMiniAppRequest as PublishMiniAppRequestDto, RestoreMiniAppRequest,
    RetryMiniAppDeleteRequest, RetryMiniAppServiceRequest,
    RollbackMiniAppRequest as RollbackMiniAppRequestDto,
    SetMiniAppEnabledRequest, SetMiniAppPublishModeRequest,
    SetMiniAppServiceRunningRequest, ShareMiniAppRequest, TestMiniAppReleaseRequest,
    TrashMiniAppRequest, ReplaceMiniAppSourceFileRequest,
};
use nomifun_db::{
    AbortMiniAppSourceMutationParams, BeginMiniAppM1ImportAsNewParams,
    BeginMiniAppSourceMutationParams, CancelMiniAppM1BuildOperationParams,
    CloseMiniAppM1SurfaceSessionParams,
    CreateMiniAppM1Params, CreateMiniAppM1WithSourceParams,
    ExecuteMiniAppM1SurfaceKvParams,
    FailMiniAppM1ExportOperationParams, FailMiniAppM1ImportParams,
    FinalizeMiniAppSourceMutationParams, FinishMiniAppM1BuildAndRecordReadyParams,
    FinishMiniAppM1BuildOperationParams,
    FinishMiniAppM1ExportOperationParams, FinishMiniAppM1ImportReadyParams,
    FinishMiniAppM1BackupImportParams, MiniAppM1BackupExportSnapshot,
    MiniAppM1BackupImportRelease, MiniAppM1BackupReleaseSlot, MiniAppKvRow,
    IMiniAppM1Repository, MiniAppM1AutoPublishGuard, MiniAppM1ImportSource, MiniAppM1Kind,
    MiniAppM1ManagedSourceLineage, MiniAppM1Snapshot, MiniAppM1SurfaceKvOperation,
    MiniAppM1SurfaceKvResult, MiniAppProductRow,
    MiniAppReleaseArtifactRow, MiniAppReleaseRow, MiniAppSurfaceSessionRow,
    MiniAppServiceTestReceiptRow, MiniAppSourceMutationIntentRow,
    OpenMiniAppM1SurfaceSessionParams,
    ProductOperationRow, ProductOperationState, PublishMiniAppM1ReadyParams,
    RecordMiniAppM1ServiceTestReceiptParams,
    ResolveMiniAppM1SurfaceSessionParams, RollbackMiniAppM1PreviousParams,
    SetMiniAppM1AutoPublishParams,
    BeginMiniAppM1DeleteParams, CommitMiniAppM1LifecycleParams,
    FailMiniAppM1DeleteParams, FinalizeMiniAppM1DeleteParams,
    RestartMiniAppM1DeleteParams, RestoreMiniAppM1Params,
    StartMiniAppM1BuildOperationParams, StartMiniAppM1ExportOperationParams,
    StartMiniAppM1BackupExportParams,
    TrashMiniAppM1Params,
};
use nomifun_js_runtime::ResolvedNodeRuntime;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    MiniAppDependencyLockV1, MiniAppImportedRelease, MiniAppImportedSource,
    MiniAppReleaseFileBytes,
    MiniAppReleasePublishRequest,
    issue_surface_capability, surface_capability_digest, MiniAppReleaseArtifactIdentity,
    MiniAppReleaseStore, MiniAppShareBundleExport, MiniAppShareBundleFilesystem,
    MiniAppShareSourceExport, MiniAppSourceExactImportRequest, MiniAppSourceFile,
    MiniAppSourceFileInput, MiniAppSourceScope, MiniAppSourceSnapshot, MiniAppSourceStore,
    MiniAppStaticBundleBuilder, MiniAppStaticBundleFile, MiniAppStaticBundleInput,
    MiniAppStaticServiceInput, MiniAppStoredRelease, MiniAppServiceRuntimeBinding,
    MiniAppServiceSpecInput, MiniAppServiceTestRunInput, NoopMiniAppServiceRuntime,
    MiniAppCallCancellation,
    MiniAppServiceObservation,
    MiniAppBackupFile, MiniAppBackupRelease, MiniAppBackupSource,
    MiniAppBackupStorage, MiniAppWholeAppBackupExport,
    MiniAppWholeAppBackupFilesystem,
    rebind_migration_ledger_for_target_with_releases,
    materialize_surface_entrypoint, MiniAppSourceContentKind,
};

#[derive(Debug, Error)]
pub enum MiniAppM1ApplicationError {
    #[error("MiniApp input is invalid: {0}")]
    Invalid(String),
    #[error("MiniApp runtime failed: {0}")]
    Runtime(String),
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

#[derive(Clone, Debug, PartialEq)]
pub struct MiniAppAgentCapabilityInvocation {
    pub owner_user_id: String,
    pub miniapp_id: MiniAppId,
    pub capability: nomifun_agent_contracts::CapabilityRef,
    pub action_id: nomifun_agent_contracts::ActionId,
    pub action_allowlist: BTreeSet<nomifun_agent_contracts::ActionId>,
    pub active_release: MiniAppReleaseRef,
    pub active_release_epoch: u64,
    pub catalog_digest: DigestHex,
    pub operation_id: OperationId,
    pub call_id: MiniAppBridgeCallId,
    pub payload: StrictJsonValue,
}

#[async_trait]
pub trait MiniAppAgentCapabilityPort: Send + Sync {
    async fn invoke_agent_capability(
        &self,
        request: MiniAppAgentCapabilityInvocation,
    ) -> Result<StrictJsonValue, MiniAppM1ApplicationError>;
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupProductMetadata {
    miniapp_id: String,
    product_revision: i64,
    display_name: String,
    description: Option<String>,
    icon_asset_id: Option<String>,
    kind: String,
    lifecycle: String,
    pointer_revision: i64,
    active_release_epoch: i64,
    materialized_catalog_digest: String,
    ready_release_id: Option<String>,
    ready_release_digest: Option<String>,
    active_release_id: Option<String>,
    active_release_digest: Option<String>,
    previous_release_id: Option<String>,
    previous_release_digest: Option<String>,
    release_lineage: BTreeMap<String, BackupReleaseMetadata>,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupReleaseMetadata {
    release_id: String,
    artifact_id: String,
    artifact_digest: String,
    manifest_digest: String,
    source_kind: String,
    project_id: Option<String>,
    source_snapshot_digest: Option<String>,
    dependency_lock_digest: Option<String>,
    build_profile_version: Option<String>,
    build_generation: Option<i64>,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupProjectMetadata {
    miniapp_id: String,
    project_id: String,
    project_revision: i64,
    source_state: String,
    build_generation: i64,
    source_snapshot_digest: Option<String>,
    dependency_lock_digest: Option<String>,
    build_profile_version: Option<String>,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupConfigMetadata {
    config_revision: i64,
    schema_digest: String,
    schema: Value,
    values: Value,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupKvMetadata {
    namespace: String,
    key: String,
    value_json: String,
    revision: i64,
    key_generation: i64,
    is_tombstone: bool,
    created_at: i64,
    updated_at: i64,
}

struct PreparedApplicationBackup {
    product: Value,
    project: Value,
    config: Value,
    credential_slots: Value,
    releases: BTreeMap<String, MiniAppBackupRelease>,
    source: Option<MiniAppBackupSource>,
    storage: MiniAppBackupStorage,
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
    source_mutation_lock: Arc<Mutex<()>>,
    service_runtime: Arc<RwLock<Arc<dyn MiniAppServiceRuntimeBinding>>>,
    catalog_sink: Arc<RwLock<Option<Arc<dyn MiniAppCapabilityCatalogSink>>>>,
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
            source_mutation_lock: Arc::new(Mutex::new(())),
            service_runtime: Arc::new(RwLock::new(Arc::new(NoopMiniAppServiceRuntime))),
            catalog_sink: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn install_service_runtime(
        &self,
        runtime: Arc<dyn MiniAppServiceRuntimeBinding>,
    ) {
        *self.service_runtime.write().await = runtime;
    }

    pub async fn install_catalog_sink(
        &self,
        sink: Arc<dyn MiniAppCapabilityCatalogSink>,
    ) {
        *self.catalog_sink.write().await = Some(sink);
    }

    async fn catalog_sink(&self) -> Option<Arc<dyn MiniAppCapabilityCatalogSink>> {
        self.catalog_sink.read().await.clone()
    }

    /// Rebuild the shared Catalog read-side from the durable M1 Active
    /// Release inventory. This is called once during startup and is also used
    /// after every pointer/lifecycle mutation.
    pub async fn hydrate_catalog_publications(
        &self,
        owner_user_id: &str,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let library = self.repository.library(owner_user_id).await?;
        for product in library.products {
            if let Some(snapshot) = self
                .repository
                .get(owner_user_id, &product.miniapp_id)
                .await?
            {
                self.sync_catalog_publication(owner_user_id, &snapshot)
                    .await?;
            }
        }
        Ok(())
    }

    async fn sync_catalog_publication(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let Some(sink) = self.catalog_sink().await else {
            return Ok(());
        };
        let miniapp_id = MiniAppId::from(snapshot.product.miniapp_id.clone());
        let publication = self.catalog_publication_for_snapshot(owner_user_id, snapshot)?;
        let update = MiniAppCapabilityCatalogPublicationUpdate {
            owner_user_id: owner_user_id.to_owned().into(),
            miniapp_id,
            product_revision: positive_u64(
                snapshot.product.product_revision,
                "MiniApp product revision",
            )?,
            pointer_revision: positive_u64(
                snapshot.product.pointer_revision,
                "MiniApp pointer revision",
            )?,
            active_release_epoch: nonnegative_u64(
                snapshot.product.active_release_epoch,
                "MiniApp active release epoch",
            )?,
            publication,
        };
        sink.replace_miniapp_publication(update)
            .map_err(MiniAppM1ApplicationError::Invalid)
    }

    fn catalog_publication_for_snapshot(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
    ) -> Result<Option<MiniAppCapabilityCatalogPublication>, MiniAppM1ApplicationError> {
        if snapshot.product.lifecycle != "enabled" {
            return Ok(None);
        }
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "enabled MiniApp has no Active Release for Catalog publication".into(),
                )
            })?;
        let stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            active,
        )?;
        let mut publication = build_miniapp_catalog_publication(
            MiniAppId::from(snapshot.product.miniapp_id.clone()),
            release_contract_ref(active),
            &stored.artifact.manifest.payload.contributions,
        )?;
        publication.active_release_epoch = positive_u64(
            snapshot.product.active_release_epoch,
            "MiniApp active release epoch",
        )?;
        if publication.catalog_digest.as_ref()
            != snapshot.product.materialized_catalog_digest.as_str()
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Product Catalog digest does not match the materialized publication"
                    .into(),
            ));
        }
        publication
            .validate()
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        Ok(Some(publication))
    }

    async fn service_runtime(&self) -> Arc<dyn MiniAppServiceRuntimeBinding> {
        self.service_runtime.read().await.clone()
    }

    /// Resolve one exact input/output schema owned by a frozen MiniApp
    /// capability projection. The lookup is intentionally application-owned:
    /// MiniApp releases are not Kernel Plugin artifacts and their schema bytes
    /// must remain behind the owner-scoped Release Store boundary.
    pub async fn resolve_agent_capability_schema(
        &self,
        owner_user_id: &str,
        capability: &nomifun_agent_contracts::ResolvedMiniAppCapability,
        reference: &nomifun_agent_contracts::CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, MiniAppM1ApplicationError> {
        validate_request_identity(owner_user_id, "owner_user_id")?;
        capability
            .validate()
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.message))?;
        let snapshot = self
            .repository
            .get(owner_user_id, capability.miniapp_id.as_ref())
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if snapshot.product.lifecycle != "enabled"
            || snapshot.product.active_release_epoch
                != i64::try_from(capability.active_release_epoch).map_err(|_| {
                    MiniAppM1ApplicationError::Invalid(
                        "Active Release epoch exceeds SQLite range".into(),
                    )
                })?
            || snapshot.active_release.as_ref().is_none_or(|release| {
                release_contract_ref(release) != capability.active_release
            })
            || snapshot.product.materialized_catalog_digest != capability.catalog_digest.as_ref()
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Agent schema request is stale against the Active Release".into(),
            ));
        }
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Active Release".into()))?;
        let stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            active,
        )?;
        let publication = build_miniapp_catalog_publication(
            capability.miniapp_id.clone(),
            capability.active_release.clone(),
            &stored.artifact.manifest.payload.contributions,
        )?;
        if publication.catalog_digest != capability.catalog_digest {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Agent schema request has a stale Catalog publication".into(),
            ));
        }
        let published = publication
            .capabilities
            .iter()
            .find(|item| item.entry.capability == capability.capability)
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "MiniApp Agent schema capability is not in the Active Release".into(),
                )
            })?;
        if published.manifest.package != capability.source_package
            || published.manifest.contribution_id != capability.contribution_id
            || published.manifest.contributions.resource_kinds
                != capability.required_resource_kinds
            || published.manifest.contributions.actions != capability.actions
            || published
                .entry
                .operation_lock(CapabilityConsumer::Agent)
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?
                .contribution
                != capability.contribution_lock
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Agent schema capability provenance drifted".into(),
            ));
        }
        if !capability
            .actions
            .iter()
            .any(|action| {
                (&action.input_schema == reference || &action.output_schema == reference)
                    && (capability.action_allowlist.is_empty()
                        || capability.action_allowlist.contains(&action.action_id))
            })
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Agent schema ref is not declared by the frozen action allowlist".into(),
            ));
        }
        stored
            .artifact
            .manifest
            .payload
            .schemas
            .get(reference)
            .cloned()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(format!(
                    "MiniApp Release is missing canonical schema {}",
                    reference.as_ref()
                ))
            })
    }

    async fn invoke_agent_capability_inner(
        &self,
        request: MiniAppAgentCapabilityInvocation,
    ) -> Result<StrictJsonValue, MiniAppM1ApplicationError> {
        validate_request_identity(request.owner_user_id.as_str(), "owner_user_id")?;
        validate_request_identity(request.miniapp_id.as_ref(), "miniapp_id")?;
        request
            .active_release
            .validate()
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        validate_digest_string(request.catalog_digest.as_ref(), "catalog digest")?;
        if request.active_release_epoch == 0 {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Agent capability invocation requires a positive Active Release epoch".into(),
            ));
        }

        let snapshot = self
            .repository
            .get(&request.owner_user_id, request.miniapp_id.as_ref())
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if snapshot.product.lifecycle != "enabled"
            || snapshot.active_release.as_ref().is_none_or(|active| {
                release_contract_ref(active) != request.active_release
            })
            || snapshot.product.active_release_epoch
                != i64::try_from(request.active_release_epoch).map_err(|_| {
                    MiniAppM1ApplicationError::Invalid(
                        "Active Release epoch exceeds SQLite range".into(),
                    )
                })?
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Agent capability invocation is stale against the Active Release"
                    .into(),
            ));
        }
        if snapshot.product.materialized_catalog_digest != request.catalog_digest.as_ref() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Agent capability invocation has a stale Catalog digest".into(),
            ));
        }
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Active Release".into()))?;
        let stored = self.load_verified_release(
            &request.owner_user_id,
            &snapshot.project.project_id,
            active,
        )?;
        let publication = build_miniapp_catalog_publication(
            request.miniapp_id.clone(),
            request.active_release.clone(),
            &stored.artifact.manifest.payload.contributions,
        )?;
        if publication.catalog_digest != request.catalog_digest {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Agent capability invocation publication digest is invalid".into(),
            ));
        }
        let capability = publication
            .capabilities
            .iter()
            .find(|capability| capability.entry.capability == request.capability)
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "MiniApp Agent capability is not in the Active Release".into(),
                )
            })?;
        if !capability
            .manifest
            .supports_consumer(CapabilityConsumer::Agent)
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp capability does not support the Agent consumer".into(),
            ));
        }
        if !capability
            .manifest
            .supports_consumer(CapabilityConsumer::MiniAppService)
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp capability does not support the MiniApp Service consumer".into(),
            ));
        }
        if !capability
            .manifest
            .contributions
            .resource_kinds
            .is_empty()
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "CAPABILITY_RESOURCE_BINDING_UNAVAILABLE".into(),
            ));
        }
        if !capability
            .manifest
            .contributions
            .actions
            .iter()
            .any(|action| action.action_id == request.action_id)
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "CAPABILITY_ACTION_NOT_DECLARED".into(),
            ));
        }
        if !request.action_allowlist.is_empty()
            && !request.action_allowlist.contains(&request.action_id)
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "CAPABILITY_ACTION_NOT_ALLOWED".into(),
            ));
        }
        let spec = self
            .resolve_service_spec(
                &snapshot,
                active,
                request.active_release_epoch,
                &request.owner_user_id,
            )
            .await?
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "MiniApp Agent capability requires an Active Service".into(),
                )
            })?;
        self.service_runtime()
            .await
            .invoke(
                &spec,
                request.call_id,
                request.action_id.as_ref().to_owned(),
                request.payload,
                MiniAppCallCancellation::default(),
                positive_now_ms(),
            )
            .await
            .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))
    }

    async fn resolve_service_spec(
        &self,
        snapshot: &MiniAppM1Snapshot,
        release: &MiniAppReleaseRow,
        active_release_epoch: u64,
        owner_user_id: &str,
    ) -> Result<Option<nomifun_agent_contracts::ResolvedMiniAppServiceSpec>, MiniAppM1ApplicationError>
    {
        self.resolve_service_spec_with_storage(
            snapshot,
            release,
            active_release_epoch,
            owner_user_id,
            None,
        )
        .await
    }

    async fn resolve_service_spec_with_storage(
        &self,
        snapshot: &MiniAppM1Snapshot,
        release: &MiniAppReleaseRow,
        active_release_epoch: u64,
        owner_user_id: &str,
        storage_override: Option<MiniAppServiceStorageDescriptor>,
    ) -> Result<Option<nomifun_agent_contracts::ResolvedMiniAppServiceSpec>, MiniAppM1ApplicationError>
    {
        let stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            release,
        )?;
        let Some(descriptor) = stored.artifact.manifest.payload.service.clone() else {
            return Ok(None);
        };
        let config: Value = serde_json::from_str(&snapshot.product.config_json)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let runtime = self.service_runtime().await;
        let storage = match storage_override {
            Some(storage) => storage,
            None => runtime
                .resolve_storage(
                    owner_user_id,
                    &MiniAppId::from(snapshot.product.miniapp_id.clone()),
                    descriptor.uses_files,
                    descriptor.uses_private_database,
                )
                .await
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?
                .descriptor,
        };
        runtime
            .register_module(
                MiniAppId::from(snapshot.product.miniapp_id.clone()),
                DigestHex::from(release.release_digest.clone()),
                stored
                    .artifact_root
                    .join("files")
                    .join("service")
                    .join("main.mjs"),
            )
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let spec = runtime
            .resolve_spec(MiniAppServiceSpecInput {
                miniapp_id: MiniAppId::from(snapshot.product.miniapp_id.clone()),
                release: release_contract_ref(release),
                active_release_epoch,
                descriptor: descriptor.clone(),
                config_schema_digest: stored.artifact.manifest.payload.config_schema_digest.clone(),
                config_snapshot_digest: digest_payload(&config)
                    .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?,
                credential_slots_digest: stored
                    .artifact
                    .manifest
                    .payload
                    .credential_slots_digest
                    .clone(),
                resource_contract_digest: stored
                    .artifact
                    .manifest
                    .payload
                    .resource_contract_digest
                    .clone(),
                resource_bindings_digest: digest_payload(&BTreeMap::<String, String>::new())
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?,
                bridge_contract_digest: stored
                    .artifact
                    .manifest
                    .payload
                    .bridge_contract_digest
                    .clone(),
                contribution_set_digest: stored
                    .artifact
                    .manifest
                    .payload
                    .contribution_set_digest()
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?,
                storage,
            })
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        Ok(Some(spec))
    }

    async fn reconcile_service_runtime(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let Some(active) = snapshot.active_release.as_ref() else {
            return Ok(());
        };
        if snapshot.product.kind != MiniAppM1Kind::Service.as_str() {
            return Ok(());
        }
        let runtime = self.service_runtime().await;
        if snapshot.product.lifecycle != "enabled" {
            runtime
                .stop(&MiniAppId::from(snapshot.product.miniapp_id.clone()))
                .await
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
            return Ok(());
        }
        let spec = self
            .resolve_service_spec(
                snapshot,
                active,
                u64::try_from(snapshot.product.active_release_epoch).map_err(|_| {
                    MiniAppM1ApplicationError::Invalid(
                        "MiniApp active release epoch is negative".into(),
                    )
                })?,
                owner_user_id,
            )
            .await?
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "Service Active Release has no Service descriptor".into(),
                )
            })?;
        runtime
            .bind_active(spec, true)
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))
    }

    async fn prepare_service_cutover(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
        target_release: &MiniAppReleaseRow,
        target_active_release_epoch: u64,
        apply_migrations: bool,
    ) -> Result<
        (
            Option<nomifun_agent_contracts::ResolvedMiniAppServiceSpec>,
            Option<nomifun_agent_contracts::ResolvedMiniAppServiceSpec>,
        ),
        MiniAppM1ApplicationError,
    > {
        if snapshot.product.kind != MiniAppM1Kind::Service.as_str() {
            return Ok((None, None));
        }
        let runtime = self.service_runtime().await;
        let miniapp_id = MiniAppId::from(snapshot.product.miniapp_id.clone());
        let mut current = if snapshot.product.lifecycle == "enabled" {
            let active = snapshot.active_release.as_ref().ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "enabled Service MiniApp has no Active Release".into(),
                )
            })?;
            Some(
                self.resolve_service_spec(
                    snapshot,
                    active,
                    u64::try_from(snapshot.product.active_release_epoch).map_err(|_| {
                        MiniAppM1ApplicationError::Invalid(
                            "MiniApp active release epoch is negative".into(),
                        )
                    })?,
                    owner_user_id,
                )
                .await?
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "Active Service Release has no Service descriptor".into(),
                    )
                })?,
            )
        } else {
            None
        };
        let target_stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            target_release,
        )?;
        let target_descriptor = target_stored
            .artifact
            .manifest
            .payload
            .service
            .clone()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "target Service Release has no Service descriptor".into(),
                )
            })?;
        let target_storage = runtime
            .resolve_storage(
                owner_user_id,
                &miniapp_id,
                target_descriptor.uses_files,
                target_descriptor.uses_private_database,
            )
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let mut target = if snapshot.product.lifecycle == "enabled" {
            Some(
                self.resolve_service_spec(
                    snapshot,
                    target_release,
                    target_active_release_epoch,
                    owner_user_id,
                )
                .await?
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "target Service Release has no Service descriptor".into(),
                    )
                })?,
            )
        } else {
            None
        };
        runtime
            .stop(&miniapp_id)
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let migrations = target_stored.artifact.manifest.payload.migrations.clone();
        if apply_migrations && !migrations.is_empty() {
            let database = target_storage
                .descriptor
                .private_database
                .as_ref()
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "Service migrations require the target Private Database".into(),
                    )
                })?;
            if let Err(error) = runtime
                .apply_storage_migrations(
                    owner_user_id,
                    &miniapp_id,
                    &target_storage.descriptor,
                    &database.migration_ledger_digest,
                    &release_contract_ref(target_release),
                    &migrations,
                    positive_now_ms(),
                )
                .await
            {
                if current.is_some() {
                    self.restore_current_service(
                        owner_user_id,
                        snapshot,
                        "Service migration failed",
                    )
                    .await?;
                }
                return Err(MiniAppM1ApplicationError::Invalid(format!(
                    "Service migration failed before Publish commit: {error}"
                )));
            }
            if current.is_some() {
                let active = snapshot.active_release.as_ref().ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "enabled Service has no Active Release while restoring its storage"
                            .into(),
                    )
                })?;
                current = match self
                    .resolve_service_spec(
                        snapshot,
                        active,
                        u64::try_from(snapshot.product.active_release_epoch).map_err(|_| {
                            MiniAppM1ApplicationError::Invalid(
                                "MiniApp active release epoch is negative".into(),
                            )
                        })?,
                        owner_user_id,
                    )
                    .await
                {
                    Ok(Some(spec)) => Some(spec),
                    Ok(None) => {
                        self.restore_current_service(
                            owner_user_id,
                            snapshot,
                            "current Service descriptor disappeared after migration",
                        )
                        .await?;
                        return Err(MiniAppM1ApplicationError::Invalid(
                            "current Service Release has no Service descriptor after migration"
                                .into(),
                        ));
                    }
                    Err(error) => {
                        if current.is_some() {
                            self.restore_current_service(
                                owner_user_id,
                                snapshot,
                                &format!("current Service resolution failed after migration ({error})"),
                            )
                            .await?;
                        }
                        return Err(error);
                    }
                };
            }
            if target.is_some() {
                target = match self
                    .resolve_service_spec(
                        snapshot,
                        target_release,
                        target_active_release_epoch,
                        owner_user_id,
                    )
                    .await
                {
                    Ok(Some(spec)) => Some(spec),
                    Ok(None) => {
                        if current.is_some() {
                            self.restore_current_service(
                                owner_user_id,
                                snapshot,
                                "target Service storage migrated but target descriptor disappeared",
                            )
                            .await?;
                        }
                        return Err(MiniAppM1ApplicationError::Invalid(
                            "target Service Release has no Service descriptor after migration"
                                .into(),
                        ));
                    }
                    Err(error) => {
                        if current.is_some() {
                            self.restore_current_service(
                                owner_user_id,
                                snapshot,
                                &format!("target Service resolution failed after migration ({error})"),
                            )
                            .await?;
                        }
                        return Err(error);
                    }
                };
            }
        }
        if let Some(target_spec) = target.as_ref() {
            if let Err(error) = runtime.start(target_spec.clone()).await {
                if current.is_some() {
                    self.restore_current_service(
                        owner_user_id,
                        snapshot,
                        &format!("target Service start failed ({error})"),
                    )
                    .await?;
                }
                return Err(MiniAppM1ApplicationError::Invalid(format!(
                    "target Service failed readiness before Release commit: {error}"
                )));
            }
        }
        Ok((current, target))
    }

    async fn restore_service_after_failed_cutover(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
        current: Option<nomifun_agent_contracts::ResolvedMiniAppServiceSpec>,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let Some(current) = current else {
            return Ok(());
        };
        let _ = current;
        self.restore_current_service(
            owner_user_id,
            snapshot,
            "release commit failed",
        )
        .await
    }

    async fn restore_current_service(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
        reason: &str,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let active = snapshot.active_release.as_ref().ok_or_else(|| {
            MiniAppM1ApplicationError::Invalid(
                "cannot restore a Service without its current Active Release".into(),
            )
        })?;
        let current = self
            .resolve_service_spec(
                snapshot,
                active,
                u64::try_from(snapshot.product.active_release_epoch).map_err(|_| {
                    MiniAppM1ApplicationError::Invalid(
                        "MiniApp active release epoch is negative".into(),
                    )
                })?,
                owner_user_id,
            )
            .await?
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "current Active Release has no Service descriptor".into(),
                )
            })?;
        self.service_runtime()
            .await
            .bind_active(current, true)
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(format!(
                "{reason}; restoring the previous Service failed for {}: {error}",
                snapshot.product.miniapp_id
            )))
    }

    async fn complete_service_cutover(
        &self,
        committed: &MiniAppM1Snapshot,
        target: Option<nomifun_agent_contracts::ResolvedMiniAppServiceSpec>,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let runtime = self.service_runtime().await;
        if committed.product.kind != MiniAppM1Kind::Service.as_str()
            || committed.product.lifecycle != "enabled"
        {
            runtime
                .stop(&MiniAppId::from(committed.product.miniapp_id.clone()))
                .await
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
            return Ok(());
        }
        let target = target.ok_or_else(|| {
            MiniAppM1ApplicationError::Invalid(
                "committed enabled Service has no resolved target spec".into(),
            )
        })?;
        runtime
            .bind_active(target, true)
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(format!(
                "Service Release committed but Host reconciliation failed: {error}"
            )))
    }

    async fn service_observation(
        &self,
        snapshot: &MiniAppM1Snapshot,
    ) -> Result<Option<MiniAppServiceObservation>, MiniAppM1ApplicationError> {
        if snapshot.product.kind != MiniAppM1Kind::Service.as_str() {
            return Ok(None);
        }
        let Some(active) = snapshot.active_release.as_ref() else {
            return Ok(Some(MiniAppServiceObservation::Stopped));
        };
        let release = release_contract_ref(active);
        let state = self
            .service_runtime()
            .await
            .state(&MiniAppId::from(snapshot.product.miniapp_id.clone()))
            .await;
        Ok(Some(match state {
            Some(crate::MiniAppServiceHostState::Starting { .. }) => {
                MiniAppServiceObservation::Starting { release }
            }
            Some(crate::MiniAppServiceHostState::Running { .. }) => {
                MiniAppServiceObservation::Ready {
                    release,
                    started_at_ms: snapshot.product.updated_at,
                }
            }
            Some(crate::MiniAppServiceHostState::Backoff { error, .. })
            | Some(crate::MiniAppServiceHostState::Error { error, .. }) => {
                MiniAppServiceObservation::Failed {
                    release,
                    error_code: bounded_error_code(&error),
                }
            }
            None | Some(crate::MiniAppServiceHostState::Stopped) => {
                MiniAppServiceObservation::Stopped
            }
        }))
    }

    async fn summary_projection(
        &self,
        snapshot: &MiniAppM1Snapshot,
    ) -> Result<MiniAppSummaryDto, MiniAppM1ApplicationError> {
        let observation = self.service_observation(snapshot).await?;
        Ok(summary_from_snapshot_with_observation(
            snapshot,
            observation.as_ref(),
        )?)
    }

    async fn workshop_projection(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
        active_operation: Option<DurableOperationSummaryDto>,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let active_operation = match active_operation {
            Some(operation) => Some(operation),
            None => {
                self.latest_miniapp_operation(
                    owner_user_id,
                    &snapshot.product.miniapp_id,
                )
                .await?
            }
        };
        let observation = self.service_observation(snapshot).await?;
        let mut workshop =
            workshop_from_snapshot_with_observation(snapshot, active_operation, observation.as_ref())?;
        if let Some(publication) =
            self.catalog_publication_for_snapshot(owner_user_id, snapshot)?
        {
            workshop.capabilities = publication
                .capabilities
                .iter()
                .map(miniapp_capability_catalog_item)
                .collect::<Result<Vec<_>, _>>()?;
        }
        if let Some(ready) = snapshot.ready_release.as_ref() {
            let stored = self.load_verified_release(
                owner_user_id,
                &snapshot.project.project_id,
                ready,
            )?;
            if let Some(service) = stored.artifact.manifest.payload.service.as_ref() {
                let ready_record: MiniAppReadyRelease =
                    serde_json::from_str(&ready.release_record_json).map_err(|error| {
                        MiniAppM1ApplicationError::Runtime(format!(
                            "stored Ready Release record is invalid: {error}"
                        ))
                    })?;
                ready_record
                    .validate_for_artifact(&stored.artifact)
                    .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))?;
                let receipt = self
                    .repository
                    .get_ready_service_test_receipt(
                        owner_user_id,
                        &snapshot.product.miniapp_id,
                    )
                    .await?;
                let test = self
                    .service_test_projection(
                        ready,
                        ready_record.matching_service_test_receipt.as_ref(),
                        receipt.as_ref(),
                    )
                    .await?;
                let blocking_reasons = match test.status {
                    MiniAppTestStatusDto::Passed => Vec::new(),
                    MiniAppTestStatusDto::Failed => vec![
                        test.error_code
                            .clone()
                            .unwrap_or_else(|| "service_test_failed".to_owned()),
                    ],
                    MiniAppTestStatusDto::NeedsTestInput => {
                        vec!["service_test_needs_input".to_owned()]
                    }
                    MiniAppTestStatusDto::Stale => {
                        vec!["service_test_stale".to_owned()]
                    }
                    MiniAppTestStatusDto::NotRun => {
                        vec!["service_test_not_run".to_owned()]
                    }
                    MiniAppTestStatusDto::NotRequired => Vec::new(),
                };
                let ready_projection = workshop.ready.as_mut().expect("ready row is present");
                ready_projection.service = Some(service_descriptor_dto(service));
                ready_projection.test = test;
                ready_projection.migration_count =
                    stored.artifact.manifest.payload.migrations.len() as u32;
                ready_projection.can_publish = true;
                ready_projection.blocking_reasons = blocking_reasons;
            }
        }
        let active_service_lifecycle = if let Some(active) = snapshot.active_release.as_ref() {
            let stored = self.load_verified_release(
                owner_user_id,
                &snapshot.project.project_id,
                active,
            )?;
            stored
                .artifact
                .manifest
                .payload
                .service
                .as_ref()
                .map(|service| match service.lifecycle {
                    MiniAppServiceLifecycle::OnDemand => MiniAppServiceLifecycleDto::OnDemand,
                    MiniAppServiceLifecycle::Continuous => MiniAppServiceLifecycleDto::Continuous,
                })
        } else {
            None
        };
        workshop.active_service = if let Some(active) = snapshot.active_release.as_ref() {
            let stored = self.load_verified_release(
                owner_user_id,
                &snapshot.project.project_id,
                active,
            )?;
            stored
                .artifact
                .manifest
                .payload
                .service
                .as_ref()
                .map(service_descriptor_dto)
        } else {
            None
        };
        workshop.service_lifecycle = active_service_lifecycle.or_else(|| {
            workshop
                .ready
                .as_ref()
                .and_then(|ready| ready.service.as_ref())
                .map(|service| service.lifecycle)
        });
        Ok(workshop)
    }

    async fn service_test_projection(
        &self,
        ready: &MiniAppReleaseRow,
        reference: Option<&nomifun_agent_contracts::MiniAppServiceTestReceiptRef>,
        row: Option<&MiniAppServiceTestReceiptRow>,
    ) -> Result<MiniAppReleaseTestDto, MiniAppM1ApplicationError> {
        let mut projection = MiniAppReleaseTestDto {
            status: MiniAppTestStatusDto::NotRun,
            release_id: ready.release_id.clone(),
            expected_release_digest: ready.release_digest.clone(),
            receipt_id: reference.map(|value| value.receipt_id.as_ref().to_owned()),
            expected_service_run_key: reference
                .map(|value| value.service_run_key.as_ref().to_owned()),
            issued_at_ms: None,
            error_code: None,
        };
        let Some(row) = row else {
            if reference.is_some() {
                projection.status = MiniAppTestStatusDto::Stale;
            }
            return Ok(projection);
        };
        let receipt: MiniAppServiceTestReceipt =
            serde_json::from_str(&row.receipt_json).map_err(|error| {
                MiniAppM1ApplicationError::Runtime(format!(
                    "stored Service Test receipt is invalid: {error}"
                ))
            })?;
        let current_runtime = self
            .service_runtime()
            .await
            .current_runtime_fingerprint()
            .await
            .map_err(|error| {
                MiniAppM1ApplicationError::Runtime(format!(
                    "cannot resolve current Runtime for Service Test receipt: {error}"
                ))
            })?;
        let runtime_matches = match current_runtime {
            Some(runtime) => digest_payload(&runtime)
                .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))?
                .as_ref()
                == row.runtime_fingerprint_digest,
            None => false,
        };
        projection.receipt_id = Some(row.receipt_id.clone());
        projection.expected_service_run_key = Some(row.service_run_key.clone());
        projection.issued_at_ms = Some(row.issued_at_ms);
        projection.error_code = row.error_code.clone();
        projection.status = if !runtime_matches {
            MiniAppTestStatusDto::Stale
        } else {
            match receipt.outcome {
                MiniAppServiceTestOutcome::Passed => MiniAppTestStatusDto::Passed,
                MiniAppServiceTestOutcome::Failed => MiniAppTestStatusDto::Failed,
                MiniAppServiceTestOutcome::NeedsTestInput => {
                    MiniAppTestStatusDto::NeedsTestInput
                }
            }
        };
        Ok(projection)
    }

    pub async fn library(
        &self,
        owner_user_id: &str,
    ) -> Result<MiniAppLibraryResponseDto, MiniAppM1ApplicationError> {
        self.reconcile_source_mutations().await?;
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
            miniapps.push(self.summary_projection(&snapshot).await?);
        }
        Ok(MiniAppLibraryResponseDto {
            library_revision: nonnegative_u64(
                library.library.revision,
                "MiniApp library revision",
            )?,
            miniapps,
        })
    }

    pub async fn source_file(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        path: &str,
    ) -> Result<MiniAppSourceFileDto, MiniAppM1ApplicationError> {
        self.reconcile_source_mutations().await?;
        let snapshot = self
            .repository
            .get(owner_user_id, miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if snapshot.project.source_state != "editable" {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Project Source is not editable".into(),
            ));
        }
        let source = self
            .stores
            .source
            .current_snapshot(
                owner_user_id,
                miniapp_id,
                &snapshot.project.project_id,
            )
            .map_err(|error| store_error("Source Store", error))?;
        require_source_matches_project(&source, &snapshot)?;
        let content = source.file(path).ok_or_else(|| {
            MiniAppM1ApplicationError::NotFound
        })?;
        let content = String::from_utf8(content.to_vec()).map_err(|_| {
            MiniAppM1ApplicationError::Invalid(
                "MiniApp Source editor supports UTF-8 text files only".into(),
            )
        })?;
        Ok(MiniAppSourceFileDto {
            miniapp_id: miniapp_id.to_owned(),
            project_id: snapshot.project.project_id,
            path: path.to_owned(),
            content,
            source_snapshot_digest: source.source_snapshot_digest.as_ref().to_owned(),
            build_generation: source.project.build_generation,
        })
    }

    pub async fn replace_source_file(
        &self,
        owner_user_id: &str,
        request: ReplaceMiniAppSourceFileRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let _mutation_guard = self.source_mutation_lock.lock().await;
        self.reconcile_source_mutations_locked().await?;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if snapshot.product.product_revision
            != to_i64(request.expected_product_revision, "product revision")?
            || snapshot.project.project_id != request.project_id
            || snapshot.project.project_revision
                != to_i64(request.expected_project_revision, "project revision")?
            || snapshot.project.build_generation
                != to_i64(request.expected_build_generation, "build generation")?
            || snapshot.project.source_head_digest.as_deref()
                != Some(request.expected_source_snapshot_digest.as_str())
            || snapshot.project.source_state != "editable"
            || matches!(snapshot.product.lifecycle.as_str(), "trashed" | "deleting")
        {
            return Err(MiniAppM1ApplicationError::Database(
                nomifun_db::DbError::Conflict(
                    "MiniApp Source edit is stale against the exact Product/Project head".into(),
                ),
            ));
        }
        require_no_running_build(
            &*self.repository,
            owner_user_id,
            &request.miniapp_id,
        )
        .await?;
        let prepared = self
            .stores
            .source
            .prepare_file_replace(
                owner_user_id,
                &request.miniapp_id,
                &request.project_id,
                &request.expected_source_snapshot_digest,
                &request.path,
                request.content.into_bytes(),
            )
            .map_err(|error| store_error("Source Store", error))?;
        if prepared.expected_build_generation != request.expected_build_generation {
            return Err(MiniAppM1ApplicationError::Database(
                nomifun_db::DbError::Conflict(
                    "MiniApp Source Store generation differs from the Project".into(),
                ),
            ));
        }
        if prepared.is_noop() {
            return self.workshop_projection(owner_user_id, &snapshot, None).await;
        }
        let intent_id = Uuid::now_v7().to_string();
        let created_at = positive_now_ms().max(snapshot.project.updated_at);
        self.repository
            .begin_source_mutation(&BeginMiniAppSourceMutationParams {
                intent_id: intent_id.clone(),
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                project_id: request.project_id.clone(),
                expected_product_revision: snapshot.product.product_revision,
                expected_project_revision: snapshot.project.project_revision,
                expected_build_generation: snapshot.project.build_generation,
                expected_source_digest: prepared
                    .expected_source_snapshot_digest
                    .as_ref()
                    .to_owned(),
                next_source_digest: prepared
                    .next_source_snapshot_digest
                    .as_ref()
                    .to_owned(),
                next_build_generation: to_i64(
                    prepared.next_build_generation,
                    "next build generation",
                )?,
                created_at,
            })
            .await?;
        if let Err(error) = self.stores.source.commit_prepared_source(&prepared) {
            self.repository
                .abort_source_mutation(&AbortMiniAppSourceMutationParams {
                    intent_id,
                    owner_user_id: owner_user_id.to_owned(),
                    miniapp_id: request.miniapp_id,
                    project_id: request.project_id,
                })
                .await?;
            return Err(store_error("Source Store", error));
        }
        let updated_at = positive_now_ms()
            .max(created_at.saturating_add(1))
            .max(snapshot.project.updated_at.saturating_add(1));
        let committed = match self
            .repository
            .finalize_source_mutation(&FinalizeMiniAppSourceMutationParams {
                intent_id: intent_id.clone(),
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                project_id: request.project_id.clone(),
                updated_at,
            })
            .await
        {
            Ok(committed) => committed,
            Err(error) => {
                if let Some(intent) = self
                    .repository
                    .get_source_mutation_intent(
                        owner_user_id,
                        &request.miniapp_id,
                        &request.project_id,
                    )
                    .await?
                {
                    self.reconcile_source_mutation(&intent)
                        .await
                        .map_err(|recovery| {
                            MiniAppM1ApplicationError::Runtime(format!(
                                "MiniApp Source was committed but database finalize failed ({error}); recovery failed ({recovery})"
                            ))
                        })?;
                }
                let recovered = self
                    .repository
                    .get(owner_user_id, &request.miniapp_id)
                    .await?
                    .ok_or(MiniAppM1ApplicationError::NotFound)?;
                if recovered.project.source_head_digest.as_deref()
                    != Some(prepared.next_source_snapshot_digest.as_ref())
                    || recovered.project.build_generation
                        != to_i64(prepared.next_build_generation, "next build generation")?
                {
                    return Err(MiniAppM1ApplicationError::Runtime(format!(
                        "MiniApp Source database finalize failed without a recoverable commit: {error}"
                    )));
                }
                recovered
            }
        };
        self.workshop_projection(owner_user_id, &committed, None)
            .await
    }

    pub async fn reconcile_source_mutations(
        &self,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let _mutation_guard = self.source_mutation_lock.lock().await;
        self.reconcile_source_mutations_locked().await
    }

    async fn reconcile_source_mutations_locked(
        &self,
    ) -> Result<(), MiniAppM1ApplicationError> {
        for intent in self.repository.list_source_mutation_intents().await? {
            self.reconcile_source_mutation(&intent).await?;
        }
        Ok(())
    }

    async fn reconcile_source_mutation(
        &self,
        intent: &MiniAppSourceMutationIntentRow,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let source = self
            .stores
            .source
            .current_snapshot(
                &intent.owner_user_id,
                &intent.miniapp_id,
                &intent.project_id,
            )
            .map_err(|error| store_error("Source recovery", error))?;
        let source_digest = source.source_snapshot_digest.as_ref();
        let generation = to_i64(source.project.build_generation, "Source recovery generation")?;
        if source_digest == intent.expected_source_digest
            && generation == intent.expected_build_generation
        {
            self.repository
                .abort_source_mutation(&AbortMiniAppSourceMutationParams {
                    intent_id: intent.intent_id.clone(),
                    owner_user_id: intent.owner_user_id.clone(),
                    miniapp_id: intent.miniapp_id.clone(),
                    project_id: intent.project_id.clone(),
                })
                .await?;
            return Ok(());
        }
        if source_digest == intent.next_source_digest
            && generation == intent.next_build_generation
        {
            self.repository
                .finalize_source_mutation(&FinalizeMiniAppSourceMutationParams {
                    intent_id: intent.intent_id.clone(),
                    owner_user_id: intent.owner_user_id.clone(),
                    miniapp_id: intent.miniapp_id.clone(),
                    project_id: intent.project_id.clone(),
                    updated_at: positive_now_ms().max(intent.created_at.saturating_add(1)),
                })
                .await?;
            return Ok(());
        }
        Err(MiniAppM1ApplicationError::Runtime(
            "MiniApp Source mutation recovery found neither the exact old nor new Source head"
                .into(),
        ))
    }

    pub async fn create(
        &self,
        owner_user_id: &str,
        request: CreateMiniAppProjectRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let expected_library_revision = to_i64(
            request.expected_library_revision,
            "library revision",
        )?;
        let stores = &self.stores;
        let miniapp_id = Uuid::now_v7().to_string();
        let project_id = Uuid::now_v7().to_string();
        let source = match request.kind {
            MiniAppKindDto::UiOnly => stores.source.create_project(
                owner_user_id,
                &miniapp_id,
                &project_id,
                request.display_name.clone(),
            ),
            MiniAppKindDto::Service => stores.source.create_service_project(
                owner_user_id,
                &miniapp_id,
                &project_id,
                request.display_name.clone(),
                default_service_module(),
            ),
        }
            .map_err(|error| store_error("Source Store", error))?;
        let create = CreateMiniAppM1Params {
            owner_user_id: owner_user_id.to_owned(),
            miniapp_id: miniapp_id.clone(),
            project_id: project_id.clone(),
            expected_library_revision,
            display_name: request.display_name,
            description: request.description,
            icon_asset_id: None,
            kind: match request.kind {
                MiniAppKindDto::UiOnly => MiniAppM1Kind::UiOnly,
                MiniAppKindDto::Service => MiniAppM1Kind::Service,
            },
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
        self.workshop_projection(owner_user_id, &snapshot, None)
            .await
    }

    pub async fn build(
        &self,
        owner_user_id: &str,
        request: BuildMiniAppRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        self.reconcile_source_mutations().await?;
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
                bounded_log_tail: vec![
                    if snapshot.product.kind == MiniAppM1Kind::Service.as_str() {
                        "Service Build started".to_owned()
                    } else {
                        "UI-only Build started".to_owned()
                    },
                ],
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
        if let Some(module_path) = &prepared.service_module_path {
            let registration = self
                .service_runtime()
                .await
                .register_module(
                    MiniAppId::from(request.miniapp_id.clone()),
                    DigestHex::from(prepared.release.release_digest.clone()),
                    module_path.clone(),
                )
                .await;
            if let Err(error) = registration {
                return Err(self
                    .finish_failed_build(
                        owner_user_id,
                        &request.miniapp_id,
                        &operation_id,
                        MiniAppM1ApplicationError::Invalid(error.to_string()),
                    )
                    .await);
            }
        }
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
                    if snapshot.product.kind == MiniAppM1Kind::Service.as_str() {
                        "Service Release admitted".to_owned()
                    } else {
                        "UI-only Release admitted".to_owned()
                    },
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
                        Ok(snapshot) => {
                            return self
                                .workshop_projection(owner_user_id, &snapshot, None)
                                .await
                        }
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
                            return self
                                .workshop_projection(owner_user_id, &observed, None)
                                .await;
                        }
                    }
                }
                self.workshop_projection(owner_user_id, &snapshot, None)
                    .await
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

    pub async fn test_ready_service(
        &self,
        owner_user_id: &str,
        request: TestMiniAppReleaseRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        validate_request_identity(&request.project_id, "project_id")?;
        validate_request_identity(&request.release_id, "release_id")?;
        validate_digest_string(
            &request.expected_release_digest,
            "expected Ready Release digest",
        )?;
        validate_digest_string(
            &request.resolved_test_input_digest,
            "resolved Service Test input digest",
        )?;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        require_release_mutation(&snapshot)?;
        require_no_running_build(&*self.repository, owner_user_id, &request.miniapp_id).await?;
        if snapshot.product.kind != MiniAppM1Kind::Service.as_str() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Service Test requires a Service MiniApp".into(),
            ));
        }
        let ready = snapshot
            .ready_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Ready Release".into()))?;
        if snapshot.product.product_revision
            != to_i64(request.expected_product_revision, "product revision")?
            || snapshot.product.pointer_revision
                != to_i64(request.expected_pointer_revision, "pointer revision")?
            || snapshot.project.project_id != request.project_id
            || snapshot.project.project_revision
                != to_i64(request.expected_project_revision, "project revision")?
            || snapshot.project.build_generation
                != to_i64(request.expected_build_generation, "build generation")?
            || snapshot.product.config_revision
                != to_i64(request.expected_config_revision, "config revision")?
            || snapshot.product.credential_bindings_revision
                != to_i64(
                    request.expected_credential_bindings_revision,
                    "credential bindings revision",
                )?
            || ready.release_id != request.release_id
            || ready.release_digest != request.expected_release_digest
        {
            return Err(MiniAppM1ApplicationError::Database(
                nomifun_db::DbError::Conflict(
                    "MiniApp Service Test request is stale against the exact Ready state"
                        .into(),
                ),
            ));
        }
        let stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            ready,
        )?;
        let descriptor = stored
            .artifact
            .manifest
            .payload
            .service
            .clone()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "Ready Release has no Service descriptor".into(),
                )
            })?;
        let ready_contract: MiniAppReadyRelease =
            serde_json::from_str(&ready.release_record_json).map_err(|error| {
                MiniAppM1ApplicationError::Runtime(format!(
                    "Ready Release record is invalid: {error}"
                ))
            })?;
        ready_contract
            .validate_for_artifact(&stored.artifact)
            .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))?;
        let prospective_epoch = nonnegative_u64(
            snapshot.product.active_release_epoch,
            "MiniApp active release epoch",
        )?
        .checked_add(1)
        .ok_or_else(|| {
            MiniAppM1ApplicationError::Invalid(
                "MiniApp active release epoch overflow".into(),
            )
        })?;
        let runtime = self.service_runtime().await;
        let miniapp_id = MiniAppId::from(request.miniapp_id.clone());
        let restore_running = matches!(
            runtime.state(&miniapp_id).await,
            Some(crate::MiniAppServiceHostState::Running { .. })
                | Some(crate::MiniAppServiceHostState::Starting { .. })
        );
        runtime.stop(&miniapp_id).await.map_err(|error| {
            MiniAppM1ApplicationError::Runtime(format!(
                "cannot stop production Service before Test: {error}"
            ))
        })?;

        let receipt_id = Uuid::now_v7().to_string();
        let test_result = async {
            let mut storage = runtime
                .create_service_test_storage(
                    owner_user_id,
                    &miniapp_id,
                    &receipt_id,
                    descriptor.uses_files,
                    descriptor.uses_private_database,
                )
                .await
                .map_err(|error| {
                    MiniAppM1ApplicationError::Runtime(format!(
                        "cannot create Service Test storage: {error}"
                    ))
                })?;
            let migrations = stored.artifact.manifest.payload.migrations.clone();
            if !migrations.is_empty() {
                let database = storage
                    .descriptor
                    .private_database
                    .as_ref()
                    .ok_or_else(|| {
                        MiniAppM1ApplicationError::Runtime(
                            "Service Test migrations require a Private Database".into(),
                        )
                    })?;
                let ledger = runtime
                    .apply_storage_migrations(
                        owner_user_id,
                        &miniapp_id,
                        &storage.descriptor,
                        &database.migration_ledger_digest,
                        &release_contract_ref(ready),
                        &migrations,
                        positive_now_ms(),
                    )
                    .await
                    .map_err(|error| {
                        MiniAppM1ApplicationError::Runtime(format!(
                            "Service Test migration failed: {error}"
                        ))
                    })?;
                let database = storage
                    .descriptor
                    .private_database
                    .as_mut()
                    .expect("migration database was checked");
                database.schema_epoch = ledger.schema_epoch;
                database.migration_ledger_digest = ledger.ledger_digest.clone();
                storage.migration_ledger = Some(ledger);
            }
            let spec = self
                .resolve_service_spec_with_storage(
                    &snapshot,
                    ready,
                    prospective_epoch,
                    owner_user_id,
                    Some(storage.descriptor.clone()),
                )
                .await?
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Runtime(
                        "Ready Release lost its Service descriptor".into(),
                    )
                })?;
            let contributions = &stored.artifact.manifest.payload.contributions;
            runtime
                .run_service_test(MiniAppServiceTestRunInput {
                    receipt_id: MiniAppServiceTestReceiptId::from(receipt_id.clone()),
                    ready: ready_contract,
                    spec,
                    resolved_test_input_digest: DigestHex::from(
                        request.resolved_test_input_digest.clone(),
                    ),
                    copied_kv_digest: storage.copied_kv_digest,
                    copied_private_database_digest: storage
                        .copied_private_database_digest,
                    empty_files_dir: storage.empty_files_dir,
                    migration_ledger_digest: storage
                        .migration_ledger
                        .map(|ledger| ledger.ledger_digest),
                    requires_managed_input: contributions
                        != &PackageContributions::default(),
                })
                .await
                .map_err(|error| {
                    MiniAppM1ApplicationError::Runtime(format!(
                        "Service Test Host failed: {error}"
                    ))
                })
        }
        .await;

        let cleanup_result = runtime
            .purge_service_test_storage(owner_user_id, &miniapp_id, &receipt_id)
            .await;
        let restore_result = self
            .restore_service_after_test(owner_user_id, &snapshot, restore_running)
            .await;
        if let Err(error) = cleanup_result {
            return Err(MiniAppM1ApplicationError::Runtime(format!(
                "Service Test storage cleanup failed: {error}"
            )));
        }
        restore_result?;
        let receipt = test_result?;
        let receipt_json = serde_json::to_value(&receipt).map_err(|error| {
            MiniAppM1ApplicationError::Runtime(format!(
                "Service Test receipt serialization failed: {error}"
            ))
        })?;
        let receipt_digest = digest_payload(&receipt)
            .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))?;
        let runtime_digest = digest_payload(&receipt.runtime)
            .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))?;
        let committed = self
            .repository
            .record_service_test_receipt_cas(
                &RecordMiniAppM1ServiceTestReceiptParams {
                    owner_user_id: owner_user_id.to_owned(),
                    miniapp_id: request.miniapp_id,
                    expected_product_revision: snapshot.product.product_revision,
                    expected_pointer_revision: snapshot.product.pointer_revision,
                    expected_config_revision: snapshot.product.config_revision,
                    expected_credential_bindings_revision: snapshot
                        .product
                        .credential_bindings_revision,
                    expected_ready_release_id: ready.release_id.clone(),
                    expected_ready_release_digest: ready.release_digest.clone(),
                    receipt_id: receipt.receipt_id.as_ref().to_owned(),
                    service_run_key: receipt.service_run_key.as_ref().to_owned(),
                    outcome: receipt.outcome,
                    error_code: receipt.error_code.as_ref().map(|value| {
                        value.as_ref().to_owned()
                    }),
                    receipt_digest: receipt_digest.as_ref().to_owned(),
                    runtime_fingerprint_digest: runtime_digest.as_ref().to_owned(),
                    resolved_test_input_digest: receipt
                        .resolved_test_input_digest
                        .as_ref()
                        .to_owned(),
                    receipt: receipt_json,
                    issued_at_ms: receipt.issued_at_ms,
                },
            )
            .await?;
        self.workshop_projection(owner_user_id, &committed, None)
            .await
    }

    async fn restore_service_after_test(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
        restore_running: bool,
    ) -> Result<(), MiniAppM1ApplicationError> {
        if snapshot.product.lifecycle != "enabled" {
            return Ok(());
        }
        let active = snapshot.active_release.as_ref().ok_or_else(|| {
            MiniAppM1ApplicationError::Runtime(
                "enabled Service MiniApp lost its Active Release during Test".into(),
            )
        })?;
        let spec = self
            .resolve_service_spec(
                snapshot,
                active,
                positive_u64(
                    snapshot.product.active_release_epoch,
                    "MiniApp active release epoch",
                )?,
                owner_user_id,
            )
            .await?
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Runtime(
                    "Active Release lost its Service descriptor during Test".into(),
                )
            })?;
        let runtime = self.service_runtime().await;
        runtime
            .bind_active(spec.clone(), true)
            .await
            .map_err(|error| {
                MiniAppM1ApplicationError::Runtime(format!(
                    "cannot restore Active Service after Test: {error}"
                ))
            })?;
        if restore_running {
            runtime.start(spec).await.map_err(|error| {
                MiniAppM1ApplicationError::Runtime(format!(
                    "cannot restart Active Service after Test: {error}"
                ))
            })?;
        }
        Ok(())
    }

    pub async fn export_share(
        &self,
        owner_user_id: &str,
        request: ShareMiniAppRequest,
    ) -> Result<DurableOperationSummaryDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        require_release_mutation(&snapshot)?;
        if snapshot.product.product_revision
            != to_i64(request.expected_product_revision, "product revision")?
            || snapshot.product.pointer_revision
                != to_i64(request.expected_pointer_revision, "pointer revision")?
        {
            return Err(nomifun_db::DbError::Conflict(
                "MiniApp Share Export request is stale".into(),
            )
            .into());
        }
        let release = match request.content {
            MiniAppShareContentDto::ReadyRelease => snapshot.ready_release.as_ref(),
            MiniAppShareContentDto::ActiveRelease => snapshot.active_release.as_ref(),
        }
        .ok_or_else(|| {
            MiniAppM1ApplicationError::Invalid(
                "selected MiniApp Share Release is unavailable".into(),
            )
        })?;
        if release.release_id != request.release_id
            || release.release_digest != request.expected_release_digest
        {
            return Err(nomifun_db::DbError::Conflict(
                "MiniApp Share Export Release changed".into(),
            )
            .into());
        }
        let stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            release,
        )?;
        let source_snapshot = if request.include_source {
            let project_id = release.project_id.as_deref().ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "selected Release has no Project lineage".into(),
                )
            })?;
            let source_digest = release.source_snapshot_digest.as_deref().ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "selected Release has no editable Source lineage".into(),
                )
            })?;
            Some(
                self.stores
                    .source
                    .read_snapshot(
                        owner_user_id,
                        &request.miniapp_id,
                        project_id,
                        source_digest,
                    )
                    .map_err(|error| store_error("Share Source", error))?,
            )
        } else {
            None
        };
        let operation_id = Uuid::now_v7().to_string();
        let started_at_ms = positive_now_ms().max(snapshot.product.updated_at);
        self.repository
            .start_export_operation(&StartMiniAppM1ExportOperationParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                operation_id: operation_id.clone(),
                expected_product_revision: snapshot.product.product_revision,
                expected_pointer_revision: snapshot.product.pointer_revision,
                bounded_log_tail: vec!["MiniApp Share Export started".into()],
                started_at_ms,
            })
            .await?;
        let exported = MiniAppShareBundleFilesystem::default().export(
            MiniAppShareBundleExport {
                bundle_id: Uuid::now_v7().to_string().into(),
                source_miniapp_id: Some(MiniAppId::from(request.miniapp_id.clone())),
                release: &stored,
                source: source_snapshot.as_ref().map(|snapshot| MiniAppShareSourceExport {
                    snapshot,
                    source_archive_artifact_id: Uuid::now_v7().to_string().into(),
                    dependency_lock_artifact_id: Uuid::now_v7().to_string().into(),
                }),
                test_provenance: None,
            },
            &request.destination_path,
        );
        let operation = match exported {
            Ok(bundle) => {
                self.repository
                    .finish_export_operation(&FinishMiniAppM1ExportOperationParams {
                        owner_user_id: owner_user_id.to_owned(),
                        miniapp_id: request.miniapp_id,
                        operation_id,
                        bounded_log_tail: vec![format!(
                            "MiniApp Share Bundle {} exported",
                            bundle.bundle_digest.as_ref()
                        )],
                        finished_at_ms: positive_now_ms().max(started_at_ms),
                    })
                    .await?
            }
            Err(error) => {
                let _ = self
                    .repository
                    .fail_export_operation(&FailMiniAppM1ExportOperationParams {
                        owner_user_id: owner_user_id.to_owned(),
                        miniapp_id: request.miniapp_id,
                        operation_id,
                        progress_percent: 0,
                        error_code: "miniapp_share_export_failed".into(),
                        bounded_log_tail: vec![bounded_log_line(&error.to_string())],
                        finished_at_ms: positive_now_ms().max(started_at_ms),
                    })
                    .await;
                return Err(MiniAppM1ApplicationError::Runtime(error.to_string()));
            }
        };
        operation_summary(&operation)
    }

    pub async fn export_backup(
        &self,
        owner_user_id: &str,
        request: ExportMiniAppBackupRequest,
    ) -> Result<DurableOperationSummaryDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        if request.expected_lifecycle != MiniAppLifecycleDto::Disabled
            || request.destination_path.trim().is_empty()
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Whole-App Backup requires an explicit disabled lifecycle and destination"
                    .into(),
            ));
        }
        let initial = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if initial.product.lifecycle != "disabled" {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Whole-App Backup requires a disabled MiniApp".into(),
            ));
        }
        let miniapp_id = MiniAppId::from(request.miniapp_id.clone());
        let runtime = self.service_runtime().await;
        if initial.product.kind == MiniAppM1Kind::Service.as_str() {
            runtime.stop(&miniapp_id).await.map_err(|error| {
                MiniAppM1ApplicationError::Runtime(format!(
                    "cannot stop MiniApp Service before Whole-App Backup: {error}"
                ))
            })?;
            if runtime
                .state(&miniapp_id)
                .await
                .is_some_and(|state| state != crate::MiniAppServiceHostState::Stopped)
            {
                return Err(MiniAppM1ApplicationError::Invalid(
                    "Whole-App Backup owner is still busy".into(),
                ));
            }
        }

        let operation_id = Uuid::now_v7().to_string();
        let started_at_ms = positive_now_ms().max(initial.product.updated_at);
        let captured = self
            .repository
            .start_backup_export(&StartMiniAppM1BackupExportParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                operation_id: operation_id.clone(),
                expected_product_revision: to_i64(
                    request.expected_product_revision,
                    "product revision",
                )?,
                expected_pointer_revision: to_i64(
                    request.expected_pointer_revision,
                    "pointer revision",
                )?,
                expected_config_revision: to_i64(
                    request.expected_config_revision,
                    "config revision",
                )?,
                expected_credential_bindings_revision: to_i64(
                    request.expected_credential_bindings_revision,
                    "credential bindings revision",
                )?,
                started_at_ms,
            })
            .await?;
        let result = async {
            let prepared = self
                .prepare_backup_export(owner_user_id, &captured)
                .await?;
            MiniAppWholeAppBackupFilesystem::new()
                .export(
                    MiniAppWholeAppBackupExport {
                        backup_id: Uuid::now_v7().to_string().into(),
                        source_miniapp_id: miniapp_id,
                        owner_quiescent: true,
                        created_at_ms: positive_now_ms().max(started_at_ms),
                        product: &prepared.product,
                        project: &prepared.project,
                        config: &prepared.config,
                        credential_slots: &prepared.credential_slots,
                        releases: &prepared.releases,
                        source: prepared.source.as_ref(),
                        storage: &prepared.storage,
                    },
                    &request.destination_path,
                )
                .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))
        }
        .await;
        let operation = match result {
            Ok(metadata) => {
                self.repository
                    .finish_export_operation(&FinishMiniAppM1ExportOperationParams {
                        owner_user_id: owner_user_id.to_owned(),
                        miniapp_id: request.miniapp_id,
                        operation_id,
                        bounded_log_tail: vec![format!(
                            "MiniApp Whole-App Backup {} exported",
                            metadata.metadata_digest().map_err(|error| {
                                MiniAppM1ApplicationError::Invalid(error.to_string())
                            })?.as_ref()
                        )],
                        finished_at_ms: positive_now_ms().max(started_at_ms),
                    })
                    .await?
            }
            Err(error) => {
                let _ = self
                    .repository
                    .fail_export_operation(&FailMiniAppM1ExportOperationParams {
                        owner_user_id: owner_user_id.to_owned(),
                        miniapp_id: request.miniapp_id,
                        operation_id,
                        progress_percent: 0,
                        error_code: "miniapp_backup_export_failed".into(),
                        bounded_log_tail: vec![bounded_log_line(&error.to_string())],
                        finished_at_ms: positive_now_ms().max(started_at_ms),
                    })
                    .await;
                return Err(error);
            }
        };
        operation_summary(&operation)
    }

    async fn prepare_backup_export(
        &self,
        owner_user_id: &str,
        captured: &MiniAppM1BackupExportSnapshot,
    ) -> Result<PreparedApplicationBackup, MiniAppM1ApplicationError> {
        let snapshot = &captured.snapshot;
        let mut product_metadata = BackupProductMetadata {
            miniapp_id: snapshot.product.miniapp_id.clone(),
            product_revision: snapshot.product.product_revision,
            display_name: snapshot.product.display_name.clone(),
            description: snapshot.product.description.clone(),
            icon_asset_id: snapshot.product.icon_asset_id.clone(),
            kind: snapshot.product.kind.clone(),
            lifecycle: snapshot.product.lifecycle.clone(),
            pointer_revision: snapshot.product.pointer_revision,
            active_release_epoch: snapshot.product.active_release_epoch,
            materialized_catalog_digest: snapshot
                .product
                .materialized_catalog_digest
                .clone(),
            ready_release_id: snapshot.product.ready_release_id.clone(),
            ready_release_digest: snapshot.product.ready_release_digest.clone(),
            active_release_id: snapshot.product.active_release_id.clone(),
            active_release_digest: snapshot.product.active_release_digest.clone(),
            previous_release_id: snapshot.product.previous_release_id.clone(),
            previous_release_digest: snapshot.product.previous_release_digest.clone(),
            release_lineage: BTreeMap::new(),
        };
        let project = serde_json::to_value(BackupProjectMetadata {
            miniapp_id: snapshot.project.miniapp_id.clone(),
            project_id: snapshot.project.project_id.clone(),
            project_revision: snapshot.project.project_revision,
            source_state: snapshot.project.source_state.clone(),
            build_generation: snapshot.project.build_generation,
            source_snapshot_digest: snapshot.project.source_head_digest.clone(),
            dependency_lock_digest: snapshot.project.dependency_lock_digest.clone(),
            build_profile_version: snapshot.project.build_profile_version.clone(),
        })
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let schema: Value = serde_json::from_str(&snapshot.product.config_schema_json)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let values: Value = serde_json::from_str(&snapshot.product.config_json)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let config = serde_json::to_value(BackupConfigMetadata {
            config_revision: snapshot.product.config_revision,
            schema_digest: digest_payload(&schema)
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?
                .as_ref()
                .to_owned(),
            schema,
            values,
        })
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;

        let mut releases = BTreeMap::new();
        let mut credential_slots = BTreeMap::<String, CredentialSlotDeclaration>::new();
        let slot_for = |release_id: &str| {
            if snapshot.product.ready_release_id.as_deref() == Some(release_id) {
                Some("ready")
            } else if snapshot.product.active_release_id.as_deref() == Some(release_id) {
                Some("active")
            } else if snapshot.product.previous_release_id.as_deref() == Some(release_id) {
                Some("previous")
            } else {
                None
            }
        };
        for release in &captured.releases {
            let slot = slot_for(&release.release_id).ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "Whole-App Backup captured a Release outside the retained pointers".into(),
                )
            })?;
            let stored = self.load_verified_release(
                owner_user_id,
                &snapshot.project.project_id,
                release,
            )?;
            product_metadata.release_lineage.insert(
                slot.to_owned(),
                BackupReleaseMetadata {
                    release_id: release.release_id.clone(),
                    artifact_id: release.artifact_id.clone(),
                    artifact_digest: release.artifact_digest.clone(),
                    manifest_digest: release.manifest_digest.clone(),
                    source_kind: release.source_kind.clone(),
                    project_id: release.project_id.clone(),
                    source_snapshot_digest: release.source_snapshot_digest.clone(),
                    dependency_lock_digest: release.dependency_lock_digest.clone(),
                    build_profile_version: release.build_profile_version.clone(),
                    build_generation: release.build_generation,
                },
            );
            for declaration in &stored.artifact.manifest.payload.credential_slots {
                match credential_slots.get(declaration.slot_key.as_ref()) {
                    Some(existing) if existing != declaration => {
                        return Err(MiniAppM1ApplicationError::Invalid(format!(
                            "Credential slot {} changed across retained Releases",
                            declaration.slot_key.as_ref()
                        )));
                    }
                    Some(_) => {}
                    None => {
                        credential_slots.insert(
                            declaration.slot_key.as_ref().to_owned(),
                            declaration.clone(),
                        );
                    }
                }
            }
            releases.insert(
                slot.to_owned(),
                MiniAppBackupRelease {
                    artifact: stored.artifact,
                    manifest_bytes: stored.manifest_bytes,
                    files: stored
                        .files
                        .into_iter()
                        .map(|file| {
                            MiniAppBackupFile::new(
                                file.normalized_relative_path,
                                file.bytes,
                            )
                        })
                        .collect(),
                },
            );
        }
        let product = serde_json::to_value(&product_metadata)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let credential_slots = serde_json::to_value(
            credential_slots.into_values().collect::<Vec<_>>(),
        )
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let source = if snapshot.project.source_state == "editable" {
            let digest = snapshot
                .project
                .source_head_digest
                .as_deref()
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "editable Backup Project has no Source digest".into(),
                    )
                })?;
            let source = self
                .stores
                .source
                .read_snapshot(
                    owner_user_id,
                    &snapshot.product.miniapp_id,
                    &snapshot.project.project_id,
                    digest,
                )
                .map_err(|error| store_error("Backup Source", error))?;
            Some(MiniAppBackupSource {
                source: MiniAppSourceBundle {
                    project_id: source.project.project_id.clone(),
                    source_archive_artifact_id: Uuid::now_v7().to_string().into(),
                    source_snapshot_digest: source.source_snapshot_digest.clone(),
                    dependency_lock_artifact_id: Uuid::now_v7().to_string().into(),
                    dependency_lock_digest: source.dependency_lock_digest.clone(),
                    build_profile_version: source.project.build_profile_version.clone(),
                },
                dependency_lock: source.dependency_lock,
                files: source
                    .files
                    .into_iter()
                    .map(|file| {
                        MiniAppBackupFile::new(
                            file.normalized_relative_path,
                            file.bytes,
                        )
                    })
                    .collect(),
            })
        } else {
            None
        };

        let mut storage = MiniAppBackupStorage {
            kv: captured
                .kv
                .iter()
                .map(|row| {
                    serde_json::to_value(BackupKvMetadata {
                        namespace: row.namespace.clone(),
                        key: row.key.clone(),
                        value_json: row.value_json.clone(),
                        revision: row.revision,
                        key_generation: row.key_generation,
                        is_tombstone: row.is_tombstone,
                        created_at: row.created_at,
                        updated_at: row.updated_at,
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?,
            files: Vec::new(),
            private_database: None,
            migration_ledger: None,
        };
        if snapshot.product.kind == MiniAppM1Kind::Service.as_str() {
            let retained = snapshot
                .active_release
                .as_ref()
                .or(snapshot.ready_release.as_ref())
                .or(snapshot.previous_release.as_ref());
            let (uses_files, uses_private_database) = match retained {
                Some(release) => {
                    let stored = self.load_verified_release(
                        owner_user_id,
                        &snapshot.project.project_id,
                        release,
                    )?;
                    stored
                        .artifact
                        .manifest
                        .payload
                        .service
                        .as_ref()
                        .map(|service| {
                            (service.uses_files, service.uses_private_database)
                        })
                        .unwrap_or((false, false))
                }
                None => (false, false),
            };
            let mut managed = self
                .service_runtime()
                .await
                .export_backup_storage(
                    owner_user_id,
                    &MiniAppId::from(snapshot.product.miniapp_id.clone()),
                    uses_files,
                    uses_private_database,
                )
                .await
                .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))?;
            managed.kv = storage.kv;
            storage = managed;
        }
        if let Some(ledger) = &storage.migration_ledger {
            let retained = product_metadata
                .release_lineage
                .values()
                .map(|release| {
                    (
                        release.release_id.as_str(),
                        release.artifact_id.as_str(),
                        release.artifact_digest.as_str(),
                        release.manifest_digest.as_str(),
                    )
                })
                .collect::<BTreeSet<_>>();
            if ledger.entries.iter().any(|entry| {
                !retained.contains(&(
                    entry.release.release_id.as_ref(),
                    entry.release.artifact_id.as_ref(),
                    entry.release.release_digest.as_ref(),
                    entry.release.manifest_digest.as_ref(),
                ))
            }) {
                return Err(MiniAppM1ApplicationError::Invalid(
                    "Whole-App Backup migration ledger references a non-retained Release".into(),
                ));
            }
        }

        Ok(PreparedApplicationBackup {
            product,
            project,
            config,
            credential_slots,
            releases,
            source,
            storage,
        })
    }

    pub async fn import_share(
        &self,
        owner_user_id: &str,
        request: ImportMiniAppShareRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let imported = MiniAppShareBundleFilesystem::default()
            .import_share_bundle(&request.source_path)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        if imported.bundle.bundle_digest.as_ref() != request.expected_bundle_digest
            || imported.release.artifact.artifact_digest.as_ref()
                != request.expected_release_digest
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Share Bundle digest expectations do not match".into(),
            ));
        }
        self.import_as_new(
            owner_user_id,
            request.expected_library_revision,
            request.display_name,
            imported.release,
            imported.source,
        )
        .await
    }

    pub async fn import_prebuilt(
        &self,
        owner_user_id: &str,
        request: ImportMiniAppArtifactRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let release = MiniAppShareBundleFilesystem::default()
            .import_prebuilt_release(&request.source_path)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        if release.artifact.artifact_digest.as_ref() != request.expected_artifact_digest {
            return Err(MiniAppM1ApplicationError::Invalid(
                "prebuilt Release digest expectation does not match".into(),
            ));
        }
        self.import_as_new(
            owner_user_id,
            request.expected_library_revision,
            request.display_name,
            release,
            None,
        )
        .await
    }

    pub async fn import_backup(
        &self,
        owner_user_id: &str,
        request: ImportMiniAppBackupRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        validate_digest_string(
            &request.expected_backup_metadata_digest,
            "Whole-App Backup metadata digest",
        )?;
        if request.display_name.trim().is_empty() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp display name is required".into(),
            ));
        }
        let imported = MiniAppWholeAppBackupFilesystem::new()
            .import(&request.source_path)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let metadata_digest = imported
            .metadata
            .metadata_digest()
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        if metadata_digest.as_ref() != request.expected_backup_metadata_digest {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Whole-App Backup metadata digest expectation does not match".into(),
            ));
        }
        let product: BackupProductMetadata =
            serde_json::from_value(imported.product.clone()).map_err(|error| {
                MiniAppM1ApplicationError::Invalid(format!(
                    "Whole-App Backup product metadata is invalid: {error}"
                ))
            })?;
        let project: BackupProjectMetadata =
            serde_json::from_value(imported.project.clone()).map_err(|error| {
                MiniAppM1ApplicationError::Invalid(format!(
                    "Whole-App Backup project metadata is invalid: {error}"
                ))
            })?;
        let config: BackupConfigMetadata =
            serde_json::from_value(imported.config.clone()).map_err(|error| {
                MiniAppM1ApplicationError::Invalid(format!(
                    "Whole-App Backup config metadata is invalid: {error}"
                ))
            })?;
        let credential_slots: Vec<CredentialSlotDeclaration> =
            serde_json::from_value(imported.credential_slots.clone()).map_err(|error| {
                MiniAppM1ApplicationError::Invalid(format!(
                    "Whole-App Backup credential slots are invalid: {error}"
                ))
            })?;
        let credential_slot_keys = credential_slots
            .iter()
            .map(|slot| slot.slot_key.as_ref().to_owned())
            .collect::<BTreeSet<_>>();
        if credential_slot_keys != imported.metadata.credential_slot_keys {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Whole-App Backup credential slot inventory does not match metadata".into(),
            ));
        }
        validate_backup_credential_slot_union(&credential_slots, &imported.releases)?;
        let expected_release_slots = [
            ("ready", product.ready_release_id.is_some()),
            ("active", product.active_release_id.is_some()),
            ("previous", product.previous_release_id.is_some()),
        ]
        .into_iter()
        .filter_map(|(slot, present)| present.then_some(slot))
        .collect::<BTreeSet<_>>();
        let actual_release_slots = imported.releases.keys().map(String::as_str).collect();
        if expected_release_slots != actual_release_slots {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Whole-App Backup Release inventory does not match Product pointers".into(),
            ));
        }
        if product.miniapp_id != imported.metadata.source_miniapp_id.as_ref()
            || project.miniapp_id != product.miniapp_id
            || project.project_id.trim().is_empty()
            || product.lifecycle != "disabled"
            || !matches!(product.kind.as_str(), "ui_only" | "service")
            || (project.source_state == "editable") != imported.source.is_some()
            || (project.source_state != "editable"
                && project.source_state != "runtime_only"
                && project.source_state != "empty")
            || (product.active_release_id.is_some() != (product.active_release_epoch > 0))
            || product.product_revision < 1
            || product.pointer_revision < 1
            || project.project_revision < 1
            || (project.source_state == "editable" && project.build_generation < 1)
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Whole-App Backup identity, lifecycle, or Source state is inconsistent".into(),
            ));
        }
        if config.schema_digest
            != digest_payload(&config.schema)
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?
                .as_ref()
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Whole-App Backup configuration schema digest is invalid".into(),
            ));
        }
        if !config.schema.is_object() || !config.values.is_object() {
            return Err(MiniAppM1ApplicationError::Invalid(
                "Whole-App Backup configuration must contain JSON objects".into(),
            ));
        }
        let kind = if product.kind == "service" {
            MiniAppM1Kind::Service
        } else {
            MiniAppM1Kind::UiOnly
        };
        let miniapp_id = Uuid::now_v7().to_string();
        let project_id = Uuid::now_v7().to_string();
        let operation_id = Uuid::now_v7().to_string();
        let started_at_ms = positive_now_ms();
        let source_lineage = imported.source.as_ref().map(|source| {
            MiniAppM1ImportSource::Managed(MiniAppM1ManagedSourceLineage {
                managed_source_path: format!(
                    "sources/{owner_user_id}/miniapps/{miniapp_id}/projects/{project_id}/source"
                ),
                source_head_digest: source.source.source_snapshot_digest.as_ref().to_owned(),
                dependency_lock_digest: source.source.dependency_lock_digest.as_ref().to_owned(),
                build_profile_version: source.source.build_profile_version.as_ref().to_owned(),
                build_generation: project.build_generation.max(1),
            })
        });
        let begun = self
            .repository
            .begin_import_as_new(&BeginMiniAppM1ImportAsNewParams {
                create: CreateMiniAppM1Params {
                    owner_user_id: owner_user_id.to_owned(),
                    miniapp_id: miniapp_id.clone(),
                    project_id: project_id.clone(),
                    expected_library_revision: to_i64(
                        request.expected_library_revision,
                        "library revision",
                    )?,
                    display_name: request.display_name.trim().to_owned(),
                    description: product.description.clone(),
                    icon_asset_id: None,
                    kind,
                    materialized_catalog_digest: digest_bytes(b"miniapp-m1-empty-catalog")
                        .as_ref()
                        .to_owned(),
                    config_schema_json: canonical_json_string(&config.schema)?,
                    config_json: canonical_json_string(&config.values)?,
                    created_at: started_at_ms,
                },
                operation_id: operation_id.clone(),
                source: source_lineage
                    .clone()
                    .unwrap_or(MiniAppM1ImportSource::RuntimeOnly),
                bounded_log_tail: vec!["MiniApp Whole-App Backup import started".into()],
                started_at_ms,
            })
            .await?;

        let imported_releases = imported.releases;
        let imported_source = imported.source;
        let mut imported_storage = imported.storage;
        let kv_payloads = imported_storage.kv.clone();
        let result = async {
            if let Some(source) = imported_source {
                let build_generation = project.build_generation.max(1) as u64;
                self.stores
                    .source
                    .import_project_exact(MiniAppSourceExactImportRequest {
                        scope: MiniAppSourceScope::new(
                            owner_user_id,
                            &miniapp_id,
                            &project_id,
                        )
                        .map_err(|error| store_error("Backup Source scope", error))?,
                        display_name: request.display_name.trim().to_owned(),
                        content_kind: if kind == MiniAppM1Kind::Service {
                            MiniAppSourceContentKind::Service
                        } else {
                            MiniAppSourceContentKind::UiOnly
                        },
                        dependency_lock: source.dependency_lock,
                        files: source
                            .files
                            .into_iter()
                            .map(|file| {
                                MiniAppSourceFileInput::new(file.relative_path, file.bytes)
                            })
                            .collect(),
                        expected_source_snapshot_digest: source.source.source_snapshot_digest,
                        expected_dependency_lock_digest: source.source.dependency_lock_digest,
                        build_generation,
                        build_profile_version: source.source.build_profile_version,
                    })
                    .map_err(|error| store_error("Backup Source import", error))?;
            }

            let mut release_refs = BTreeMap::new();
            let mut release_items = Vec::with_capacity(imported_releases.len());
            for (slot, release) in &imported_releases {
                let slot = match slot.as_str() {
                    "ready" => MiniAppM1BackupReleaseSlot::Ready,
                    "active" => MiniAppM1BackupReleaseSlot::Active,
                    "previous" => MiniAppM1BackupReleaseSlot::Previous,
                    _ => {
                        return Err(MiniAppM1ApplicationError::Invalid(
                            "Whole-App Backup contains an unsupported Release slot".into(),
                        ));
                    }
                };
                let old_id = match slot {
                    MiniAppM1BackupReleaseSlot::Ready => product.ready_release_id.as_deref(),
                    MiniAppM1BackupReleaseSlot::Active => product.active_release_id.as_deref(),
                    MiniAppM1BackupReleaseSlot::Previous => {
                        product.previous_release_id.as_deref()
                    }
                }
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "Whole-App Backup Release slot is not referenced by Product metadata"
                            .into(),
                    )
                })?;
                let slot_name = slot.as_str();
                let release_metadata = product.release_lineage.get(slot_name).ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(format!(
                        "Whole-App Backup is missing lineage metadata for Release slot {slot_name}"
                    ))
                })?;
                let artifact = &release.artifact;
                if release_metadata.release_id != old_id
                    || release_metadata.artifact_id != artifact.artifact_id.as_ref()
                    || release_metadata.artifact_digest != artifact.artifact_digest.as_ref()
                    || release_metadata.manifest_digest
                        != artifact.manifest.payload_digest.as_ref()
                {
                    return Err(MiniAppM1ApplicationError::Invalid(
                        "Whole-App Backup Release lineage does not match its Artifact".into(),
                    ));
                }
                if (kind == MiniAppM1Kind::Service)
                    != artifact.manifest.payload.service.is_some()
                {
                    return Err(MiniAppM1ApplicationError::Invalid(
                        "Whole-App Backup Release kind differs from Product kind".into(),
                    ));
                }
                let release_id = Uuid::now_v7().to_string();
                let managed_lineage = match release_metadata.source_kind.as_str() {
                    "managed" => {
                        let source_digest = release_metadata
                            .source_snapshot_digest
                            .clone()
                            .ok_or_else(|| {
                                MiniAppM1ApplicationError::Invalid(
                                    "managed Backup Release has no Source digest".into(),
                                )
                            })?;
                        let lock_digest = release_metadata
                            .dependency_lock_digest
                            .clone()
                            .ok_or_else(|| {
                                MiniAppM1ApplicationError::Invalid(
                                    "managed Backup Release has no dependency lock digest".into(),
                                )
                            })?;
                        let build_generation = release_metadata.build_generation.ok_or_else(|| {
                            MiniAppM1ApplicationError::Invalid(
                                "managed Backup Release has no build generation".into(),
                            )
                        })?;
                        if release_metadata.project_id.as_deref() != Some(
                            project.project_id.as_str(),
                        ) || release_metadata.build_profile_version.as_deref()
                            != Some(MINIAPP_RELEASE_PROFILE_VERSION)
                        {
                            return Err(MiniAppM1ApplicationError::Invalid(
                                "managed Backup Release Project lineage is invalid".into(),
                            ));
                        }
                        MiniAppSourceLineage::Managed {
                            project_id: MiniAppProjectId::from(project_id.clone()),
                            source_snapshot_digest: DigestHex::from(source_digest),
                            dependency_lock_digest: DigestHex::from(lock_digest),
                            build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
                            build_generation: u64::try_from(build_generation).map_err(|_| {
                                MiniAppM1ApplicationError::Invalid(
                                    "Backup Release build generation is invalid".into(),
                                )
                            })?,
                        }
                    }
                    "runtime_only" => MiniAppSourceLineage::RuntimeOnly,
                    _ => {
                        return Err(MiniAppM1ApplicationError::Invalid(
                            "Whole-App Backup Release source kind is invalid".into(),
                        ));
                    }
                };
                let is_managed = matches!(
                    managed_lineage,
                    MiniAppSourceLineage::Managed { .. }
                );
                let release_source_digest = release_metadata
                    .source_snapshot_digest
                    .clone()
                    .unwrap_or_else(|| artifact.artifact_digest.as_ref().to_owned());
                let release_lock_digest = release_metadata
                    .dependency_lock_digest
                    .clone()
                    .unwrap_or_else(|| {
                        artifact
                            .manifest
                            .payload
                            .dependency_lock_digest
                            .as_ref()
                            .to_owned()
                    });
                let release_generation = release_metadata
                    .build_generation
                    .and_then(|value| u64::try_from(value).ok())
                    .unwrap_or(1);
                let ready = MiniAppReadyRelease {
                    miniapp_id: MiniAppId::from(miniapp_id.clone()),
                    release: MiniAppReleaseRef {
                        release_id: MiniAppReleaseId::from(release_id.clone()),
                        artifact_id: artifact.artifact_id.clone(),
                        release_digest: artifact.artifact_digest.clone(),
                        manifest_digest: artifact.manifest.payload_digest.clone(),
                    },
                    origin_operation_id: OperationId::from(operation_id.clone()),
                    origin: MiniAppReadyOrigin::Import,
                    source_lineage: managed_lineage.clone(),
                    matching_service_test_receipt: None,
                    created_at_ms: positive_now_ms().max(started_at_ms),
                };
                ready
                    .validate_for_artifact(artifact)
                    .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
                let published = self
                    .stores
                    .release
                    .publish(if kind == MiniAppM1Kind::Service {
                        MiniAppReleasePublishRequest::service(
                            MiniAppSourceScope::new(
                                owner_user_id,
                                &miniapp_id,
                                &project_id,
                            )
                            .map_err(|error| store_error("Backup Release scope", error))?,
                            release_source_digest.clone().into(),
                            release_lock_digest.clone().into(),
                            release_generation,
                            artifact.clone(),
                            release
                                .files
                                .iter()
                                .map(|file| {
                                    MiniAppReleaseFileBytes::new(
                                        file.relative_path.clone(),
                                        file.bytes.clone(),
                                    )
                                })
                                .collect(),
                        )
                    } else {
                        MiniAppReleasePublishRequest::ui_only(
                            MiniAppSourceScope::new(
                                owner_user_id,
                                &miniapp_id,
                                &project_id,
                            )
                            .map_err(|error| store_error("Backup Release scope", error))?,
                            release_source_digest.clone().into(),
                            release_lock_digest.clone().into(),
                            release_generation,
                            artifact.clone(),
                            release
                                .files
                                .iter()
                                .map(|file| {
                                    MiniAppReleaseFileBytes::new(
                                        file.relative_path.clone(),
                                        file.bytes.clone(),
                                    )
                                })
                                .collect(),
                        )
                    })
                    .map_err(|error| store_error("Backup Release import", error))?;
                let artifact = published.stored.artifact;
                let mut release_ref = ready.release.clone();
                release_ref.artifact_id = artifact.artifact_id.clone();
                release_ref.manifest_digest = artifact.manifest.payload_digest.clone();
                release_refs.insert(old_id.to_owned(), release_ref.clone());
                release_items.push(MiniAppM1BackupImportRelease {
                    slot,
                    artifact: MiniAppReleaseArtifactRow {
                        id: 0,
                        artifact_id: artifact.artifact_id.as_ref().to_owned(),
                        owner_user_id: owner_user_id.to_owned(),
                        artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
                        manifest_digest: artifact.manifest.payload_digest.as_ref().to_owned(),
                        artifact_record_json: canonical_json_string(&artifact)?,
                        managed_path: published.stored.managed_relative_path,
                        created_at: ready.created_at_ms,
                    },
                    release: MiniAppReleaseRow {
                        id: 0,
                        release_id,
                        miniapp_id: miniapp_id.clone(),
                        owner_user_id: owner_user_id.to_owned(),
                        artifact_id: artifact.artifact_id.as_ref().to_owned(),
                        artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
                        manifest_digest: artifact.manifest.payload_digest.as_ref().to_owned(),
                        release_digest: artifact.artifact_digest.as_ref().to_owned(),
                        origin_kind: "import".into(),
                        origin_operation_id: operation_id.clone(),
                        source_kind: if matches!(
                            ready.source_lineage,
                            MiniAppSourceLineage::Managed { .. }
                        ) {
                            "managed".into()
                        } else {
                            "runtime_only".into()
                        },
                        project_id: if matches!(
                            ready.source_lineage,
                            MiniAppSourceLineage::Managed { .. }
                        ) {
                            Some(project_id.clone())
                        } else {
                            None
                        },
                        source_snapshot_digest: if is_managed {
                            Some(release_source_digest.clone())
                        } else {
                            None
                        },
                        dependency_lock_digest: if is_managed {
                            Some(release_lock_digest.clone())
                        } else {
                            None
                        },
                        build_profile_version: if is_managed {
                            Some(MINIAPP_RELEASE_PROFILE_VERSION.into())
                        } else {
                            None
                        },
                        build_generation: if is_managed {
                            Some(release_generation as i64)
                        } else {
                            None
                        },
                        release_record_json: canonical_json_string(&ready)?,
                        created_at: ready.created_at_ms,
                    },
                });
            }
            let target_catalog_digest = match product.active_release_id.as_deref() {
                Some(old_active_release_id) => {
                    let active = release_items
                        .iter()
                        .find(|item| item.slot == MiniAppM1BackupReleaseSlot::Active)
                        .ok_or_else(|| {
                            MiniAppM1ApplicationError::Invalid(
                                "Whole-App Backup Active Release is missing".into(),
                            )
                        })?;
                    let active_ref = release_refs
                        .get(old_active_release_id)
                        .ok_or_else(|| {
                            MiniAppM1ApplicationError::Invalid(
                                "Whole-App Backup Active Release identity is missing".into(),
                            )
                        })?;
                    let active_artifact: nomifun_agent_contracts::MiniAppReleaseArtifactV1 =
                        serde_json::from_str(&active.artifact.artifact_record_json).map_err(
                            |error| {
                                MiniAppM1ApplicationError::Invalid(format!(
                                    "imported Active Release Artifact is invalid: {error}"
                                ))
                            },
                        )?;
                    miniapp_catalog_digest(
                        &miniapp_id,
                        active_ref,
                        &active_artifact.manifest.payload.contributions,
                    )?
                    .as_ref()
                    .to_owned()
                }
                None => digest_bytes(b"miniapp-m1-empty-catalog").as_ref().to_owned(),
            };
            let mut ledger = imported_storage.migration_ledger.take();
            if let Some(ledger_value) = ledger.as_mut() {
                *ledger_value = rebind_migration_ledger_for_target_with_releases(
                    ledger_value,
                    MiniAppId::from(miniapp_id.clone()),
                    MiniAppDatabaseHandleId::from(format!("miniapp-db-{miniapp_id}")),
                    &release_refs,
                )
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
            }
            let mut storage = imported_storage;
            storage.migration_ledger = ledger;
            storage.kv.clear();
            if kind == MiniAppM1Kind::Service {
                let active_artifact = release_items
                    .iter()
                    .find(|item| item.slot == MiniAppM1BackupReleaseSlot::Active)
                    .or_else(|| {
                        release_items
                            .iter()
                            .find(|item| item.slot == MiniAppM1BackupReleaseSlot::Ready)
                    })
                    .map(|item| item.artifact.artifact_record_json.clone());
                let (uses_files, uses_private_database) = active_artifact
                    .as_deref()
                    .map(|value| {
                        serde_json::from_str::<nomifun_agent_contracts::MiniAppReleaseArtifactV1>(
                            value,
                        )
                        .ok()
                        .and_then(|artifact| {
                            artifact.manifest.payload.service.map(|service| {
                                (service.uses_files, service.uses_private_database)
                            })
                        })
                        .unwrap_or((false, false))
                    })
                    .unwrap_or((false, false));
                self.service_runtime()
                    .await
                    .import_backup_storage(
                        owner_user_id,
                        &MiniAppId::from(miniapp_id.clone()),
                        storage,
                        uses_files,
                        uses_private_database,
                    )
                    .await
                    .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))?;
            }
            let kv = kv_payloads
                .iter()
                .map(|value| {
                    let row: BackupKvMetadata = serde_json::from_value(value.clone())
                        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
                    Ok(MiniAppKvRow {
                        id: 0,
                        miniapp_id: miniapp_id.clone(),
                        owner_user_id: owner_user_id.to_owned(),
                        namespace: row.namespace,
                        key: row.key,
                        value_json: row.value_json,
                        revision: row.revision,
                        key_generation: row.key_generation,
                        is_tombstone: row.is_tombstone,
                        created_at: row.created_at,
                        updated_at: row.updated_at,
                    })
                })
                .collect::<Result<Vec<_>, MiniAppM1ApplicationError>>()?;
            self.repository
                .finish_backup_import(&FinishMiniAppM1BackupImportParams {
                    owner_user_id: owner_user_id.to_owned(),
                    miniapp_id: miniapp_id.clone(),
                    project_id: project_id.clone(),
                    operation_id: operation_id.clone(),
                    expected_library_revision: begun.snapshot.library_revision,
                    expected_product_revision: begun.snapshot.product.product_revision,
                    expected_pointer_revision: begun.snapshot.product.pointer_revision,
                    expected_project_revision: begun.snapshot.project.project_revision,
                    releases: release_items,
                    kv,
                    target_catalog_digest,
                    finished_at_ms: positive_now_ms().max(started_at_ms),
                })
                .await
                .map_err(MiniAppM1ApplicationError::from)
        }
        .await;
        match result {
            Ok(snapshot) => self.workshop_projection(owner_user_id, &snapshot, None).await,
            Err(error) => {
                let _ = self
                    .repository
                    .fail_import(&FailMiniAppM1ImportParams {
                        owner_user_id: owner_user_id.to_owned(),
                        miniapp_id: miniapp_id.clone(),
                        project_id: project_id.clone(),
                        operation_id,
                        expected_product_revision: begun.snapshot.product.product_revision,
                        expected_pointer_revision: begun.snapshot.product.pointer_revision,
                        expected_project_revision: begun.snapshot.project.project_revision,
                        progress_percent: 0,
                        error_code: "miniapp_backup_import_failed".into(),
                        bounded_log_tail: vec![bounded_log_line(&error.to_string())],
                        finished_at_ms: positive_now_ms().max(started_at_ms),
                    })
                    .await;
                let _ = self
                    .service_runtime()
                    .await
                    .purge_storage(
                        owner_user_id,
                        &MiniAppId::from(miniapp_id.clone()),
                    )
                    .await;
                let _ = self
                    .stores
                    .source
                    .purge_project(owner_user_id, &miniapp_id, &project_id);
                let _ = self
                    .stores
                    .release
                    .purge_project(owner_user_id, &miniapp_id, &project_id);
                Err(error)
            }
        }
    }

    async fn import_as_new(
        &self,
        owner_user_id: &str,
        expected_library_revision: u64,
        display_name: String,
        imported: MiniAppImportedRelease,
        source: Option<MiniAppImportedSource>,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        let miniapp_id = Uuid::now_v7().to_string();
        let project_id = Uuid::now_v7().to_string();
        let operation_id = Uuid::now_v7().to_string();
        let started_at_ms = positive_now_ms();
        let kind = if imported.artifact.manifest.payload.service.is_some() {
            MiniAppM1Kind::Service
        } else {
            MiniAppM1Kind::UiOnly
        };
        let managed_path = format!(
            "sources/{owner_user_id}/miniapps/{miniapp_id}/projects/{project_id}/source"
        );
        let import_source = source.as_ref().map_or(
            MiniAppM1ImportSource::RuntimeOnly,
            |source| {
                MiniAppM1ImportSource::Managed(MiniAppM1ManagedSourceLineage {
                    managed_source_path: managed_path.clone(),
                    source_head_digest: source
                        .source
                        .source_snapshot_digest
                        .as_ref()
                        .to_owned(),
                    dependency_lock_digest: source
                        .source
                        .dependency_lock_digest
                        .as_ref()
                        .to_owned(),
                    build_profile_version: source
                        .source
                        .build_profile_version
                        .as_ref()
                        .to_owned(),
                    build_generation: 1,
                })
            },
        );
        let begun = self
            .repository
            .begin_import_as_new(&BeginMiniAppM1ImportAsNewParams {
                create: CreateMiniAppM1Params {
                    owner_user_id: owner_user_id.to_owned(),
                    miniapp_id: miniapp_id.clone(),
                    project_id: project_id.clone(),
                    expected_library_revision: to_i64(
                        expected_library_revision,
                        "library revision",
                    )?,
                    display_name: display_name.clone(),
                    description: Some(
                        imported.artifact.manifest.payload.display.description.clone(),
                    ),
                    icon_asset_id: None,
                    kind,
                    materialized_catalog_digest: digest_payload(
                        &"miniapp-m1-empty-catalog",
                    )
                    .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?
                    .as_ref()
                    .to_owned(),
                    config_schema_json: canonical_json_string(
                        &imported.artifact.manifest.payload.config_schema.0,
                    )?,
                    config_json: "{}".into(),
                    created_at: started_at_ms,
                },
                operation_id: operation_id.clone(),
                source: import_source,
                bounded_log_tail: vec!["MiniApp Import started".into()],
                started_at_ms,
            })
            .await?;
        let result = async {
            if let Some(source) = source {
                self.stores
                    .source
                    .import_project_exact(MiniAppSourceExactImportRequest {
                        scope: MiniAppSourceScope::new(
                            owner_user_id,
                            &miniapp_id,
                            &project_id,
                        )
                        .map_err(|error| store_error("Import Source scope", error))?,
                        display_name: display_name.clone(),
                        content_kind: if kind == MiniAppM1Kind::Service {
                            crate::MiniAppSourceContentKind::Service
                        } else {
                            crate::MiniAppSourceContentKind::UiOnly
                        },
                        dependency_lock: source.dependency_lock,
                        files: source
                            .files
                            .into_iter()
                            .map(|file| {
                                MiniAppSourceFileInput::new(
                                    file.normalized_relative_path,
                                    file.bytes,
                                )
                            })
                            .collect(),
                        expected_source_snapshot_digest: source
                            .source
                            .source_snapshot_digest,
                        expected_dependency_lock_digest: source
                            .source
                            .dependency_lock_digest,
                        build_generation: 1,
                        build_profile_version: source.source.build_profile_version,
                    })
                    .map_err(|error| store_error("Import Source", error))?;
            }
            let scope = MiniAppSourceScope::new(owner_user_id, &miniapp_id, &project_id)
                .map_err(|error| store_error("Import Release scope", error))?;
            let source_digest = begun
                .snapshot
                .project
                .source_head_digest
                .clone()
                .unwrap_or_else(|| imported.artifact.artifact_digest.as_ref().to_owned());
            let lock_digest = begun
                .snapshot
                .project
                .dependency_lock_digest
                .clone()
                .unwrap_or_else(|| {
                    imported
                        .artifact
                        .manifest
                        .payload
                        .dependency_lock_digest
                        .as_ref()
                        .to_owned()
                });
            let published = self
                .stores
                .release
                .publish(if kind == MiniAppM1Kind::Service {
                    MiniAppReleasePublishRequest::service(
                        scope,
                        source_digest.clone().into(),
                        lock_digest.clone().into(),
                        1,
                        imported.artifact,
                        imported.files,
                    )
                } else {
                    MiniAppReleasePublishRequest::ui_only(
                        scope,
                        source_digest.clone().into(),
                        lock_digest.clone().into(),
                        1,
                        imported.artifact,
                        imported.files,
                    )
                })
                .map_err(|error| store_error("Import Release", error))?;
            let artifact = published.stored.artifact;
            let release_id = Uuid::now_v7().to_string();
            let finished_at_ms = positive_now_ms().max(started_at_ms);
            let source_lineage = if begun.snapshot.project.source_state == "editable" {
                MiniAppSourceLineage::Managed {
                    project_id: project_id.clone().into(),
                    source_snapshot_digest: source_digest.clone().into(),
                    dependency_lock_digest: lock_digest.clone().into(),
                    build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
                    build_generation: 1,
                }
            } else {
                MiniAppSourceLineage::RuntimeOnly
            };
            let ready = MiniAppReadyRelease {
                miniapp_id: miniapp_id.clone().into(),
                release: MiniAppReleaseRef {
                    release_id: release_id.clone().into(),
                    artifact_id: artifact.artifact_id.clone(),
                    release_digest: artifact.artifact_digest.clone(),
                    manifest_digest: artifact.manifest.payload_digest.clone(),
                },
                origin_operation_id: operation_id.clone().into(),
                origin: MiniAppReadyOrigin::Import,
                source_lineage,
                matching_service_test_receipt: None,
                created_at_ms: finished_at_ms,
            };
            ready
                .validate_for_artifact(&artifact)
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
            self.repository
                .finish_import_ready(&FinishMiniAppM1ImportReadyParams {
                    owner_user_id: owner_user_id.to_owned(),
                    miniapp_id: miniapp_id.clone(),
                    project_id: project_id.clone(),
                    operation_id: operation_id.clone(),
                    expected_library_revision: begun.snapshot.library_revision,
                    expected_product_revision: begun.snapshot.product.product_revision,
                    expected_pointer_revision: begun.snapshot.product.pointer_revision,
                    expected_project_revision: begun.snapshot.project.project_revision,
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
                        miniapp_id: miniapp_id.clone(),
                        owner_user_id: owner_user_id.to_owned(),
                        artifact_id: artifact.artifact_id.as_ref().to_owned(),
                        artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
                        manifest_digest: artifact.manifest.payload_digest.as_ref().to_owned(),
                        release_digest: artifact.artifact_digest.as_ref().to_owned(),
                        origin_kind: "import".into(),
                        origin_operation_id: operation_id.clone(),
                        source_kind: if begun.snapshot.project.source_state == "editable" {
                            "managed".into()
                        } else {
                            "runtime_only".into()
                        },
                        project_id: (begun.snapshot.project.source_state == "editable")
                            .then_some(project_id.clone()),
                        source_snapshot_digest: (begun.snapshot.project.source_state
                            == "editable")
                            .then_some(source_digest),
                        dependency_lock_digest: (begun.snapshot.project.source_state
                            == "editable")
                            .then_some(lock_digest),
                        build_profile_version: (begun.snapshot.project.source_state
                            == "editable")
                            .then_some(MINIAPP_RELEASE_PROFILE_VERSION.into()),
                        build_generation: (begun.snapshot.project.source_state == "editable")
                            .then_some(1),
                        release_record_json: canonical_json_string(&ready)?,
                        created_at: finished_at_ms,
                    },
                    bounded_log_tail: vec!["MiniApp Import Ready committed".into()],
                    finished_at_ms,
                })
                .await
                .map_err(MiniAppM1ApplicationError::from)
        }
        .await;
        match result {
            Ok(snapshot) => self.workshop_projection(owner_user_id, &snapshot, None).await,
            Err(error) => {
                let _ = self
                    .repository
                    .fail_import(&FailMiniAppM1ImportParams {
                        owner_user_id: owner_user_id.to_owned(),
                        miniapp_id: miniapp_id.clone(),
                        project_id: project_id.clone(),
                        operation_id,
                        expected_product_revision: begun.snapshot.product.product_revision,
                        expected_pointer_revision: begun.snapshot.product.pointer_revision,
                        expected_project_revision: begun.snapshot.project.project_revision,
                        progress_percent: 0,
                        error_code: "miniapp_import_failed".into(),
                        bounded_log_tail: vec![bounded_log_line(&error.to_string())],
                        finished_at_ms: positive_now_ms().max(started_at_ms),
                    })
                    .await;
                let _ = self
                    .stores
                    .source
                    .purge_project(owner_user_id, &miniapp_id, &project_id);
                let _ = self
                    .stores
                    .release
                    .purge_project(owner_user_id, &miniapp_id, &project_id);
                Err(error)
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
        require_release_mutation(&snapshot)?;
        require_no_running_build(&*self.repository, owner_user_id, &request.miniapp_id).await?;
        validate_publish_request(&snapshot, &request)?;

        let ready = snapshot
            .ready_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Ready Release".to_owned()))?;
        let stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            ready,
        )?;
        if snapshot.product.kind == MiniAppM1Kind::UiOnly.as_str()
            && !stored.artifact.manifest.payload.is_ui_only()
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "UI-only MiniApp cannot publish a Service Release".to_owned(),
            ));
        }
        self.validate_service_publish_receipt(owner_user_id, &snapshot, &request)
            .await?;

        let target = release_contract_ref(ready);
        let target_catalog_digest = miniapp_catalog_digest(
            &request.miniapp_id,
            &target,
            &stored.artifact.manifest.payload.contributions,
        )?;
        let target_epoch = u64::try_from(snapshot.product.active_release_epoch)
            .map_err(|_| {
                MiniAppM1ApplicationError::Invalid(
                    "MiniApp active Release epoch is negative".to_owned(),
                )
            })?
            .checked_add(1)
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "MiniApp active Release epoch overflow".to_owned(),
                )
            })?;
        let (current_service, target_service) = self
            .prepare_service_cutover(owner_user_id, &snapshot, ready, target_epoch, true)
            .await?;
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
            .await;
        let committed = match committed {
            Ok(committed) => committed,
            Err(error) => {
                self.restore_service_after_failed_cutover(
                    owner_user_id,
                    &snapshot,
                    current_service,
                )
                    .await?;
                return Err(error.into());
            }
        };
        self.complete_service_cutover(&committed, target_service).await?;
        self.sync_catalog_publication(owner_user_id, &committed)
            .await?;
        self.workshop_projection(owner_user_id, &committed, None)
            .await
    }

    async fn validate_service_publish_receipt(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
        request: &PublishMiniAppRequestDto,
    ) -> Result<(), MiniAppM1ApplicationError> {
        if snapshot.product.kind != MiniAppM1Kind::Service.as_str() {
            return Ok(());
        }
        let Some(expected_receipt_id) =
            request.expected_service_test_receipt_id.as_deref()
        else {
            if request.acknowledge_test_warning {
                return Ok(());
            }
            return Err(MiniAppM1ApplicationError::Invalid(
                "Service Publish requires a current passed receipt or explicit Test warning acknowledgement"
                    .into(),
            ));
        };
        let row = self
            .repository
            .get_ready_service_test_receipt(
                owner_user_id,
                &snapshot.product.miniapp_id,
            )
            .await?
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Database(
                    nomifun_db::DbError::Conflict(
                        "Service Test receipt is stale for the current Ready state".into(),
                    ),
                )
            })?;
        if row.receipt_id != expected_receipt_id
            || row.release_id != request.ready_release_id
            || row.release_digest != request.expected_ready_release_digest
        {
            return Err(MiniAppM1ApplicationError::Database(
                nomifun_db::DbError::Conflict(
                    "Service Publish does not bind the current Ready Test receipt".into(),
                ),
            ));
        }
        let current_runtime = self
            .service_runtime()
            .await
            .current_runtime_fingerprint()
            .await
            .map_err(|error| {
                MiniAppM1ApplicationError::Runtime(format!(
                    "cannot resolve current Runtime for Service Publish: {error}"
                ))
            })?
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Database(
                    nomifun_db::DbError::Conflict(
                        "Service Test receipt is stale because no Runtime is selected".into(),
                    ),
                )
            })?;
        let runtime_digest = digest_payload(&current_runtime)
            .map_err(|error| MiniAppM1ApplicationError::Runtime(error.to_string()))?;
        if runtime_digest.as_ref() != row.runtime_fingerprint_digest {
            return Err(MiniAppM1ApplicationError::Database(
                nomifun_db::DbError::Conflict(
                    "Service Test receipt Runtime no longer matches the committed Runtime".into(),
                ),
            ));
        }
        let receipt: MiniAppServiceTestReceipt =
            serde_json::from_str(&row.receipt_json).map_err(|error| {
                MiniAppM1ApplicationError::Runtime(format!(
                    "stored Service Test receipt is invalid: {error}"
                ))
            })?;
        match receipt.outcome {
            MiniAppServiceTestOutcome::Passed => Ok(()),
            MiniAppServiceTestOutcome::Failed
            | MiniAppServiceTestOutcome::NeedsTestInput
                if request.acknowledge_test_warning =>
            {
                Ok(())
            }
            MiniAppServiceTestOutcome::Failed
            | MiniAppServiceTestOutcome::NeedsTestInput => {
                Err(MiniAppM1ApplicationError::Invalid(
                    "Service Publish requires explicit acknowledgement for a non-passed Test receipt"
                        .into(),
                ))
            }
        }
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
        require_release_mutation(&snapshot)?;
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
        self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            active,
        )?;
        let target = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            previous,
        )?;
        if snapshot.product.kind == MiniAppM1Kind::UiOnly.as_str()
            && !target.artifact.manifest.payload.is_ui_only()
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "UI-only MiniApp cannot roll back to a Service Release".to_owned(),
            ));
        }

        let rollback_target = release_contract_ref(previous);
        let target_catalog_digest = miniapp_catalog_digest(
            &request.miniapp_id,
            &rollback_target,
            &target.artifact.manifest.payload.contributions,
        )?;
        let target_epoch = u64::try_from(snapshot.product.active_release_epoch)
            .map_err(|_| {
                MiniAppM1ApplicationError::Invalid(
                    "MiniApp active Release epoch is negative".to_owned(),
                )
            })?
            .checked_add(1)
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "MiniApp active Release epoch overflow".to_owned(),
                )
            })?;
        let (current_service, target_service) = self
            .prepare_service_cutover(owner_user_id, &snapshot, previous, target_epoch, false)
            .await?;
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
            .await;
        let committed = match committed {
            Ok(committed) => committed,
            Err(error) => {
                self.restore_service_after_failed_cutover(
                    owner_user_id,
                    &snapshot,
                    current_service,
                )
                    .await?;
                return Err(error.into());
            }
        };
        self.complete_service_cutover(&committed, target_service).await?;
        self.sync_catalog_publication(owner_user_id, &committed)
            .await?;
        self.workshop_projection(owner_user_id, &committed, None)
            .await
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
        require_release_mutation(&snapshot)?;
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
        self.reconcile_service_runtime(owner_user_id, &committed)
            .await?;
        self.sync_catalog_publication(owner_user_id, &committed)
            .await?;
        self.workshop_projection(owner_user_id, &committed, None)
            .await
    }

    pub async fn trash(
        &self,
        owner_user_id: &str,
        request: TrashMiniAppRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if snapshot.product.product_revision
            != to_i64(request.expected_product_revision, "product revision")?
            || snapshot.product.pointer_revision
                != to_i64(request.expected_pointer_revision, "pointer revision")?
            || snapshot.product.active_release_digest.as_deref()
                != request.expected_active_release_digest.as_deref()
        {
            return Err(MiniAppM1ApplicationError::Database(
                nomifun_db::DbError::Conflict(
                    "MiniApp Trash request is stale against the exact Product pointers"
                        .to_owned(),
                ),
            ));
        }
        if !matches!(snapshot.product.lifecycle.as_str(), "enabled" | "disabled") {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "MiniApp cannot be trashed while lifecycle is {}",
                snapshot.product.lifecycle
            )));
        }
        let updated_at = positive_now_ms().max(snapshot.product.updated_at);
        let committed = self
            .repository
            .trash_cas(&TrashMiniAppM1Params {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                expected_product_revision: snapshot.product.product_revision,
                expected_pointer_revision: snapshot.product.pointer_revision,
                expected_active_release_digest: snapshot
                    .product
                    .active_release_digest
                    .clone(),
                updated_at,
            })
            .await?;
        if committed.product.kind == MiniAppM1Kind::Service.as_str() {
            self.service_runtime()
                .await
                .stop(&MiniAppId::from(request.miniapp_id.clone()))
                .await
                .map_err(|error| {
                    MiniAppM1ApplicationError::Invalid(format!(
                        "MiniApp was trashed but its Service Host could not stop: {error}"
                    ))
                })?;
        }
        self.sync_catalog_publication(owner_user_id, &committed)
            .await?;
        self.workshop_projection(owner_user_id, &committed, None)
            .await
    }

    pub async fn restore(
        &self,
        owner_user_id: &str,
        request: RestoreMiniAppRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        if request.expected_lifecycle != MiniAppLifecycleDto::Trashed {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Restore requires expected_lifecycle=trashed".into(),
            ));
        }
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        let committed = self
            .repository
            .restore_cas(&RestoreMiniAppM1Params {
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
                expected_lifecycle: "trashed".to_owned(),
                updated_at: positive_now_ms().max(snapshot.product.updated_at),
            })
            .await?;
        self.sync_catalog_publication(owner_user_id, &committed)
            .await?;
        self.workshop_projection(owner_user_id, &committed, None)
            .await
    }

    pub async fn delete(
        &self,
        owner_user_id: &str,
        request: DeleteMiniAppRequest,
    ) -> Result<MiniAppLibraryResponseDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        if request.expected_lifecycle != MiniAppLifecycleDto::Trashed {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp Delete requires expected_lifecycle=trashed".into(),
            ));
        }
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        let operation_id = Uuid::now_v7().to_string();
        let deleting = self
            .repository
            .begin_delete(&BeginMiniAppM1DeleteParams {
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
                expected_active_release_digest: request.expected_active_release_digest,
                operation_id: operation_id.clone(),
                started_at_ms: positive_now_ms().max(snapshot.product.updated_at),
            })
            .await?;
        self.sync_catalog_publication(owner_user_id, &deleting)
            .await?;
        self.run_delete_cleanup(
            owner_user_id,
            &deleting,
            &operation_id,
            1,
        )
        .await?;
        self.library(owner_user_id).await
    }

    pub async fn retry_delete(
        &self,
        owner_user_id: &str,
        request: RetryMiniAppDeleteRequest,
    ) -> Result<MiniAppLibraryResponseDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        validate_request_identity(&request.failed_operation_id, "failed_operation_id")?;
        let operation = self
            .repository
            .get_miniapp_operation(
                owner_user_id,
                &request.miniapp_id,
                &request.failed_operation_id,
            )
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        if operation.kind != "miniapp_permanent_delete"
            || operation.state != ProductOperationState::Failed.as_str()
            || operation_revision(&operation)
                != request.expected_operation_revision
        {
            return Err(operation_conflict(
                &request.failed_operation_id,
                &operation.state,
                "delete operation is not the expected failed revision",
            ));
        }
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        let operation_id = Uuid::now_v7().to_string();
        let restarted = self
            .repository
            .restart_delete(&RestartMiniAppM1DeleteParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: request.miniapp_id.clone(),
                expected_failed_operation_id: request.failed_operation_id,
                new_operation_id: operation_id.clone(),
                started_at_ms: positive_now_ms().max(snapshot.product.updated_at),
            })
            .await?;
        self.run_delete_cleanup(owner_user_id, &restarted, &operation_id, 1)
            .await?;
        self.library(owner_user_id).await
    }

    async fn run_delete_cleanup(
        &self,
        owner_user_id: &str,
        snapshot: &MiniAppM1Snapshot,
        operation_id: &str,
        expected_operation_revision: i64,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let cleanup = async {
            let miniapp_id = MiniAppId::from(snapshot.product.miniapp_id.clone());
            let runtime = self.service_runtime().await;
            runtime
                .stop(&miniapp_id)
                .await
                .map_err(|error| {
                    MiniAppM1ApplicationError::Runtime(format!(
                        "Service stop during permanent delete failed: {error}"
                    ))
                })?;
            runtime
                .purge_storage(owner_user_id, &miniapp_id)
                .await
                .map_err(|error| {
                    MiniAppM1ApplicationError::Runtime(format!(
                        "managed storage purge failed: {error}"
                    ))
                })?;
            self.stores
                .source
                .purge_project(
                    owner_user_id,
                    &snapshot.product.miniapp_id,
                    &snapshot.project.project_id,
                )
                .map_err(|error| {
                    MiniAppM1ApplicationError::Runtime(format!(
                        "Source purge failed: {error}"
                    ))
                })?;
            self.stores
                .release
                .purge_project(
                    owner_user_id,
                    &snapshot.product.miniapp_id,
                    &snapshot.project.project_id,
                )
                .map_err(|error| {
                    MiniAppM1ApplicationError::Runtime(format!(
                        "Release purge failed: {error}"
                    ))
                })?;
            Ok::<(), MiniAppM1ApplicationError>(())
        }
        .await;
        if let Err(error) = cleanup {
            let fail = self
                .repository
                .fail_delete(&FailMiniAppM1DeleteParams {
                    owner_user_id: owner_user_id.to_owned(),
                    miniapp_id: snapshot.product.miniapp_id.clone(),
                    operation_id: operation_id.to_owned(),
                    expected_operation_revision,
                    error_code: "miniapp_delete_cleanup_failed".to_owned(),
                    updated_at: positive_now_ms(),
                })
                .await;
            return match fail {
                Ok(_) => Err(error),
                Err(record_error) => Err(MiniAppM1ApplicationError::Runtime(format!(
                    "MiniApp Delete cleanup failed ({error}); recording its durable failure failed: {record_error}"
                ))),
            };
        }
        self.repository
            .finalize_delete(&FinalizeMiniAppM1DeleteParams {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: snapshot.product.miniapp_id.clone(),
                operation_id: operation_id.to_owned(),
                expected_operation_revision,
                finished_at_ms: positive_now_ms(),
            })
            .await?;
        Ok(())
    }

    pub async fn set_service_running(
        &self,
        owner_user_id: &str,
        request: SetMiniAppServiceRunningRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        validate_service_runtime_request(
            &snapshot,
            request.expected_product_revision,
            request.expected_pointer_revision,
            request.expected_active_release_epoch,
            &request.expected_active_release_digest,
        )?;
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "Service Start/Stop requires an Active Release".to_owned(),
                )
            })?;
        if request.running {
            let spec = self
                .resolve_service_spec(
                    &snapshot,
                    active,
                    request.expected_active_release_epoch,
                    owner_user_id,
                )
                .await?
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "Active Release has no Service descriptor".to_owned(),
                    )
                })?;
            self.service_runtime()
                .await
                .start(spec)
                .await
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        } else {
            self.service_runtime()
                .await
                .stop(&MiniAppId::from(request.miniapp_id.clone()))
                .await
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        }
        self.workshop_projection(owner_user_id, &snapshot, None)
            .await
    }

    pub async fn retry_service(
        &self,
        owner_user_id: &str,
        request: RetryMiniAppServiceRequest,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        validate_request_identity(&request.miniapp_id, "miniapp_id")?;
        let snapshot = self
            .repository
            .get(owner_user_id, &request.miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        validate_service_runtime_request(
            &snapshot,
            request.expected_product_revision,
            request.expected_pointer_revision,
            request.expected_active_release_epoch,
            &request.expected_active_release_digest,
        )?;
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "Service Retry requires an Active Release".to_owned(),
                )
            })?;
        let spec = self
            .resolve_service_spec(
                &snapshot,
                active,
                request.expected_active_release_epoch,
                owner_user_id,
            )
            .await?
            .ok_or_else(|| {
                MiniAppM1ApplicationError::Invalid(
                    "Active Release has no Service descriptor".to_owned(),
                )
            })?;
        self.service_runtime()
            .await
            .start(spec)
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        self.workshop_projection(owner_user_id, &snapshot, None)
            .await
    }

    pub async fn shutdown_service_runtime(
        &self,
        owner_user_id: &str,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let library = self.repository.library(owner_user_id).await?;
        let runtime = self.service_runtime().await;
        for product in library.products {
            if product.kind == MiniAppM1Kind::Service.as_str() {
                runtime
                    .stop(&MiniAppId::from(product.miniapp_id))
                    .await
                    .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
            }
        }
        Ok(())
    }

    pub async fn validate_service_runtime_candidate(
        &self,
        owner_user_id: &str,
        candidate: &ResolvedNodeRuntime,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let library = self.repository.library(owner_user_id).await?;
        let mut expected = library
            .products
            .iter()
            .filter(|product| {
                product.kind == MiniAppM1Kind::Service.as_str()
                    && product.lifecycle == "enabled"
            })
            .map(|product| MiniAppId::from(product.miniapp_id.clone()))
            .collect::<Vec<_>>();
        expected.sort();
        if expected.is_empty() {
            return Ok(());
        }
        let mut observed = self
            .service_runtime()
            .await
            .validate_candidate(candidate)
            .await
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        observed.sort();
        if observed != expected {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "MiniApp Service Runtime candidate identities {observed:?} do not match enabled Services {expected:?}"
            )));
        }
        Ok(())
    }

    pub async fn reconcile_all_service_runtime(
        &self,
        owner_user_id: &str,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let library = self.repository.library(owner_user_id).await?;
        for product in library.products {
            let Some(snapshot) = self
                .repository
                .get(owner_user_id, &product.miniapp_id)
                .await?
            else {
                continue;
            };
            if let Err(error) = self
                .reconcile_service_runtime(owner_user_id, &snapshot)
                .await
            {
                tracing::warn!(
                    miniapp_id = %product.miniapp_id,
                    error = %error,
                    "MiniApp Service startup reconciliation failed; keeping the persisted Product authoritative"
                );
            }
        }
        Ok(())
    }

    pub async fn reconcile_pending_deletions(
        &self,
        owner_user_id: &str,
    ) -> Result<(), MiniAppM1ApplicationError> {
        let library = self.repository.library(owner_user_id).await?;
        let mut failures = Vec::new();
        for product in library
            .products
            .into_iter()
            .filter(|product| product.lifecycle == "deleting")
        {
            let result = async {
                let snapshot = self
                    .repository
                    .get(owner_user_id, &product.miniapp_id)
                    .await?
                    .ok_or(MiniAppM1ApplicationError::NotFound)?;
                let mut operations = self
                    .repository
                    .list_miniapp_operations(owner_user_id, &product.miniapp_id)
                    .await?
                    .into_iter()
                    .filter(|operation| operation.kind == "miniapp_permanent_delete")
                    .collect::<Vec<_>>();
                operations.sort_by(|left, right| {
                    left.started_at_ms
                        .cmp(&right.started_at_ms)
                        .then_with(|| left.operation_id.cmp(&right.operation_id))
                });
                let operation = operations.pop().ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(format!(
                        "deleting MiniApp {} has no durable Delete operation",
                        product.miniapp_id
                    ))
                })?;
                match operation.state.as_str() {
                    "running" => {
                        self.run_delete_cleanup(
                            owner_user_id,
                            &snapshot,
                            &operation.operation_id,
                            1,
                        )
                        .await
                    }
                    "failed" => {
                        let operation_id = Uuid::now_v7().to_string();
                        let restarted = self
                            .repository
                            .restart_delete(&RestartMiniAppM1DeleteParams {
                                owner_user_id: owner_user_id.to_owned(),
                                miniapp_id: product.miniapp_id.clone(),
                                expected_failed_operation_id: operation.operation_id,
                                new_operation_id: operation_id.clone(),
                                started_at_ms: positive_now_ms()
                                    .max(snapshot.product.updated_at),
                            })
                            .await?;
                        self.run_delete_cleanup(
                            owner_user_id,
                            &restarted,
                            &operation_id,
                            1,
                        )
                        .await
                    }
                    state => Err(MiniAppM1ApplicationError::Invalid(format!(
                        "deleting MiniApp {} references terminal operation state {state}",
                        product.miniapp_id
                    ))),
                }
            }
            .await;
            if let Err(error) = result {
                tracing::warn!(
                    miniapp_id = %product.miniapp_id,
                    error = %error,
                    "MiniApp permanent-delete startup reconciliation failed; durable intent remains"
                );
                failures.push(format!("{}: {error}", product.miniapp_id));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(MiniAppM1ApplicationError::Invalid(format!(
                "MiniApp deletion reconciliation left durable failures: {}",
                failures.join("; ")
            )))
        }
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
        self.workshop_projection(owner_user_id, &committed, active_operation)
            .await
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
        let current_stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            active,
        )?;
        let target_stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            ready,
        )?;
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
        let catalog_digest = miniapp_catalog_digest(
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
        let committed = self
            .repository
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
            ?;
        self.sync_catalog_publication(owner_user_id, &committed)
            .await?;
        Ok(committed)
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
        let active = snapshot
            .active_release
            .as_ref()
            .ok_or_else(|| MiniAppM1ApplicationError::Invalid("no Active Release".to_owned()))?;
        let stored = self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            active,
        )?;
        if snapshot.product.kind == MiniAppM1Kind::Service.as_str() {
            let spec = self
                .resolve_service_spec(
                    &snapshot,
                    active,
                    positive_u64(
                        snapshot.product.active_release_epoch,
                        "MiniApp active release epoch",
                    )?,
                    owner_user_id,
                )
                .await?
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "Service Active Release has no Service descriptor".to_owned(),
                    )
                })?;
            self.service_runtime()
                .await
                .bind_active(spec, true)
                .await
                .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        }
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
            kind: kind_dto(&snapshot.product.kind)?,
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
        let stored = self.load_verified_release(
            &session.owner_user_id,
            &snapshot.project.project_id,
            active,
        )?;
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
        self.load_verified_release(
            owner_user_id,
            &snapshot.project.project_id,
            active,
        )?;
        let pointer = pointer_state_from_snapshot(&snapshot)?;
        let service_spec = if snapshot.product.kind == MiniAppM1Kind::Service.as_str() {
            Some(
                self.resolve_service_spec(
                    &snapshot,
                    active,
                    active_release_epoch,
                    owner_user_id,
                )
                .await?
                .ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "Service Active Release has no Service descriptor".to_owned(),
                    )
                })?,
            )
        } else {
            None
        };
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
            service_run_key: service_spec
                .as_ref()
                .map(|spec| spec.service_run_key.clone()),
        };
        request
            .validate_for(&session, &pointer)
            .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        let call_id = request.call_id.clone();
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
            MiniAppBridgeTarget::Service { method, payload } => {
                let spec = service_spec.ok_or_else(|| {
                    MiniAppM1ApplicationError::Invalid(
                        "MiniApp has no Active Service specification".to_owned(),
                    )
                })?;
                self.service_runtime()
                    .await
                    .invoke(
                        &spec,
                        call_id,
                        method,
                        payload,
                        MiniAppCallCancellation::default(),
                        positive_now_ms(),
                    )
                    .await
                    .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))
            }
        }
    }

    pub async fn workshop(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
        self.reconcile_source_mutations().await?;
        let snapshot = self
            .repository
            .get(owner_user_id, miniapp_id)
            .await?
            .ok_or(MiniAppM1ApplicationError::NotFound)?;
        let active_operation = self
            .latest_miniapp_operation(owner_user_id, miniapp_id)
            .await?;
        self.workshop_projection(owner_user_id, &snapshot, active_operation)
            .await
    }

    async fn latest_miniapp_operation(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<Option<DurableOperationSummaryDto>, MiniAppM1ApplicationError> {
        let operations = self
            .repository
            .list_miniapp_operations(owner_user_id, miniapp_id)
            .await?;
        let mut active = operations
            .into_iter()
            .filter(|operation| {
                operation.state == ProductOperationState::Running.as_str()
                    || (operation.kind == "miniapp_permanent_delete"
                        && operation.state == ProductOperationState::Failed.as_str())
            })
            .collect::<Vec<_>>();
        active.sort_by(|left, right| {
            left.started_at_ms
                .cmp(&right.started_at_ms)
                .then_with(|| left.operation_id.cmp(&right.operation_id))
        });
        let Some(operation) = active.pop() else {
            return Ok(None);
        };
        if active.iter().any(|other| {
            other.state == ProductOperationState::Running.as_str()
                && operation.state == ProductOperationState::Running.as_str()
        }) {
            return Err(MiniAppM1ApplicationError::Invalid(
                "MiniApp has more than one active owner operation".into(),
            ));
        }
        operation_summary(&operation).map(Some)
    }

    fn load_verified_release(
        &self,
        owner_user_id: &str,
        storage_project_id: &str,
        release: &MiniAppReleaseRow,
    ) -> Result<MiniAppStoredRelease, MiniAppM1ApplicationError> {
        let project_id = release.project_id.as_deref().unwrap_or(storage_project_id);
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
        if record.miniapp_id.as_ref() != release.miniapp_id
            || record.release.release_id.as_ref() != release.release_id
            || record.release.artifact_id.as_ref() != release.artifact_id
            || record.release.release_digest.as_ref() != release.release_digest
            || record.release.manifest_digest.as_ref() != release.manifest_digest
            || record.origin
                != if release.origin_kind == "build" {
                    MiniAppReadyOrigin::Build
                } else {
                    MiniAppReadyOrigin::Import
                }
            || record.origin_operation_id.as_ref() != release.origin_operation_id
            || record.created_at_ms != release.created_at
        {
            return Err(MiniAppM1ApplicationError::Invalid(
                "database Release row does not match its canonical Release record".to_owned(),
            ));
        }
        match (&record.source_lineage, release.source_kind.as_str()) {
            (
                MiniAppSourceLineage::Managed {
                    project_id: record_project_id,
                    source_snapshot_digest,
                    dependency_lock_digest,
                    build_profile_version,
                    build_generation,
                },
                "managed",
            ) if record_project_id.as_ref() == project_id
                && source_snapshot_digest.as_ref()
                    == release.source_snapshot_digest.as_deref().unwrap_or_default()
                && dependency_lock_digest.as_ref()
                    == release.dependency_lock_digest.as_deref().unwrap_or_default()
                && build_profile_version.as_ref()
                    == release.build_profile_version.as_deref().unwrap_or_default()
                && i64::try_from(*build_generation).ok() == release.build_generation => {}
            (MiniAppSourceLineage::RuntimeOnly, "runtime_only")
                if release.project_id.is_none()
                    && release.source_snapshot_digest.is_none()
                    && release.dependency_lock_digest.is_none()
                    && release.build_profile_version.is_none()
                    && release.build_generation.is_none() => {}
            _ => {
                return Err(MiniAppM1ApplicationError::Invalid(
                    "database Release lineage does not match its canonical Release record".into(),
                ));
            }
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
    service_module_path: Option<PathBuf>,
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
            "M1-1-01 accepts only the empty MiniApp dependency lock".to_owned(),
        ));
    }

    let is_service = snapshot.product.kind == MiniAppM1Kind::Service.as_str();
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
            "MiniApp Source must contain ui/index.html".to_owned(),
        )
    })?;
    let service_main_mjs = if is_service {
        Some(source_files.remove("service/main.mjs").ok_or_else(|| {
            MiniAppM1ApplicationError::Invalid(
                "Service Source must contain service/main.mjs".to_owned(),
            )
        })?)
    } else {
        None
    };
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
    let service_lifecycle = service_lifecycle_from_request(request.service_lifecycle)?;
    let artifact_input = MiniAppStaticBundleInput {
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
            service: service_main_mjs.clone().map(|main_mjs| MiniAppStaticServiceInput {
                main_mjs,
                lifecycle: service_lifecycle,
                uses_files: false,
                uses_private_database: false,
                service_contract_digest: digest_payload(&MINIAPP_SERVICE_HOST_PROTOCOL_VERSION)
                    .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))
                    .unwrap_or_else(|_| digest_bytes(b"miniapp-service-contract")),
                runtime_requirements_digest: digest_bytes(b"miniapp-service-runtime"),
            }),
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
        };
    let artifact = MiniAppStaticBundleBuilder::new()
        .build(artifact_input)
        .map_err(|error| {
            MiniAppM1ApplicationError::Invalid(format!("MiniApp Build failed: {error}"))
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
        .publish(if is_service {
            MiniAppReleasePublishRequest::service(
                scope,
                source.source_snapshot_digest.clone(),
                source.dependency_lock_digest.clone(),
                request.expected_build_generation,
                artifact,
                file_bytes,
            )
        } else {
            MiniAppReleasePublishRequest::ui_only(
                scope,
                source.source_snapshot_digest.clone(),
                source.dependency_lock_digest.clone(),
                request.expected_build_generation,
                artifact,
                file_bytes,
            )
        })
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
        service_module_path: is_service.then(|| {
            published
                .stored
                .artifact_root
                .join("files")
                .join("service")
                .join("main.mjs")
        }),
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
    if snapshot.product.kind != MiniAppM1Kind::UiOnly.as_str()
        && snapshot.product.kind != MiniAppM1Kind::Service.as_str()
    {
        return Err(MiniAppM1ApplicationError::Invalid(
            "MiniApp product kind is invalid".to_owned(),
        ));
    }
    if snapshot.product.kind == MiniAppM1Kind::UiOnly.as_str()
        && request.service_lifecycle.is_some()
    {
        return Err(MiniAppM1ApplicationError::Invalid(
            "UI-only Build cannot declare a Service lifecycle".to_owned(),
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

fn require_source_matches_project(
    source: &MiniAppSourceSnapshot,
    snapshot: &MiniAppM1Snapshot,
) -> Result<(), MiniAppM1ApplicationError> {
    if snapshot.project.source_head_digest.as_deref()
        != Some(source.source_snapshot_digest.as_ref())
        || snapshot.project.dependency_lock_digest.as_deref()
            != Some(source.dependency_lock_digest.as_ref())
        || snapshot.project.build_generation
            != to_i64(source.project.build_generation, "Source build generation")?
        || snapshot.project.managed_source_path.as_deref()
            != Some(source.project.managed_relative_path.as_str())
        || source.project.build_profile != JavaScriptBuildProfile::MiniAppReleaseV1
        || source.project.build_profile_version.as_ref()
            != MINIAPP_RELEASE_PROFILE_VERSION
    {
        return Err(MiniAppM1ApplicationError::Invalid(
            "Source Store head does not match the exact DB Project lineage".into(),
        ));
    }
    Ok(())
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
    require_release_mutation(snapshot)?;
    if snapshot.product.kind != MiniAppM1Kind::UiOnly.as_str() {
        return Err(MiniAppM1ApplicationError::Invalid(
            "auto Publish mode is available only for UI-only MiniApps".to_owned(),
        ));
    }
    Ok(())
}

fn require_release_mutation(
    snapshot: &MiniAppM1Snapshot,
) -> Result<(), MiniAppM1ApplicationError> {
    if snapshot.product.kind != MiniAppM1Kind::UiOnly.as_str()
        && snapshot.product.kind != MiniAppM1Kind::Service.as_str()
    {
        return Err(MiniAppM1ApplicationError::Invalid(
            "MiniApp product kind is invalid".to_owned(),
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
    let is_service = snapshot.product.kind == MiniAppM1Kind::Service.as_str();
    if !is_service
        && (request.expected_service_test_receipt_id.is_some()
            || request.acknowledge_test_warning)
    {
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

fn validate_service_runtime_request(
    snapshot: &MiniAppM1Snapshot,
    expected_product_revision: u64,
    expected_pointer_revision: u64,
    expected_active_release_epoch: u64,
    expected_active_release_digest: &str,
) -> Result<(), MiniAppM1ApplicationError> {
    if snapshot.product.kind != MiniAppM1Kind::Service.as_str() {
        return Err(MiniAppM1ApplicationError::Invalid(
            "Service lifecycle operations require a Service MiniApp".to_owned(),
        ));
    }
    validate_digest_string(expected_active_release_digest, "expected Active Release digest")?;
    if snapshot.product.product_revision
        != to_i64(expected_product_revision, "product revision")?
        || snapshot.product.pointer_revision
            != to_i64(expected_pointer_revision, "pointer revision")?
        || snapshot.product.active_release_epoch
            != to_i64(expected_active_release_epoch, "active release epoch")?
        || snapshot.product.active_release_digest.as_deref()
            != Some(expected_active_release_digest)
    {
        return Err(MiniAppM1ApplicationError::Database(
            nomifun_db::DbError::Conflict(
                "MiniApp Service lifecycle request is stale against the exact Active Release"
                    .to_owned(),
            ),
        ));
    }
    if snapshot.product.lifecycle != "enabled" {
        return Err(MiniAppM1ApplicationError::Invalid(
            "MiniApp Service must be enabled before it can start or retry".to_owned(),
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

#[async_trait]
impl MiniAppAgentCapabilityPort for MiniAppM1ApplicationService {
    async fn invoke_agent_capability(
        &self,
        request: MiniAppAgentCapabilityInvocation,
    ) -> Result<StrictJsonValue, MiniAppM1ApplicationError> {
        self.invoke_agent_capability_inner(request).await
    }
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

pub fn miniapp_catalog_digest(
    miniapp_id: &str,
    active_release: &MiniAppReleaseRef,
    contributions: &PackageContributions,
) -> Result<DigestHex, MiniAppM1ApplicationError> {
    let publication = build_miniapp_catalog_publication(
        MiniAppId::from(miniapp_id),
        active_release.clone(),
        contributions,
    )?;
    Ok(publication.catalog_digest)
}

fn build_miniapp_catalog_publication(
    miniapp_id: MiniAppId,
    active_release: MiniAppReleaseRef,
    contributions: &PackageContributions,
) -> Result<MiniAppCapabilityCatalogPublication, MiniAppM1ApplicationError> {
    let mut capabilities = Vec::with_capacity(contributions.capabilities.len());
    for manifest in &contributions.capabilities {
        let consumers = manifest
            .supported_consumers()
            .map_err(MiniAppM1ApplicationError::Invalid)?;
        let service_dispatch_available =
            consumers.contains(&CapabilityConsumer::MiniAppService);
        let availability = consumers
            .iter()
            .copied()
            .map(|consumer| {
                (
                    consumer,
                    if service_dispatch_available
                        && matches!(
                            consumer,
                            CapabilityConsumer::Agent
                                | CapabilityConsumer::MiniAppService
                        )
                    {
                        CatalogAvailability::Active
                    } else {
                        CatalogAvailability::Unavailable {
                            reason: format!(
                                "CAPABILITY_MINIAPP_{}_DISPATCH_UNAVAILABLE",
                                consumer.as_str().to_ascii_uppercase()
                            ),
                        }
                    },
                )
            })
            .collect();
        let entry = CapabilityCatalogMaterializer::materialize(
            CapabilityCatalogMaterialization {
                manifest: manifest.clone(),
                provenance: CapabilityProvenance {
                    owner: CapabilityOwner::Package {
                        package: manifest.package.clone(),
                    },
                    source_kind: ContributionSourceKind::MiniAppActiveRelease,
                    source_identity: format!("miniapp:{}", miniapp_id.as_ref()).into(),
                    mount_id: None,
                    miniapp_id: Some(miniapp_id.clone()),
                    mcp_binding_id: None,
                    artifact_digest: Some(active_release.release_digest.clone()),
                },
                release_state: CapabilityReleaseState::PublishedActive,
                availability,
            },
        )
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
        capabilities.push(CapabilityCatalogPublication {
            manifest: manifest.clone(),
            entry,
        });
    }
    let mut publication = MiniAppCapabilityCatalogPublication {
        miniapp_id,
        active_release,
        active_release_epoch: 1,
        catalog_digest: DigestHex::from("0".repeat(64)),
        capabilities,
    };
    publication.catalog_digest = publication
        .computed_catalog_digest()
        .map_err(|error| MiniAppM1ApplicationError::Invalid(error.to_string()))?;
    Ok(publication)
}

fn miniapp_capability_catalog_item(
    publication: &CapabilityCatalogPublication,
) -> Result<CapabilityCatalogItemDto, MiniAppM1ApplicationError> {
    let manifest = &publication.manifest;
    let unavailable_code = match publication
        .entry
        .availability_for(nomifun_agent_contracts::CapabilityConsumer::Agent)
    {
        Some(CatalogAvailability::Active) => None,
        Some(CatalogAvailability::Unavailable { reason })
        | Some(CatalogAvailability::Disabled { reason }) => Some(reason.clone()),
        Some(CatalogAvailability::NeedsRuntime { .. }) => {
            Some("CAPABILITY_NEEDS_RUNTIME".to_owned())
        }
        Some(CatalogAvailability::ContractMismatch { .. }) => {
            Some("CAPABILITY_CONTRACT_MISMATCH".to_owned())
        }
        None => Some("CAPABILITY_CONSUMER_UNSUPPORTED".to_owned()),
    };
    Ok(CapabilityCatalogItemDto {
        capability: nomifun_api_types::ExactCatalogRefDto {
            id: manifest.id.as_ref().to_owned(),
            version: manifest.version.as_ref().to_owned(),
        },
        kind: match manifest.kind {
            nomifun_agent_contracts::CapabilityKind::Tool => "tool",
            nomifun_agent_contracts::CapabilityKind::ContextContributor => {
                "context_contributor"
            }
            nomifun_agent_contracts::CapabilityKind::ResourceProvider => {
                "resource_provider"
            }
            nomifun_agent_contracts::CapabilityKind::EventSource => "event_source",
            nomifun_agent_contracts::CapabilityKind::EventConsumer => "event_consumer",
            nomifun_agent_contracts::CapabilityKind::TurnMiddleware => "turn_middleware",
            nomifun_agent_contracts::CapabilityKind::Transport => "transport",
            nomifun_agent_contracts::CapabilityKind::Scheduler => "scheduler",
            nomifun_agent_contracts::CapabilityKind::BackgroundService => {
                "background_service"
            }
            nomifun_agent_contracts::CapabilityKind::UiContribution => "ui_contribution",
        }
        .to_owned(),
        display_name: manifest.display.name.clone(),
        description: manifest.display.description.clone(),
        source_package: nomifun_api_types::ExactCatalogRefDto {
            id: manifest.package.id.as_ref().to_owned(),
            version: manifest.package.version.as_ref().to_owned(),
        },
        source_kind: "miniapp_active_release".to_owned(),
        materialization_state: if unavailable_code.is_some() {
            CatalogMaterializationStateDto::Unavailable
        } else {
            CatalogMaterializationStateDto::Materialized
        },
        unavailable_code,
        supported_surfaces: publication.entry.host_surfaces.clone(),
        required_runtime_features: manifest
            .requires_runtime_features
            .iter()
            .map(|feature| feature.id.as_ref().to_owned())
            .collect(),
        required_resource_kinds: manifest
            .contributions
            .resource_kinds
            .iter()
            .map(|kind| kind.as_ref().to_owned())
            .collect(),
        required_capabilities: manifest
            .requires
            .iter()
            .map(|reference| nomifun_api_types::ExactCatalogRefDto {
                id: reference.id.as_ref().to_owned(),
                version: reference.version.as_ref().to_owned(),
            })
            .collect(),
        conflicting_capabilities: manifest
            .conflicts
            .iter()
            .map(|conflict| nomifun_api_types::ExactCatalogRefDto {
                id: conflict.capability.id.as_ref().to_owned(),
                version: conflict.capability.version.as_ref().to_owned(),
            })
            .collect(),
        action_count: manifest.contributions.actions.len() as u32,
        context_contributor_count: manifest
            .contributions
            .context_schema_refs
            .len() as u32,
    })
}

fn validate_backup_credential_slot_union(
    credential_slots: &[CredentialSlotDeclaration],
    releases: &BTreeMap<String, MiniAppBackupRelease>,
) -> Result<(), MiniAppM1ApplicationError> {
    let declared = credential_slots
        .iter()
        .map(|slot| (slot.slot_key.as_ref().to_owned(), slot))
        .collect::<BTreeMap<_, _>>();
    if declared.len() != credential_slots.len() {
        return Err(MiniAppM1ApplicationError::Invalid(
            "Whole-App Backup credential slots contain duplicates".into(),
        ));
    }
    let mut observed = BTreeMap::<String, CredentialSlotDeclaration>::new();
    for release in releases.values() {
        for slot in &release.artifact.manifest.payload.credential_slots {
            match observed.get(slot.slot_key.as_ref()) {
                Some(existing) if existing != slot => {
                    return Err(MiniAppM1ApplicationError::Invalid(format!(
                        "Whole-App Backup credential slot {} changes across Releases",
                        slot.slot_key.as_ref()
                    )));
                }
                Some(_) => {}
                None => {
                    observed.insert(slot.slot_key.as_ref().to_owned(), slot.clone());
                }
            }
        }
    }
    if observed != declared
        .into_iter()
        .map(|(key, slot)| (key, slot.clone()))
        .collect::<BTreeMap<_, _>>()
    {
        return Err(MiniAppM1ApplicationError::Invalid(
            "Whole-App Backup credential slots do not match the retained Release union".into(),
        ));
    }
    Ok(())
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

fn summary_from_snapshot_with_observation(
    snapshot: &MiniAppM1Snapshot,
    observation: Option<&MiniAppServiceObservation>,
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
    if let Some(observation) = observation {
        summary.service_health = service_health_dto_from_observation(
            &snapshot.product.kind,
            observation,
        )?;
    }
    Ok(summary)
}

fn workshop_from_snapshot_with_observation(
    snapshot: &MiniAppM1Snapshot,
    active_operation: Option<DurableOperationSummaryDto>,
    observation: Option<&MiniAppServiceObservation>,
) -> Result<MiniAppWorkshopDto, MiniAppM1ApplicationError> {
    let product = &snapshot.product;
    let project = &snapshot.project;
    let summary = summary_from_snapshot_with_observation(snapshot, observation)?;
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
        service_lifecycle: None,
        active_service: None,
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
    if operation.owner_kind != "miniapp" {
        return Err(MiniAppM1ApplicationError::Invalid(
            "MiniApp operation is not owner-scoped".to_owned(),
        ));
    }
    let kind = match operation.kind.as_str() {
        "build" => DurableOperationKindDto::Build,
        "import" => DurableOperationKindDto::Import,
        "export" => DurableOperationKindDto::Export,
        "miniapp_permanent_delete" => DurableOperationKindDto::MiniappPermanentDelete,
        value => {
            return Err(MiniAppM1ApplicationError::Invalid(format!(
                "unknown MiniApp operation kind {value}"
            )));
        }
    };
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
                    "MiniApp operation progress is outside u8 range".to_owned(),
                )
            })
        })
        .transpose()?;
    Ok(DurableOperationSummaryDto {
        operation_id: operation.operation_id.clone(),
        operation_revision: operation_revision(operation),
        kind,
        owner: DurableOperationOwnerDto::Miniapp {
            miniapp_id: operation.owner_id.clone(),
        },
        state,
        cancelable: matches!(
            kind,
            DurableOperationKindDto::Build
                | DurableOperationKindDto::Import
                | DurableOperationKindDto::Export
        ) && operation.state == ProductOperationState::Running.as_str(),
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

fn service_lifecycle_from_request(
    value: Option<MiniAppServiceLifecycleDto>,
) -> Result<MiniAppServiceLifecycle, MiniAppM1ApplicationError> {
    match value.unwrap_or(MiniAppServiceLifecycleDto::OnDemand) {
        MiniAppServiceLifecycleDto::OnDemand => Ok(MiniAppServiceLifecycle::OnDemand),
        MiniAppServiceLifecycleDto::Continuous => Ok(MiniAppServiceLifecycle::Continuous),
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
        MiniAppM1ApplicationError::Runtime(_) => "MINIAPP_RUNTIME_ERROR",
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

fn bounded_error_code(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .take(128)
        .collect::<String>()
        .to_ascii_uppercase()
}

fn service_descriptor_dto(
    service: &nomifun_agent_contracts::MiniAppServiceReleaseDescriptor,
) -> MiniAppServiceDescriptorDto {
    MiniAppServiceDescriptorDto {
        lifecycle: match service.lifecycle {
            MiniAppServiceLifecycle::OnDemand => MiniAppServiceLifecycleDto::OnDemand,
            MiniAppServiceLifecycle::Continuous => MiniAppServiceLifecycleDto::Continuous,
        },
        uses_files: service.uses_files,
        uses_private_database: service.uses_private_database,
        service_contract_digest: service.service_contract_digest.as_ref().to_owned(),
    }
}

fn service_health_dto_from_observation(
    kind: &str,
    observation: &MiniAppServiceObservation,
) -> Result<MiniAppServiceHealthDto, MiniAppM1ApplicationError> {
    if kind != MiniAppM1Kind::Service.as_str() {
        return Ok(MiniAppServiceHealthDto::NotApplicable);
    }
    Ok(match observation {
        MiniAppServiceObservation::Stopped => MiniAppServiceHealthDto::Stopped,
        MiniAppServiceObservation::Starting { release } => {
            MiniAppServiceHealthDto::Starting {
                release_id: release.release_id.as_ref().to_owned(),
                expected_release_digest: release.release_digest.as_ref().to_owned(),
            }
        }
        MiniAppServiceObservation::Ready {
            release,
            started_at_ms,
        } => MiniAppServiceHealthDto::Ready {
            release_id: release.release_id.as_ref().to_owned(),
            expected_release_digest: release.release_digest.as_ref().to_owned(),
            started_at_ms: *started_at_ms,
        },
        MiniAppServiceObservation::Failed {
            release,
            error_code,
        } => MiniAppServiceHealthDto::Failed {
            release_id: release.release_id.as_ref().to_owned(),
            expected_release_digest: release.release_digest.as_ref().to_owned(),
            error_code: error_code.clone(),
        },
    })
}

fn default_service_module() -> Vec<u8> {
    br#"export async function start(context) {
  return {
    async invoke({ method, payload }) {
      if (method === "echo") return payload;
      throw new Error(`Unknown MiniApp Service method: ${method}`);
    },
    async dispose() {},
  };
}
"#
    .to_vec()
}
