use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    CredentialId, DigestHex, MiniAppDeletingIntent, MiniAppId, MiniAppProductLifecycleRecord,
    MiniAppProductLifecycleState, MiniAppProjectId, MiniAppReadyRelease,
    MiniAppReleaseArtifactV1, MiniAppReleasePointerState, MiniAppReleaseRef,
    MiniAppServiceStorageDescriptor, MiniAppUiOnlyAutoPublishAuthorization, OperationId,
    StrictJsonValue, digest_payload,
};
use serde::{Deserialize, Serialize};

use crate::{
    DurableMiniAppOperation, MiniAppOperationKind, MiniAppOperationState, MiniAppPlatformError,
    MiniAppPlatformResult,
};

const EMPTY_CATALOG_DIGEST_SEED: &str = "miniapp-m1-empty-catalog";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppKind {
    UiOnly,
    Service,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppProjectSourceState {
    Empty,
    Editable,
    RuntimeOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppProject {
    pub project_id: MiniAppProjectId,
    pub project_revision: u64,
    pub source_state: MiniAppProjectSourceState,
    pub build_generation: u64,
    pub source_snapshot_digest: Option<DigestHex>,
    pub dependency_lock_digest: Option<DigestHex>,
}

impl MiniAppProject {
    pub fn empty(project_id: MiniAppProjectId) -> Self {
        Self {
            project_id,
            project_revision: 1,
            source_state: MiniAppProjectSourceState::Empty,
            build_generation: 0,
            source_snapshot_digest: None,
            dependency_lock_digest: None,
        }
    }

    pub fn validate(&self) -> MiniAppPlatformResult<()> {
        if self.project_id.as_ref().trim().is_empty() || self.project_revision == 0 {
            return Err(MiniAppPlatformError::InvalidState(
                "project identity and revision must be positive".into(),
            ));
        }
        match self.source_state {
            MiniAppProjectSourceState::Empty => {
                if self.build_generation != 0
                    || self.source_snapshot_digest.is_some()
                    || self.dependency_lock_digest.is_some()
                {
                    return Err(MiniAppPlatformError::InvalidState(
                        "empty project cannot carry source lineage".into(),
                    ));
                }
            }
            MiniAppProjectSourceState::Editable => {
                if self.build_generation == 0
                    || self.source_snapshot_digest.is_none()
                    || self.dependency_lock_digest.is_none()
                    || self
                        .source_snapshot_digest
                        .as_ref()
                        .is_some_and(|digest| !is_digest(digest))
                    || self
                        .dependency_lock_digest
                        .as_ref()
                        .is_some_and(|digest| !is_digest(digest))
                {
                    return Err(MiniAppPlatformError::InvalidState(
                        "editable project requires complete source lineage".into(),
                    ));
                }
            }
            MiniAppProjectSourceState::RuntimeOnly => {
                if self.source_snapshot_digest.is_some() || self.dependency_lock_digest.is_some() {
                    return Err(MiniAppPlatformError::InvalidState(
                        "runtime-only project cannot expose editable source".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppConfigState {
    pub revision: u64,
    pub schema_digest: DigestHex,
    pub schema: StrictJsonValue,
    pub values: StrictJsonValue,
    pub valid: bool,
    pub validation_errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppCredentialBindings {
    pub revision: u64,
    pub slots: BTreeMap<String, Option<CredentialId>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MiniAppImportProvenance {
    Native,
    ShareBundle {
        bundle_digest: DigestHex,
        source_miniapp_id: Option<MiniAppId>,
    },
    WholeAppBackup {
        metadata_digest: DigestHex,
        source_miniapp_id: MiniAppId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredMiniAppRelease {
    pub miniapp_id: MiniAppId,
    pub artifact: MiniAppReleaseArtifactV1,
    pub ready: MiniAppReadyRelease,
}

impl StoredMiniAppRelease {
    pub fn new(
        miniapp_id: MiniAppId,
        artifact: MiniAppReleaseArtifactV1,
        ready: MiniAppReadyRelease,
    ) -> MiniAppPlatformResult<Self> {
        let value = Self {
            miniapp_id,
            artifact,
            ready,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn release_ref(&self) -> &MiniAppReleaseRef {
        &self.ready.release
    }

    pub fn validate(&self) -> MiniAppPlatformResult<()> {
        if self.ready.miniapp_id != self.miniapp_id {
            return Err(MiniAppPlatformError::InvalidState(
                "release provenance belongs to another MiniApp".into(),
            ));
        }
        self.ready.validate_for_artifact(&self.artifact)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppProduct {
    pub miniapp_id: MiniAppId,
    pub product_revision: u64,
    pub display_name: String,
    pub description: Option<String>,
    pub icon_asset_id: Option<String>,
    pub kind: MiniAppKind,
    pub lifecycle: MiniAppProductLifecycleState,
    pub pointers: MiniAppReleasePointerState,
    pub auto_publish: Option<MiniAppUiOnlyAutoPublishAuthorization>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppDataRoot {
    pub product: MiniAppProduct,
    pub project: MiniAppProject,
    pub releases: BTreeMap<DigestHex, StoredMiniAppRelease>,
    pub config: MiniAppConfigState,
    pub credential_bindings: MiniAppCredentialBindings,
    pub storage: MiniAppServiceStorageDescriptor,
    pub import_provenance: MiniAppImportProvenance,
}

impl MiniAppDataRoot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        miniapp_id: MiniAppId,
        project_id: MiniAppProjectId,
        display_name: String,
        description: Option<String>,
        kind: MiniAppKind,
        storage: MiniAppServiceStorageDescriptor,
        now_ms: i64,
    ) -> MiniAppPlatformResult<Self> {
        let empty_digest = digest_payload(&EMPTY_CATALOG_DIGEST_SEED)
            .map_err(|error| MiniAppPlatformError::InvalidState(error.to_string()))?;
        let value = Self {
            product: MiniAppProduct {
                miniapp_id: miniapp_id.clone(),
                product_revision: 1,
                display_name,
                description,
                icon_asset_id: None,
                kind,
                lifecycle: MiniAppProductLifecycleState::Disabled,
                pointers: MiniAppReleasePointerState {
                    miniapp_id,
                    pointer_revision: 1,
                    active_release_epoch: 0,
                    ready_release: None,
                    active_release: None,
                    previous_release: None,
                    materialized_catalog_digest: empty_digest.clone(),
                },
                auto_publish: None,
                updated_at_ms: now_ms,
            },
            project: MiniAppProject::empty(project_id),
            releases: BTreeMap::new(),
            config: MiniAppConfigState {
                revision: 1,
                schema_digest: empty_digest,
                schema: StrictJsonValue(serde_json::json!({"type": "object"})),
                values: StrictJsonValue(serde_json::json!({})),
                valid: true,
                validation_errors: Vec::new(),
            },
            credential_bindings: MiniAppCredentialBindings {
                revision: 1,
                slots: BTreeMap::new(),
            },
            storage,
            import_provenance: MiniAppImportProvenance::Native,
        };
        value.validate_structure()?;
        Ok(value)
    }

    pub fn validate_structure(&self) -> MiniAppPlatformResult<()> {
        let product = &self.product;
        if product.miniapp_id.as_ref().trim().is_empty()
            || product.product_revision == 0
            || product.display_name.trim().is_empty()
            || product.updated_at_ms <= 0
        {
            return Err(MiniAppPlatformError::InvalidState(
                "product identity, revision, display name, and timestamp are required".into(),
            ));
        }
        self.project.validate()?;
        product.pointers.validate()?;
        if product.pointers.miniapp_id != product.miniapp_id {
            return Err(MiniAppPlatformError::InvalidState(
                "pointer state belongs to another MiniApp".into(),
            ));
        }
        if self.config.revision == 0
            || self.credential_bindings.revision == 0
            || !is_digest(&self.config.schema_digest)
            || !self.config.schema.0.is_object()
            || !self.config.values.0.is_object()
        {
            return Err(MiniAppPlatformError::InvalidState(
                "config and credential state must be revisioned objects".into(),
            ));
        }
        if self
            .credential_bindings
            .slots
            .keys()
            .any(|slot| slot.trim().is_empty())
        {
            return Err(MiniAppPlatformError::InvalidState(
                "credential slot keys must not be empty".into(),
            ));
        }
        if let Some(authorization) = &product.auto_publish
            && (authorization.authorization_id.as_ref().trim().is_empty()
                || authorization.miniapp_id != product.miniapp_id
                || authorization.authorization_revision == 0
                || authorization.user_authorized_at_ms <= 0)
        {
            return Err(MiniAppPlatformError::InvalidState(
                "auto Publish authorization must belong to the exact MiniApp".into(),
            ));
        }
        match &self.import_provenance {
            MiniAppImportProvenance::Native => {}
            MiniAppImportProvenance::ShareBundle { bundle_digest, .. } => {
                if !is_digest(bundle_digest) {
                    return Err(MiniAppPlatformError::InvalidState(
                        "Share Bundle provenance requires a digest".into(),
                    ));
                }
            }
            MiniAppImportProvenance::WholeAppBackup {
                metadata_digest, ..
            } => {
                if !is_digest(metadata_digest) {
                    return Err(MiniAppPlatformError::InvalidState(
                        "Whole-App Backup provenance requires a digest".into(),
                    ));
                }
            }
        }
        self.validate_storage()?;

        for (digest, release) in &self.releases {
            release.validate()?;
            if release.miniapp_id != product.miniapp_id
                || digest != &release.artifact.artifact_digest
            {
                return Err(MiniAppPlatformError::InvalidState(
                    "release inventory identity or digest is inconsistent".into(),
                ));
            }
            self.validate_release_shape(release)?;
        }

        let referenced = self.referenced_release_digests();
        let stored = self.releases.keys().cloned().collect::<BTreeSet<_>>();
        if stored != referenced {
            return Err(MiniAppPlatformError::InvalidState(
                "release inventory must contain exactly Ready, Active, and Previous".into(),
            ));
        }
        self.validate_pointer_targets()?;

        if product.lifecycle == MiniAppProductLifecycleState::Enabled
            && product.pointers.active_release.is_none()
        {
            return Err(MiniAppPlatformError::InvalidState(
                "enabled MiniApp requires an Active Release".into(),
            ));
        }
        Ok(())
    }

    pub fn ready_release(&self) -> MiniAppPlatformResult<&StoredMiniAppRelease> {
        let ready = self
            .product
            .pointers
            .ready_release
            .as_ref()
            .ok_or_else(|| MiniAppPlatformError::InvalidState("no Ready Release".into()))?;
        self.releases
            .get(&ready.release_digest)
            .ok_or_else(|| MiniAppPlatformError::UnknownRelease(ready.release_digest.0.clone()))
    }

    pub fn active_release(&self) -> MiniAppPlatformResult<&StoredMiniAppRelease> {
        let active = self
            .product
            .pointers
            .active_release
            .as_ref()
            .ok_or_else(|| MiniAppPlatformError::InvalidState("no Active Release".into()))?;
        self.releases
            .get(&active.release_digest)
            .ok_or_else(|| MiniAppPlatformError::UnknownRelease(active.release_digest.0.clone()))
    }

    pub fn previous_release(&self) -> MiniAppPlatformResult<&StoredMiniAppRelease> {
        let previous = self
            .product
            .pointers
            .previous_release
            .as_ref()
            .ok_or_else(|| MiniAppPlatformError::InvalidState("no Previous Release".into()))?;
        self.releases
            .get(&previous.release_digest)
            .ok_or_else(|| MiniAppPlatformError::UnknownRelease(previous.release_digest.0.clone()))
    }

    pub fn credential_ids(&self) -> BTreeSet<&CredentialId> {
        self.credential_bindings
            .slots
            .values()
            .flatten()
            .collect()
    }

    pub fn ensure_release_mutable(&self) -> MiniAppPlatformResult<()> {
        if matches!(
            self.product.lifecycle,
            MiniAppProductLifecycleState::Trashed | MiniAppProductLifecycleState::Deleting
        ) {
            return Err(MiniAppPlatformError::LifecycleConflict(format!(
                "{:?}",
                self.product.lifecycle
            )));
        }
        Ok(())
    }

    pub(crate) fn replace_ready(
        &mut self,
        release: StoredMiniAppRelease,
        now_ms: i64,
    ) -> MiniAppPlatformResult<()> {
        self.ensure_release_mutable()?;
        release.validate()?;
        if release.miniapp_id != self.product.miniapp_id {
            return Err(MiniAppPlatformError::InvalidState(
                "Ready Release belongs to another MiniApp".into(),
            ));
        }
        self.validate_release_shape(&release)?;
        let digest = release.artifact.artifact_digest.clone();
        if self
            .product
            .pointers
            .active_release
            .as_ref()
            .is_some_and(|active| active.release_digest == digest)
            || self
                .product
                .pointers
                .previous_release
                .as_ref()
                .is_some_and(|previous| previous.release_digest == digest)
        {
            return Err(MiniAppPlatformError::InvalidState(
                "Ready Release must differ from Active and Previous".into(),
            ));
        }
        self.product.pointers.pointer_revision =
            checked_increment(self.product.pointers.pointer_revision, "pointer revision")?;
        self.product.pointers.ready_release =
            Some(nomifun_agent_contracts::MiniAppReadyReleaseRef::from(&release.ready));
        self.releases.insert(digest, release);
        self.retain_pointer_releases();
        self.bump(now_ms)?;
        self.validate_structure()
    }

    pub(crate) fn apply_pointer_state(
        &mut self,
        next: MiniAppReleasePointerState,
        now_ms: i64,
    ) -> MiniAppPlatformResult<()> {
        self.ensure_release_mutable()?;
        if next.miniapp_id != self.product.miniapp_id {
            return Err(MiniAppPlatformError::InvalidState(
                "next pointer state belongs to another MiniApp".into(),
            ));
        }
        next.validate()?;
        self.product.pointers = next;
        self.retain_pointer_releases();
        self.bump(now_ms)?;
        self.validate_structure()
    }

    pub(crate) fn apply_lifecycle(
        &mut self,
        lifecycle: MiniAppProductLifecycleState,
        now_ms: i64,
    ) -> MiniAppPlatformResult<()> {
        self.product.lifecycle = lifecycle;
        self.bump(now_ms)?;
        self.validate_structure()
    }

    pub(crate) fn set_auto_publish(
        &mut self,
        authorization: Option<MiniAppUiOnlyAutoPublishAuthorization>,
        now_ms: i64,
    ) -> MiniAppPlatformResult<()> {
        self.ensure_release_mutable()?;
        self.product.auto_publish = authorization;
        self.bump(now_ms)?;
        self.validate_structure()
    }

    fn validate_storage(&self) -> MiniAppPlatformResult<()> {
        let miniapp_id = &self.product.miniapp_id;
        if self.storage.kv.handle_id.as_ref().trim().is_empty()
            || self.storage.kv.miniapp_id != *miniapp_id
            || self.storage.kv.namespace_revision == 0
        {
            return Err(MiniAppPlatformError::InvalidState(
                "KV handle must be positive and owned by the MiniApp".into(),
            ));
        }
        if self.storage.files_dir.as_ref().is_some_and(|files| {
            files.handle_id.as_ref().trim().is_empty()
                || files.miniapp_id != *miniapp_id
                || files.absolute_path.trim().is_empty()
        }) {
            return Err(MiniAppPlatformError::InvalidState(
                "files handle must be owned by the MiniApp".into(),
            ));
        }
        if self.storage.private_database.as_ref().is_some_and(|database| {
            database.handle_id.as_ref().trim().is_empty()
                || database.miniapp_id != *miniapp_id
                || database.schema_epoch == 0
                || !is_digest(&database.migration_ledger_digest)
        }) {
            return Err(MiniAppPlatformError::InvalidState(
                "Private SQLite handle must be revisioned and owned by the MiniApp".into(),
            ));
        }
        if self.product.kind == MiniAppKind::UiOnly
            && (self.storage.files_dir.is_some() || self.storage.private_database.is_some())
        {
            return Err(MiniAppPlatformError::InvalidState(
                "UI-only MiniApp can own Host KV only".into(),
            ));
        }
        Ok(())
    }

    fn validate_release_shape(&self, release: &StoredMiniAppRelease) -> MiniAppPlatformResult<()> {
        let service = release.artifact.manifest.payload.service.as_ref();
        match (self.product.kind, service) {
            (MiniAppKind::UiOnly, None) => Ok(()),
            (MiniAppKind::UiOnly, Some(_)) => Err(MiniAppPlatformError::InvalidState(
                "UI-only MiniApp cannot stage a Service Release".into(),
            )),
            (MiniAppKind::Service, None) => Err(MiniAppPlatformError::InvalidState(
                "Service MiniApp requires exactly one service/main.mjs".into(),
            )),
            (MiniAppKind::Service, Some(service)) => {
                if self.storage.files_dir.is_some() != service.uses_files
                    || self.storage.private_database.is_some() != service.uses_private_database
                {
                    return Err(MiniAppPlatformError::InvalidState(
                        "Service Release storage contract differs from the managed data root"
                            .into(),
                    ));
                }
                Ok(())
            }
        }
    }

    fn validate_pointer_targets(&self) -> MiniAppPlatformResult<()> {
        if let Some(ready) = &self.product.pointers.ready_release {
            let stored = self
                .releases
                .get(&ready.release_digest)
                .ok_or_else(|| MiniAppPlatformError::UnknownRelease(ready.release_digest.0.clone()))?;
            if stored.ready.release.release_id != ready.release_id {
                return Err(MiniAppPlatformError::InvalidState(
                    "Ready pointer does not match stored release identity".into(),
                ));
            }
        }
        for pointer in [
            self.product.pointers.active_release.as_ref(),
            self.product.pointers.previous_release.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let stored = self
                .releases
                .get(&pointer.release_digest)
                .ok_or_else(|| MiniAppPlatformError::UnknownRelease(pointer.release_digest.0.clone()))?;
            if stored.release_ref() != pointer {
                return Err(MiniAppPlatformError::InvalidState(
                    "Release pointer does not bind the stored immutable artifact".into(),
                ));
            }
        }
        Ok(())
    }

    fn referenced_release_digests(&self) -> BTreeSet<DigestHex> {
        self.product
            .pointers
            .ready_release
            .iter()
            .map(|ready| ready.release_digest.clone())
            .chain(
                self.product
                    .pointers
                    .active_release
                    .iter()
                    .map(|active| active.release_digest.clone()),
            )
            .chain(
                self.product
                    .pointers
                    .previous_release
                    .iter()
                    .map(|previous| previous.release_digest.clone()),
            )
            .collect()
    }

    fn retain_pointer_releases(&mut self) {
        let retained = self.referenced_release_digests();
        self.releases.retain(|digest, _| retained.contains(digest));
    }

    fn bump(&mut self, now_ms: i64) -> MiniAppPlatformResult<()> {
        if now_ms < self.product.updated_at_ms {
            return Err(MiniAppPlatformError::InvalidState(
                "product timestamp cannot move backwards".into(),
            ));
        }
        self.product.product_revision =
            checked_increment(self.product.product_revision, "product revision")?;
        self.product.updated_at_ms = now_ms;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppCatalogRecord {
    pub miniapp_id: MiniAppId,
    pub active_release: MiniAppReleaseRef,
    pub active_release_epoch: u64,
    pub catalog_digest: DigestHex,
}

impl MiniAppCatalogRecord {
    pub fn for_root(root: &MiniAppDataRoot) -> MiniAppPlatformResult<Self> {
        let active_release = root.product.pointers.active_release.clone().ok_or_else(|| {
            MiniAppPlatformError::InvalidState(
                "catalog publication requires an Active Release".into(),
            )
        })?;
        Ok(Self {
            miniapp_id: root.product.miniapp_id.clone(),
            active_release,
            active_release_epoch: root.product.pointers.active_release_epoch,
            catalog_digest: root
                .product
                .pointers
                .materialized_catalog_digest
                .clone(),
        })
    }

    fn validate_for(&self, root: &MiniAppDataRoot) -> MiniAppPlatformResult<()> {
        if self.miniapp_id != root.product.miniapp_id
            || root.product.pointers.active_release.as_ref() != Some(&self.active_release)
            || self.active_release_epoch != root.product.pointers.active_release_epoch
            || self.catalog_digest != root.product.pointers.materialized_catalog_digest
        {
            return Err(MiniAppPlatformError::InvalidState(
                "catalog publication does not match the exact Active Release epoch".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppDeletionRecord {
    pub intent: MiniAppDeletingIntent,
    pub operation: DurableMiniAppOperation,
}

impl MiniAppDeletionRecord {
    pub fn validate_for(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        if self.intent.miniapp_id != *miniapp_id
            || self.operation.miniapp_id != *miniapp_id
            || self.intent.operation_id != self.operation.operation_id
            || self.operation.kind != MiniAppOperationKind::PermanentDelete
            || self.operation.cancelable
            || !matches!(
                self.operation.state,
                MiniAppOperationState::Running | MiniAppOperationState::Failed
            )
        {
            return Err(MiniAppPlatformError::InvalidState(
                "deleting intent and non-cancelable operation must describe the same MiniApp"
                    .into(),
            ));
        }
        self.operation.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppRepositorySnapshot {
    pub library_revision: u64,
    pub root: MiniAppDataRoot,
    pub catalog: Option<MiniAppCatalogRecord>,
    pub deletion: Option<MiniAppDeletionRecord>,
}

impl MiniAppRepositorySnapshot {
    pub fn validate(&self) -> MiniAppPlatformResult<()> {
        self.root.validate_structure()?;
        if let Some(catalog) = &self.catalog {
            catalog.validate_for(&self.root)?;
        }
        if let Some(deletion) = &self.deletion {
            deletion.validate_for(&self.root.product.miniapp_id)?;
        }
        let lifecycle = MiniAppProductLifecycleRecord {
            miniapp_id: self.root.product.miniapp_id.clone(),
            state: self.root.product.lifecycle,
            pointer_state: self.root.product.pointers.clone(),
            surface_available: self.root.product.lifecycle
                == MiniAppProductLifecycleState::Enabled,
            catalog_published: self.catalog.is_some(),
            deleting_intent: self
                .deletion
                .as_ref()
                .map(|deletion| deletion.intent.clone()),
        };
        lifecycle.validate()?;
        Ok(())
    }

    pub fn miniapp_id(&self) -> &MiniAppId {
        &self.root.product.miniapp_id
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppWholeAppBackupPayload {
    pub root: MiniAppDataRoot,
    pub release_digests: BTreeSet<DigestHex>,
    pub metadata_digest: DigestHex,
}

fn checked_increment(value: u64, field: &str) -> MiniAppPlatformResult<u64> {
    value
        .checked_add(1)
        .ok_or_else(|| MiniAppPlatformError::InvalidState(format!("{field} overflow")))
}

fn is_digest(value: &DigestHex) -> bool {
    value.as_ref().len() == 64
        && value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn new_delete_record(
    miniapp_id: MiniAppId,
    operation_id: OperationId,
    now_ms: i64,
) -> MiniAppPlatformResult<MiniAppDeletionRecord> {
    let operation = DurableMiniAppOperation::running(
        operation_id.clone(),
        miniapp_id.clone(),
        MiniAppOperationKind::PermanentDelete,
        false,
        now_ms,
    )?;
    let value = MiniAppDeletionRecord {
        intent: MiniAppDeletingIntent {
            miniapp_id,
            operation_id,
            started_at_ms: now_ms,
            last_error: None,
        },
        operation,
    };
    value.validate_for(&value.intent.miniapp_id)?;
    Ok(value)
}
