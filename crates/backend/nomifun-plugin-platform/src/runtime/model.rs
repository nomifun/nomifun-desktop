use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    CredentialId, DigestHex, MiniAppDeletingIntent, MiniAppId, MiniAppProductLifecycleRecord,
    MiniAppProductLifecycleState, MiniAppProjectId, MiniAppReadyRelease,
    MiniAppReleaseArtifactV1, MiniAppReleasePointerState, MiniAppReleaseRef,
    MiniAppServiceStorageDescriptor, MiniAppUiOnlyAutoPublishAuthorization, OperationId,
    StrictJsonValue, digest_payload,
};
use serde::{Deserialize, Serialize};

use crate::runtime::{
    DurablePluginRuntimeOperation, PluginRuntimeOperationKind, PluginRuntimeOperationState, PluginRuntimePlatformError,
    PluginRuntimePlatformResult,
};

const EMPTY_CATALOG_DIGEST_SEED: &str = "miniapp-m1-empty-catalog";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeKind {
    UiOnly,
    Service,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeProjectSourceState {
    Empty,
    Editable,
    RuntimeOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeProject {
    pub project_id: MiniAppProjectId,
    pub project_revision: u64,
    pub source_state: PluginRuntimeProjectSourceState,
    pub build_generation: u64,
    pub source_snapshot_digest: Option<DigestHex>,
    pub dependency_lock_digest: Option<DigestHex>,
}

impl PluginRuntimeProject {
    pub fn empty(project_id: MiniAppProjectId) -> Self {
        Self {
            project_id,
            project_revision: 1,
            source_state: PluginRuntimeProjectSourceState::Empty,
            build_generation: 0,
            source_snapshot_digest: None,
            dependency_lock_digest: None,
        }
    }

    pub fn validate(&self) -> PluginRuntimePlatformResult<()> {
        if self.project_id.as_ref().trim().is_empty() || self.project_revision == 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
                "project identity and revision must be positive".into(),
            ));
        }
        match self.source_state {
            PluginRuntimeProjectSourceState::Empty => {
                if self.build_generation != 0
                    || self.source_snapshot_digest.is_some()
                    || self.dependency_lock_digest.is_some()
                {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "empty project cannot carry source lineage".into(),
                    ));
                }
            }
            PluginRuntimeProjectSourceState::Editable => {
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
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "editable project requires complete source lineage".into(),
                    ));
                }
            }
            PluginRuntimeProjectSourceState::RuntimeOnly => {
                if self.source_snapshot_digest.is_some() || self.dependency_lock_digest.is_some() {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "runtime-only project cannot expose editable source".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRuntimeConfigState {
    pub revision: u64,
    pub schema_digest: DigestHex,
    pub schema: StrictJsonValue,
    pub values: StrictJsonValue,
    pub valid: bool,
    pub validation_errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeCredentialBindings {
    pub revision: u64,
    pub slots: BTreeMap<String, Option<CredentialId>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginRuntimeImportProvenance {
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
pub struct StoredPluginRuntimeRelease {
    pub miniapp_id: MiniAppId,
    pub artifact: MiniAppReleaseArtifactV1,
    pub ready: MiniAppReadyRelease,
}

impl StoredPluginRuntimeRelease {
    pub fn new(
        miniapp_id: MiniAppId,
        artifact: MiniAppReleaseArtifactV1,
        ready: MiniAppReadyRelease,
    ) -> PluginRuntimePlatformResult<Self> {
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

    pub fn validate(&self) -> PluginRuntimePlatformResult<()> {
        if self.ready.miniapp_id != self.miniapp_id {
            return Err(PluginRuntimePlatformError::InvalidState(
                "release provenance belongs to another Plugin".into(),
            ));
        }
        self.ready.validate_for_artifact(&self.artifact)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRuntimeProduct {
    pub miniapp_id: MiniAppId,
    pub product_revision: u64,
    pub display_name: String,
    pub description: Option<String>,
    pub icon_asset_id: Option<String>,
    pub kind: PluginRuntimeKind,
    pub lifecycle: MiniAppProductLifecycleState,
    pub pointers: MiniAppReleasePointerState,
    pub auto_publish: Option<MiniAppUiOnlyAutoPublishAuthorization>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRuntimeDataRoot {
    pub product: PluginRuntimeProduct,
    pub project: PluginRuntimeProject,
    pub releases: BTreeMap<DigestHex, StoredPluginRuntimeRelease>,
    pub config: PluginRuntimeConfigState,
    pub credential_bindings: PluginRuntimeCredentialBindings,
    pub storage: MiniAppServiceStorageDescriptor,
    pub import_provenance: PluginRuntimeImportProvenance,
}

impl PluginRuntimeDataRoot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        miniapp_id: MiniAppId,
        project_id: MiniAppProjectId,
        display_name: String,
        description: Option<String>,
        kind: PluginRuntimeKind,
        storage: MiniAppServiceStorageDescriptor,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<Self> {
        let empty_digest = digest_payload(&EMPTY_CATALOG_DIGEST_SEED)
            .map_err(|error| PluginRuntimePlatformError::InvalidState(error.to_string()))?;
        let value = Self {
            product: PluginRuntimeProduct {
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
            project: PluginRuntimeProject::empty(project_id),
            releases: BTreeMap::new(),
            config: PluginRuntimeConfigState {
                revision: 1,
                schema_digest: empty_digest,
                schema: StrictJsonValue(serde_json::json!({"type": "object"})),
                values: StrictJsonValue(serde_json::json!({})),
                valid: true,
                validation_errors: Vec::new(),
            },
            credential_bindings: PluginRuntimeCredentialBindings {
                revision: 1,
                slots: BTreeMap::new(),
            },
            storage,
            import_provenance: PluginRuntimeImportProvenance::Native,
        };
        value.validate_structure()?;
        Ok(value)
    }

    pub fn validate_structure(&self) -> PluginRuntimePlatformResult<()> {
        let product = &self.product;
        if product.miniapp_id.as_ref().trim().is_empty()
            || product.product_revision == 0
            || product.display_name.trim().is_empty()
            || product.updated_at_ms <= 0
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "product identity, revision, display name, and timestamp are required".into(),
            ));
        }
        self.project.validate()?;
        product.pointers.validate()?;
        if product.pointers.miniapp_id != product.miniapp_id {
            return Err(PluginRuntimePlatformError::InvalidState(
                "pointer state belongs to another Plugin".into(),
            ));
        }
        if self.config.revision == 0
            || self.credential_bindings.revision == 0
            || !is_digest(&self.config.schema_digest)
            || !self.config.schema.0.is_object()
            || !self.config.values.0.is_object()
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "config and credential state must be revisioned objects".into(),
            ));
        }
        if self
            .credential_bindings
            .slots
            .keys()
            .any(|slot| slot.trim().is_empty())
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "credential slot keys must not be empty".into(),
            ));
        }
        if let Some(authorization) = &product.auto_publish
            && (authorization.authorization_id.as_ref().trim().is_empty()
                || authorization.miniapp_id != product.miniapp_id
                || authorization.authorization_revision == 0
                || authorization.user_authorized_at_ms <= 0)
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "auto Publish authorization must belong to the exact Plugin".into(),
            ));
        }
        match &self.import_provenance {
            PluginRuntimeImportProvenance::Native => {}
            PluginRuntimeImportProvenance::ShareBundle { bundle_digest, .. } => {
                if !is_digest(bundle_digest) {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "Share Bundle provenance requires a digest".into(),
                    ));
                }
            }
            PluginRuntimeImportProvenance::WholeAppBackup {
                metadata_digest, ..
            } => {
                if !is_digest(metadata_digest) {
                    return Err(PluginRuntimePlatformError::InvalidState(
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
                return Err(PluginRuntimePlatformError::InvalidState(
                    "release inventory identity or digest is inconsistent".into(),
                ));
            }
            self.validate_release_shape(release)?;
        }

        let referenced = self.referenced_release_digests();
        let stored = self.releases.keys().cloned().collect::<BTreeSet<_>>();
        if stored != referenced {
            return Err(PluginRuntimePlatformError::InvalidState(
                "release inventory must contain exactly Ready, Active, and Previous".into(),
            ));
        }
        self.validate_pointer_targets()?;

        if product.lifecycle == MiniAppProductLifecycleState::Enabled
            && product.pointers.active_release.is_none()
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "enabled Plugin requires an Active Release".into(),
            ));
        }
        Ok(())
    }

    pub fn ready_release(&self) -> PluginRuntimePlatformResult<&StoredPluginRuntimeRelease> {
        let ready = self
            .product
            .pointers
            .ready_release
            .as_ref()
            .ok_or_else(|| PluginRuntimePlatformError::InvalidState("no Ready Release".into()))?;
        self.releases
            .get(&ready.release_digest)
            .ok_or_else(|| PluginRuntimePlatformError::UnknownRelease(ready.release_digest.0.clone()))
    }

    pub fn active_release(&self) -> PluginRuntimePlatformResult<&StoredPluginRuntimeRelease> {
        let active = self
            .product
            .pointers
            .active_release
            .as_ref()
            .ok_or_else(|| PluginRuntimePlatformError::InvalidState("no Active Release".into()))?;
        self.releases
            .get(&active.release_digest)
            .ok_or_else(|| PluginRuntimePlatformError::UnknownRelease(active.release_digest.0.clone()))
    }

    pub fn previous_release(&self) -> PluginRuntimePlatformResult<&StoredPluginRuntimeRelease> {
        let previous = self
            .product
            .pointers
            .previous_release
            .as_ref()
            .ok_or_else(|| PluginRuntimePlatformError::InvalidState("no Previous Release".into()))?;
        self.releases
            .get(&previous.release_digest)
            .ok_or_else(|| PluginRuntimePlatformError::UnknownRelease(previous.release_digest.0.clone()))
    }

    pub fn credential_ids(&self) -> BTreeSet<&CredentialId> {
        self.credential_bindings
            .slots
            .values()
            .flatten()
            .collect()
    }

    pub fn ensure_release_mutable(&self) -> PluginRuntimePlatformResult<()> {
        if matches!(
            self.product.lifecycle,
            MiniAppProductLifecycleState::Trashed | MiniAppProductLifecycleState::Deleting
        ) {
            return Err(PluginRuntimePlatformError::LifecycleConflict(format!(
                "{:?}",
                self.product.lifecycle
            )));
        }
        Ok(())
    }

    pub(crate) fn replace_ready(
        &mut self,
        release: StoredPluginRuntimeRelease,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<()> {
        self.ensure_release_mutable()?;
        release.validate()?;
        if release.miniapp_id != self.product.miniapp_id {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Ready Release belongs to another Plugin".into(),
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
            return Err(PluginRuntimePlatformError::InvalidState(
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
    ) -> PluginRuntimePlatformResult<()> {
        self.ensure_release_mutable()?;
        if next.miniapp_id != self.product.miniapp_id {
            return Err(PluginRuntimePlatformError::InvalidState(
                "next pointer state belongs to another Plugin".into(),
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
    ) -> PluginRuntimePlatformResult<()> {
        self.product.lifecycle = lifecycle;
        self.bump(now_ms)?;
        self.validate_structure()
    }

    pub(crate) fn set_auto_publish(
        &mut self,
        authorization: Option<MiniAppUiOnlyAutoPublishAuthorization>,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<()> {
        self.ensure_release_mutable()?;
        self.product.auto_publish = authorization;
        self.bump(now_ms)?;
        self.validate_structure()
    }

    fn validate_storage(&self) -> PluginRuntimePlatformResult<()> {
        let miniapp_id = &self.product.miniapp_id;
        if self.storage.kv.handle_id.as_ref().trim().is_empty()
            || self.storage.kv.miniapp_id != *miniapp_id
            || self.storage.kv.namespace_revision == 0
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "KV handle must be positive and owned by the Plugin".into(),
            ));
        }
        if self.storage.files_dir.as_ref().is_some_and(|files| {
            files.handle_id.as_ref().trim().is_empty()
                || files.miniapp_id != *miniapp_id
                || files.absolute_path.trim().is_empty()
        }) {
            return Err(PluginRuntimePlatformError::InvalidState(
                "files handle must be owned by the Plugin".into(),
            ));
        }
        if self.storage.private_database.as_ref().is_some_and(|database| {
            database.handle_id.as_ref().trim().is_empty()
                || database.miniapp_id != *miniapp_id
                || database.schema_epoch == 0
                || !is_digest(&database.migration_ledger_digest)
        }) {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Private SQLite handle must be revisioned and owned by the Plugin".into(),
            ));
        }
        if self.product.kind == PluginRuntimeKind::UiOnly
            && (self.storage.files_dir.is_some() || self.storage.private_database.is_some())
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "UI-only Plugin can own Host KV only".into(),
            ));
        }
        Ok(())
    }

    fn validate_release_shape(&self, release: &StoredPluginRuntimeRelease) -> PluginRuntimePlatformResult<()> {
        let service = release.artifact.manifest.payload.service.as_ref();
        match (self.product.kind, service) {
            (PluginRuntimeKind::UiOnly, None) => Ok(()),
            (PluginRuntimeKind::UiOnly, Some(_)) => Err(PluginRuntimePlatformError::InvalidState(
                "UI-only Plugin cannot stage a Service Release".into(),
            )),
            (PluginRuntimeKind::Service, None) => Err(PluginRuntimePlatformError::InvalidState(
                "Service Plugin requires exactly one service/main.mjs".into(),
            )),
            (PluginRuntimeKind::Service, Some(service)) => {
                if self.storage.files_dir.is_some() != service.uses_files
                    || self.storage.private_database.is_some() != service.uses_private_database
                {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "Service Release storage contract differs from the managed data root"
                            .into(),
                    ));
                }
                Ok(())
            }
        }
    }

    fn validate_pointer_targets(&self) -> PluginRuntimePlatformResult<()> {
        if let Some(ready) = &self.product.pointers.ready_release {
            let stored = self
                .releases
                .get(&ready.release_digest)
                .ok_or_else(|| PluginRuntimePlatformError::UnknownRelease(ready.release_digest.0.clone()))?;
            if stored.ready.release.release_id != ready.release_id {
                return Err(PluginRuntimePlatformError::InvalidState(
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
                .ok_or_else(|| PluginRuntimePlatformError::UnknownRelease(pointer.release_digest.0.clone()))?;
            if stored.release_ref() != pointer {
                return Err(PluginRuntimePlatformError::InvalidState(
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

    fn bump(&mut self, now_ms: i64) -> PluginRuntimePlatformResult<()> {
        if now_ms < self.product.updated_at_ms {
            return Err(PluginRuntimePlatformError::InvalidState(
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
pub struct PluginRuntimeCatalogRecord {
    pub miniapp_id: MiniAppId,
    pub active_release: MiniAppReleaseRef,
    pub active_release_epoch: u64,
    pub catalog_digest: DigestHex,
}

impl PluginRuntimeCatalogRecord {
    pub fn for_root(root: &PluginRuntimeDataRoot) -> PluginRuntimePlatformResult<Self> {
        let active_release = root.product.pointers.active_release.clone().ok_or_else(|| {
            PluginRuntimePlatformError::InvalidState(
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

    fn validate_for(&self, root: &PluginRuntimeDataRoot) -> PluginRuntimePlatformResult<()> {
        if self.miniapp_id != root.product.miniapp_id
            || root.product.pointers.active_release.as_ref() != Some(&self.active_release)
            || self.active_release_epoch != root.product.pointers.active_release_epoch
            || self.catalog_digest != root.product.pointers.materialized_catalog_digest
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "catalog publication does not match the exact Active Release epoch".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeDeletionRecord {
    pub intent: MiniAppDeletingIntent,
    pub operation: DurablePluginRuntimeOperation,
}

impl PluginRuntimeDeletionRecord {
    pub fn validate_for(&self, miniapp_id: &MiniAppId) -> PluginRuntimePlatformResult<()> {
        if self.intent.miniapp_id != *miniapp_id
            || self.operation.miniapp_id != *miniapp_id
            || self.intent.operation_id != self.operation.operation_id
            || self.operation.kind != PluginRuntimeOperationKind::PermanentDelete
            || self.operation.cancelable
            || !matches!(
                self.operation.state,
                PluginRuntimeOperationState::Running | PluginRuntimeOperationState::Failed
            )
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "deleting intent and non-cancelable operation must describe the same Plugin"
                    .into(),
            ));
        }
        self.operation.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRuntimeRepositorySnapshot {
    pub library_revision: u64,
    pub root: PluginRuntimeDataRoot,
    pub catalog: Option<PluginRuntimeCatalogRecord>,
    pub deletion: Option<PluginRuntimeDeletionRecord>,
}

impl PluginRuntimeRepositorySnapshot {
    pub fn validate(&self) -> PluginRuntimePlatformResult<()> {
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
pub struct PluginRuntimeWholeAppBackupPayload {
    pub root: PluginRuntimeDataRoot,
    pub release_digests: BTreeSet<DigestHex>,
    pub metadata_digest: DigestHex,
}

fn checked_increment(value: u64, field: &str) -> PluginRuntimePlatformResult<u64> {
    value
        .checked_add(1)
        .ok_or_else(|| PluginRuntimePlatformError::InvalidState(format!("{field} overflow")))
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
) -> PluginRuntimePlatformResult<PluginRuntimeDeletionRecord> {
    let operation = DurablePluginRuntimeOperation::running(
        operation_id.clone(),
        miniapp_id.clone(),
        PluginRuntimeOperationKind::PermanentDelete,
        false,
        now_ms,
    )?;
    let value = PluginRuntimeDeletionRecord {
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
