//! Canonical Plugin Product runtime machine contracts.
//!
//! This module freezes immutable Release, Service, Bridge, storage, publish,
//! sharing, backup, and deletion shapes. It intentionally contains no runtime,
//! database, filesystem, or product-service implementation.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::plugin_n1::CredentialSlotDeclaration;
use crate::{NativePluginTarget, PluginServiceExecution};
use crate::{
    ArtifactEnvelope, ArtifactId, CanonicalErrorCode, CanonicalSchemaRef,
    DigestHex, JavaScriptBuildProfile, LocalizedMetadata, OperationId,
    PackageContributions, PackageRef, PluginBackupId, PluginBridgeCallId,
    PluginBridgeSessionId, PluginDatabaseHandleId, PluginFilesHandleId,
    PluginKvHandleId, PluginMigrationId, PluginProductId, PluginProjectId,
    PluginReleaseId, PluginServiceTestReceiptId, PluginShareBundleId,
    PluginSurfaceSessionId, PluginUserAuthorizationId, ResourceKind,
    RuntimeInstallationId, RuntimeTarget, StrictJsonValue, VersionString,
    digest_payload,
};

pub const PLUGIN_RUNTIME_SCHEMA_VERSION: &str = "1.0.0";
pub const PLUGIN_RELEASE_PROFILE_VERSION: &str = "1.0.0";
pub const PLUGIN_SERVICE_HOST_PROTOCOL_VERSION: &str = "1.0.0";
pub const PLUGIN_SERVICE_SDK_CONTRACT_VERSION: &str = "1.0.0";
pub const PLUGIN_BRIDGE_CONTRACT_VERSION: &str = "1.0.0";
pub const PLUGIN_SERVICE_TEST_CONTRACT_VERSION: &str = "1.0.0";
pub const PLUGIN_SHARE_BUNDLE_VERSION: &str = "1.0.0";
pub const PLUGIN_PRODUCT_BACKUP_VERSION: &str = "1.0.0";

pub type PluginRuntimeContractArtifact = ArtifactEnvelope<PluginRuntimeContractManifest>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeContractManifest {
    pub schema_version: VersionString,
    pub release_profile_version: VersionString,
    pub service_host_protocol_version: VersionString,
    pub service_sdk_contract_version: VersionString,
    pub bridge_contract_version: VersionString,
    pub service_test_contract_version: VersionString,
    pub share_bundle_version: VersionString,
    pub product_backup_version: VersionString,
    pub service_lifecycles: BTreeSet<PluginServiceLifecycle>,
    pub bridge_transports: BTreeSet<PluginBridgeTransport>,
    pub max_services_per_plugin_product: u8,
    pub ui_only_starts_node: bool,
    pub exposes_localhost_bridge: bool,
}

impl PluginRuntimeContractManifest {
    pub fn canonical() -> Self {
        Self {
            schema_version: PLUGIN_RUNTIME_SCHEMA_VERSION.into(),
            release_profile_version: PLUGIN_RELEASE_PROFILE_VERSION.into(),
            service_host_protocol_version: PLUGIN_SERVICE_HOST_PROTOCOL_VERSION.into(),
            service_sdk_contract_version: PLUGIN_SERVICE_SDK_CONTRACT_VERSION.into(),
            bridge_contract_version: PLUGIN_BRIDGE_CONTRACT_VERSION.into(),
            service_test_contract_version: PLUGIN_SERVICE_TEST_CONTRACT_VERSION.into(),
            share_bundle_version: PLUGIN_SHARE_BUNDLE_VERSION.into(),
            product_backup_version: PLUGIN_PRODUCT_BACKUP_VERSION.into(),
            service_lifecycles: BTreeSet::from([
                PluginServiceLifecycle::OnDemand,
                PluginServiceLifecycle::Continuous,
            ]),
            bridge_transports: BTreeSet::from([
                PluginBridgeTransport::MessageChannelV1,
            ]),
            max_services_per_plugin_product: 1,
            ui_only_starts_node: false,
            exposes_localhost_bridge: false,
        }
    }

    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        if self != &Self::canonical() {
            return Err(invalid(
                "plugin_runtime_contract",
                "manifest differs from the frozen Plugin runtime exact set",
            ));
        }
        Ok(())
    }
}

pub type PluginReleaseManifestArtifact = ArtifactEnvelope<PluginReleaseV1Manifest>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginUiReleaseDescriptor {
    pub entrypoint: String,
    pub entrypoint_digest: DigestHex,
    pub ui_tree_digest: DigestHex,
}

impl PluginUiReleaseDescriptor {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        if self.entrypoint != "ui/index.html" {
            return Err(invalid(
                "ui.entrypoint",
                "Plugin Product UI entrypoint must be ui/index.html",
            ));
        }
        validate_digest(&self.entrypoint_digest, "ui.entrypoint_digest")?;
        validate_digest(&self.ui_tree_digest, "ui.ui_tree_digest")
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginServiceLifecycle {
    OnDemand,
    Continuous,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceReleaseDescriptor {
    #[serde(default, skip_serializing_if = "PluginServiceExecution::is_node")]
    pub execution: PluginServiceExecution,
    pub entrypoint: String,
    pub module_digest: DigestHex,
    pub lifecycle: PluginServiceLifecycle,
    pub uses_files: bool,
    pub uses_private_database: bool,
    pub service_contract_digest: DigestHex,
    pub host_protocol_version: VersionString,
    pub sdk_contract_version: VersionString,
    pub runtime_requirements_digest: DigestHex,
}

impl PluginServiceReleaseDescriptor {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        if self.entrypoint != self.execution.entrypoint() {
            return Err(invalid(
                "service.entrypoint",
                "Service entrypoint does not match its declared execution backend",
            ));
        }
        validate_digest(&self.module_digest, "service.module_digest")?;
        validate_digest(
            &self.service_contract_digest,
            "service.service_contract_digest",
        )?;
        require_version(
            &self.host_protocol_version,
            PLUGIN_SERVICE_HOST_PROTOCOL_VERSION,
            "service.host_protocol_version",
        )?;
        require_version(
            &self.sdk_contract_version,
            PLUGIN_SERVICE_SDK_CONTRACT_VERSION,
            "service.sdk_contract_version",
        )?;
        validate_digest(
            &self.runtime_requirements_digest,
            "service.runtime_requirements_digest",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginMigrationColumn {
    pub name: String,
    pub declared_type: String,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_literal: Option<String>,
}

impl PluginMigrationColumn {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_sql_identifier(&self.name, "migration.column.name")?;
        validate_nonempty(&self.declared_type, "migration.column.declared_type")?;
        reject_sql_control_tokens(
            &self.declared_type,
            "migration.column.declared_type",
        )?;
        if let Some(default_literal) = &self.default_literal {
            validate_nonempty(default_literal, "migration.column.default_literal")?;
            reject_sql_control_tokens(
                default_literal,
                "migration.column.default_literal",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginAdditiveMigrationAction {
    CreateTable {
        table_name: String,
        columns: Vec<PluginMigrationColumn>,
        primary_key_columns: Vec<String>,
    },
    CreateIndex {
        index_name: String,
        table_name: String,
        columns: Vec<String>,
        unique: bool,
    },
    AddColumn {
        table_name: String,
        column: PluginMigrationColumn,
    },
}

impl PluginAdditiveMigrationAction {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        match self {
            Self::CreateTable {
                table_name,
                columns,
                primary_key_columns,
            } => {
                validate_sql_identifier(table_name, "migration.table_name")?;
                if columns.is_empty() {
                    return Err(invalid(
                        "migration.columns",
                        "CREATE TABLE requires at least one column",
                    ));
                }
                let mut names = BTreeSet::new();
                for column in columns {
                    column.validate()?;
                    if !names.insert(column.name.as_str()) {
                        return Err(duplicate("migration.column", &column.name));
                    }
                }
                for primary_key in primary_key_columns {
                    validate_sql_identifier(primary_key, "migration.primary_key_column")?;
                    if !names.contains(primary_key.as_str()) {
                        return Err(invalid(
                            "migration.primary_key_columns",
                            "primary key columns must exist in the new table",
                        ));
                    }
                }
                Ok(())
            }
            Self::CreateIndex {
                index_name,
                table_name,
                columns,
                ..
            } => {
                validate_sql_identifier(index_name, "migration.index_name")?;
                validate_sql_identifier(table_name, "migration.table_name")?;
                if columns.is_empty() {
                    return Err(invalid(
                        "migration.columns",
                        "CREATE INDEX requires at least one column",
                    ));
                }
                let mut unique_columns = BTreeSet::new();
                for column in columns {
                    validate_sql_identifier(column, "migration.index_column")?;
                    if !unique_columns.insert(column.as_str()) {
                        return Err(duplicate("migration.index_column", column));
                    }
                }
                Ok(())
            }
            Self::AddColumn { table_name, column } => {
                validate_sql_identifier(table_name, "migration.table_name")?;
                column.validate()
            }
        }
    }
}

#[derive(Serialize)]
struct PluginMigrationDigestInput<'a> {
    migration_id: &'a PluginMigrationId,
    actions: &'a [PluginAdditiveMigrationAction],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginMigration {
    pub migration_id: PluginMigrationId,
    pub migration_digest: DigestHex,
    pub actions: Vec<PluginAdditiveMigrationAction>,
}

impl PluginMigration {
    pub fn new(
        migration_id: PluginMigrationId,
        actions: Vec<PluginAdditiveMigrationAction>,
    ) -> Result<Self, PluginRuntimeContractError> {
        let migration_digest = digest_payload(&PluginMigrationDigestInput {
            migration_id: &migration_id,
            actions: &actions,
        })?;
        let value = Self {
            migration_id,
            migration_digest,
            actions,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_machine_key(self.migration_id.as_ref(), "migration_id")?;
        validate_digest(&self.migration_digest, "migration_digest")?;
        if self.actions.is_empty() {
            return Err(invalid(
                "migration.actions",
                "a migration requires at least one additive action",
            ));
        }
        for action in &self.actions {
            action.validate()?;
        }
        let expected = digest_payload(&PluginMigrationDigestInput {
            migration_id: &self.migration_id,
            actions: &self.actions,
        })?;
        if expected != self.migration_digest {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "migration_digest",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginResourceContract {
    pub required_resource_kinds: BTreeSet<ResourceKind>,
}

impl PluginResourceContract {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        for resource_kind in &self.required_resource_kinds {
            validate_machine_key(
                resource_kind.as_ref(),
                "resource_contract.required_resource_kinds",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReleaseV1Manifest {
    pub schema_version: VersionString,
    pub build_profile: JavaScriptBuildProfile,
    pub build_profile_version: VersionString,
    pub display: LocalizedMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<PluginUiReleaseDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<PluginServiceReleaseDescriptor>,
    pub dependency_lock_digest: DigestHex,
    pub dependency_graph_digest: DigestHex,
    pub config_schema: StrictJsonValue,
    pub config_schema_digest: DigestHex,
    pub credential_slots: Vec<CredentialSlotDeclaration>,
    pub credential_slots_digest: DigestHex,
    pub resource_contract: PluginResourceContract,
    pub resource_contract_digest: DigestHex,
    pub schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
    pub bridge_contract_digest: DigestHex,
    pub contribution_package: PackageRef,
    pub contributions: PackageContributions,
    pub migrations: Vec<PluginMigration>,
}

impl PluginReleaseV1Manifest {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        require_version(
            &self.schema_version,
            PLUGIN_RUNTIME_SCHEMA_VERSION,
            "schema_version",
        )?;
        if self.build_profile != JavaScriptBuildProfile::PluginReleaseV1 {
            return Err(invalid(
                "build_profile",
                "Plugin Product Release must use plugin_release_v1",
            ));
        }
        require_version(
            &self.build_profile_version,
            PLUGIN_RELEASE_PROFILE_VERSION,
            "build_profile_version",
        )?;
        validate_display(&self.display)?;
        if let Some(ui) = &self.ui {
            ui.validate()?;
        } else if self.service.is_none() {
            return Err(invalid("roles", "A plugin release must provide a surface or a service"));
        }
        if let Some(service) = &self.service {
            service.validate()?;
            if !self.migrations.is_empty() && !service.uses_private_database {
                return Err(invalid(
                    "migrations",
                    "Plugin Product migrations require the managed Private Database",
                ));
            }
        } else if !self.migrations.is_empty()
            || self.contributions.capabilities.iter().any(|capability| capability.kind != crate::CapabilityKind::UiContribution)
            || !self.contributions.mcp_tools.is_empty()
            || !self.contributions.role_contracts.is_empty()
            || !self.contributions.role_providers.is_empty()
            || !self.credential_slots.is_empty()
            || !self.resource_contract.required_resource_kinds.is_empty()
        {
            return Err(invalid(
                "service",
                "UI-only Plugins cannot declare Service migrations, executable contributions, Credential slots, or typed Resource requirements",
            ));
        }
        validate_digest(&self.dependency_lock_digest, "dependency_lock_digest")?;
        validate_digest(&self.dependency_graph_digest, "dependency_graph_digest")?;
        validate_config_schema(&self.config_schema)?;
        validate_digest(&self.config_schema_digest, "config_schema_digest")?;
        let expected_config_schema_digest = digest_payload(&self.config_schema.0)?;
        if expected_config_schema_digest != self.config_schema_digest {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "config_schema_digest",
            });
        }
        validate_credential_slots(&self.credential_slots)?;
        validate_digest(&self.credential_slots_digest, "credential_slots_digest")?;
        let expected_credential_slots_digest = digest_payload(&self.credential_slots)?;
        if expected_credential_slots_digest != self.credential_slots_digest {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "credential_slots_digest",
            });
        }
        self.resource_contract.validate()?;
        validate_digest(&self.resource_contract_digest, "resource_contract_digest")?;
        let expected_resource_contract_digest = digest_payload(&self.resource_contract)?;
        if expected_resource_contract_digest != self.resource_contract_digest {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "resource_contract_digest",
            });
        }
        validate_digest(&self.bridge_contract_digest, "bridge_contract_digest")?;
        validate_nonempty(
            self.contribution_package.id.as_ref(),
            "contribution_package.id",
        )?;
        validate_nonempty(
            self.contribution_package.version.as_ref(),
            "contribution_package.version",
        )?;
        validate_contributions(
            &self.contribution_package,
            &self.contributions,
        )?;
        if self.ui.is_none() && self.contributions.capabilities.iter().any(|capability| capability.contributions.ui_slot.is_some()) {
            return Err(invalid("contributions.ui_slot", "A UI contribution requires release UI bytes"));
        }
        validate_release_schema_registry(&self.contributions, &self.schemas)?;
        let contribution_resource_kinds = self
            .contributions
            .capabilities
            .iter()
            .flat_map(|capability| {
                capability
                    .contributions
                    .resource_kinds
                    .iter()
                    .cloned()
            })
            .collect::<BTreeSet<_>>();
        if !contribution_resource_kinds
            .is_subset(&self.resource_contract.required_resource_kinds)
        {
            let missing = contribution_resource_kinds
                .difference(&self.resource_contract.required_resource_kinds)
                .map(AsRef::as_ref)
                .collect::<Vec<_>>();
            return Err(invalid(
                "resource_contract.required_resource_kinds",
                format!(
                    "Resource contract is missing contribution requirements {missing:?}"
                ),
            ));
        }
        let mut migration_ids = BTreeSet::new();
        for migration in &self.migrations {
            migration.validate()?;
            if !migration_ids.insert(migration.migration_id.as_ref()) {
                return Err(duplicate(
                    "migration_id",
                    migration.migration_id.as_ref(),
                ));
            }
        }
        Ok(())
    }

    pub fn is_ui_only(&self) -> bool {
        self.service.is_none()
    }

    pub fn migration_set_digest(&self) -> Result<DigestHex, PluginRuntimeContractError> {
        Ok(digest_payload(&self.migrations)?)
    }

    pub fn contribution_set_digest(&self) -> Result<DigestHex, PluginRuntimeContractError> {
        Ok(digest_payload(&self.contributions)?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReleaseFile {
    pub normalized_relative_path: String,
    pub digest: DigestHex,
    pub size_bytes: u64,
}

impl PluginReleaseFile {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_relative_path(
            &self.normalized_relative_path,
            "files.normalized_relative_path",
        )?;
        validate_digest(&self.digest, "files.digest")
    }
}

#[derive(Serialize)]
struct PluginReleaseArtifactDigestInput<'a> {
    manifest: &'a PluginReleaseManifestArtifact,
    files: &'a [PluginReleaseFile],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReleaseArtifactV1 {
    pub artifact_id: ArtifactId,
    pub artifact_digest: DigestHex,
    pub manifest: PluginReleaseManifestArtifact,
    pub files: Vec<PluginReleaseFile>,
}

impl PluginReleaseArtifactV1 {
    pub fn new(
        artifact_id: ArtifactId,
        manifest: PluginReleaseV1Manifest,
        mut files: Vec<PluginReleaseFile>,
    ) -> Result<Self, PluginRuntimeContractError> {
        manifest.validate()?;
        files.sort_by(|left, right| {
            left.normalized_relative_path
                .cmp(&right.normalized_relative_path)
        });
        let manifest = ArtifactEnvelope::new(manifest)?;
        let artifact_digest = canonical_release_artifact_digest(&manifest, &files)?;
        let value = Self {
            artifact_id,
            artifact_digest,
            manifest,
            files,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.artifact_id.as_ref(), "artifact_id")?;
        validate_digest(&self.artifact_digest, "artifact_digest")?;
        if !self.manifest.verify()? {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "manifest.payload_digest",
            });
        }
        self.manifest.payload.validate()?;
        if self.files.is_empty() {
            return Err(invalid(
                "files",
                "Plugin Product Release must contain ui/index.html",
            ));
        }
        let mut previous: Option<&str> = None;
        let mut ui_entrypoint_digest = None;
        let mut service_digest = None;
        for file in &self.files {
            file.validate()?;
            let path = file.normalized_relative_path.as_str();
            if previous.is_some_and(|previous| previous >= path) {
                return Err(invalid(
                    "files",
                    "Release files must be sorted and unique",
                ));
            }
            if path.starts_with("ui/") {
                if path == "ui/index.html" {
                    if file.size_bytes == 0 {
                        return Err(invalid(
                            "files.ui/index.html",
                            "Plugin Product UI entrypoint must not be empty",
                        ));
                    }
                    ui_entrypoint_digest = Some(&file.digest);
                }
            } else if self.manifest.payload.service.as_ref().is_some_and(|service| path == service.entrypoint) {
                if file.size_bytes == 0 {
                    return Err(invalid(
                        "files.service.entrypoint",
                        "Plugin Product Service entrypoint must not be empty",
                    ));
                }
                service_digest = Some(&file.digest);
            } else {
                return Err(invalid(
                    "files.normalized_relative_path",
                    "plugin-release-v1 permits ui/** and the exact declared Service entrypoint only",
                ));
            }
            previous = Some(path);
        }
        if let Some(ui) = &self.manifest.payload.ui {
            if ui_entrypoint_digest != Some(&ui.entrypoint_digest) {
                return Err(PluginRuntimeContractError::DigestMismatch { field: "ui.entrypoint_digest" });
            }
            if canonical_ui_tree_digest(&self.files)? != ui.ui_tree_digest {
                return Err(PluginRuntimeContractError::DigestMismatch { field: "ui.ui_tree_digest" });
            }
        } else if self.files.iter().any(|file| file.normalized_relative_path.starts_with("ui/")) {
            return Err(invalid("files", "UI files require a declared plugin surface"));
        }
        match (&self.manifest.payload.service, service_digest) {
            (None, None) => {}
            (Some(service), Some(observed)) if &service.module_digest == observed => {}
            (Some(_), None) => {
                return Err(invalid(
                    "files",
                    "Service manifest requires its exact declared entrypoint",
                ));
            }
            (None, Some(_)) => {
                return Err(invalid(
                    "files",
                    "UI-only manifest cannot carry a Service entrypoint",
                ));
            }
            (Some(_), Some(_)) => {
                return Err(PluginRuntimeContractError::DigestMismatch {
                    field: "service.module_digest",
                });
            }
        }
        let expected = canonical_release_artifact_digest(&self.manifest, &self.files)?;
        if expected != self.artifact_digest {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "artifact_digest",
            });
        }
        Ok(())
    }
}

pub fn canonical_release_artifact_digest(
    manifest: &PluginReleaseManifestArtifact,
    files: &[PluginReleaseFile],
) -> Result<DigestHex, PluginRuntimeContractError> {
    Ok(digest_payload(&PluginReleaseArtifactDigestInput {
        manifest,
        files,
    })?)
}

pub fn canonical_ui_tree_digest(
    files: &[PluginReleaseFile],
) -> Result<DigestHex, PluginRuntimeContractError> {
    let mut ui_files = files
        .iter()
        .filter(|file| file.normalized_relative_path.starts_with("ui/"))
        .collect::<Vec<_>>();
    ui_files.sort_by(|left, right| {
        left.normalized_relative_path
            .cmp(&right.normalized_relative_path)
    });
    Ok(digest_payload(&ui_files)?)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReleaseRef {
    pub release_id: PluginReleaseId,
    pub artifact_id: ArtifactId,
    pub release_digest: DigestHex,
    pub manifest_digest: DigestHex,
}

impl PluginReleaseRef {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.release_id.as_ref(), "release_id")?;
        validate_nonempty(self.artifact_id.as_ref(), "artifact_id")?;
        validate_digest(&self.release_digest, "release_digest")?;
        validate_digest(&self.manifest_digest, "manifest_digest")
    }

    pub fn validate_artifact(
        &self,
        artifact: &PluginReleaseArtifactV1,
    ) -> Result<(), PluginRuntimeContractError> {
        self.validate()?;
        artifact.validate()?;
        if self.artifact_id != artifact.artifact_id
            || self.release_digest != artifact.artifact_digest
            || self.manifest_digest != artifact.manifest.payload_digest
        {
            return Err(invalid(
                "release",
                "Release ref must bind the exact Artifact and Manifest digests",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginReleaseSourceLineage {
    Managed {
        project_id: PluginProjectId,
        source_snapshot_digest: DigestHex,
        dependency_lock_digest: DigestHex,
        build_profile_version: VersionString,
        build_generation: u64,
    },
    RuntimeOnly,
}

impl PluginReleaseSourceLineage {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        match self {
            Self::Managed {
                project_id,
                source_snapshot_digest,
                dependency_lock_digest,
                build_profile_version,
                build_generation,
            } => {
                validate_nonempty(project_id.as_ref(), "project_id")?;
                validate_digest(source_snapshot_digest, "source_snapshot_digest")?;
                validate_digest(dependency_lock_digest, "dependency_lock_digest")?;
                require_version(
                    build_profile_version,
                    PLUGIN_RELEASE_PROFILE_VERSION,
                    "build_profile_version",
                )?;
                if *build_generation == 0 {
                    return Err(invalid(
                        "build_generation",
                        "managed source requires a positive build generation",
                    ));
                }
                Ok(())
            }
            Self::RuntimeOnly => Ok(()),
        }
    }

    pub fn is_managed(&self) -> bool {
        matches!(self, Self::Managed { .. })
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginReadyOrigin {
    Build,
    Import,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceTestReceiptRef {
    pub receipt_id: PluginServiceTestReceiptId,
    pub release_id: PluginReleaseId,
    pub release_digest: DigestHex,
    pub service_run_key: DigestHex,
}

impl PluginServiceTestReceiptRef {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.receipt_id.as_ref(), "test_receipt.receipt_id")?;
        validate_nonempty(self.release_id.as_ref(), "test_receipt.release_id")?;
        validate_digest(&self.release_digest, "test_receipt.release_digest")?;
        validate_digest(&self.service_run_key, "test_receipt.service_run_key")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReadyRelease {
    pub plugin_product_id: PluginProductId,
    pub release: PluginReleaseRef,
    pub origin_operation_id: OperationId,
    pub origin: PluginReadyOrigin,
    pub source_lineage: PluginReleaseSourceLineage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_service_test_receipt: Option<PluginServiceTestReceiptRef>,
    pub created_at_ms: i64,
}

impl PluginReadyRelease {
    pub fn validate_for_artifact(
        &self,
        artifact: &PluginReleaseArtifactV1,
    ) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.plugin_product_id.as_ref(), "plugin_product_id")?;
        validate_nonempty(
            self.origin_operation_id.as_ref(),
            "origin_operation_id",
        )?;
        self.release.validate_artifact(artifact)?;
        self.source_lineage.validate()?;
        if self.origin == PluginReadyOrigin::Build && !self.source_lineage.is_managed() {
            return Err(invalid(
                "source_lineage",
                "Build Ready Releases require managed source lineage",
            ));
        }
        if let PluginReleaseSourceLineage::Managed {
            dependency_lock_digest,
            ..
        } = &self.source_lineage
            && dependency_lock_digest != &artifact.manifest.payload.dependency_lock_digest
        {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "dependency_lock_digest",
            });
        }
        if let Some(receipt) = &self.matching_service_test_receipt {
            receipt.validate()?;
            if receipt.release_id != self.release.release_id
                || receipt.release_digest != self.release.release_digest
            {
                return Err(invalid(
                    "matching_service_test_receipt",
                    "Service Test receipt must bind the exact Ready Release",
                ));
            }
            if artifact.manifest.payload.service.is_none() {
                return Err(invalid(
                    "matching_service_test_receipt",
                    "UI-only Ready Releases cannot carry a Service Test receipt",
                ));
            }
        }
        if self.created_at_ms <= 0 {
            return Err(invalid(
                "created_at_ms",
                "Ready Release time must be positive",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReadyReleaseRef {
    pub release_id: PluginReleaseId,
    pub release_digest: DigestHex,
}

impl PluginReadyReleaseRef {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.release_id.as_ref(), "ready_release.release_id")?;
        validate_digest(&self.release_digest, "ready_release.release_digest")
    }
}

impl From<&PluginReadyRelease> for PluginReadyReleaseRef {
    fn from(value: &PluginReadyRelease) -> Self {
        Self {
            release_id: value.release.release_id.clone(),
            release_digest: value.release.release_digest.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginReleasePointerState {
    pub plugin_product_id: PluginProductId,
    pub pointer_revision: u64,
    pub active_release_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_release: Option<PluginReadyReleaseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_release: Option<PluginReleaseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_release: Option<PluginReleaseRef>,
    pub materialized_catalog_digest: DigestHex,
}

impl PluginReleasePointerState {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.plugin_product_id.as_ref(), "plugin_product_id")?;
        if self.pointer_revision == 0 {
            return Err(invalid(
                "pointer_revision",
                "pointer revision must be positive",
            ));
        }
        if self.active_release.is_some() != (self.active_release_epoch > 0) {
            return Err(invalid(
                "active_release_epoch",
                "active Release and positive epoch must appear together",
            ));
        }
        if let Some(ready) = &self.ready_release {
            ready.validate()?;
        }
        if let Some(active) = &self.active_release {
            active.validate()?;
        }
        if let Some(previous) = &self.previous_release {
            previous.validate()?;
        }
        if self.active_release.as_ref().is_some_and(|active| {
            self.previous_release
                .as_ref()
                .is_some_and(|previous| previous.release_id == active.release_id)
                || self
                    .ready_release
                    .as_ref()
                    .is_some_and(|ready| ready.release_id == active.release_id)
        }) {
            return Err(invalid(
                "release_pointers",
                "Ready, Active, and Previous must not point to the same Release identity",
            ));
        }
        if self.previous_release.as_ref().is_some_and(|previous| {
            self.ready_release
                .as_ref()
                .is_some_and(|ready| ready.release_id == previous.release_id)
        }) {
            return Err(invalid(
                "release_pointers",
                "Ready, Active, and Previous must not point to the same Release identity",
            ));
        }
        validate_digest(
            &self.materialized_catalog_digest,
            "materialized_catalog_digest",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginPointerExpectation {
    pub pointer_revision: u64,
    pub active_release_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_release: Option<PluginReadyReleaseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_release: Option<PluginReleaseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_release: Option<PluginReleaseRef>,
    pub materialized_catalog_digest: DigestHex,
}

impl PluginPointerExpectation {
    pub fn from_state(state: &PluginReleasePointerState) -> Self {
        Self {
            pointer_revision: state.pointer_revision,
            active_release_epoch: state.active_release_epoch,
            ready_release: state.ready_release.clone(),
            active_release: state.active_release.clone(),
            previous_release: state.previous_release.clone(),
            materialized_catalog_digest: state.materialized_catalog_digest.clone(),
        }
    }

    fn matches(&self, state: &PluginReleasePointerState) -> bool {
        self.pointer_revision == state.pointer_revision
            && self.active_release_epoch == state.active_release_epoch
            && self.ready_release == state.ready_release
            && self.active_release == state.active_release
            && self.previous_release == state.previous_release
            && self.materialized_catalog_digest == state.materialized_catalog_digest
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginNonUiReleaseFingerprint {
    pub manifest_without_ui_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_run_key: Option<DigestHex>,
    pub migration_set_digest: DigestHex,
    pub contribution_set_digest: DigestHex,
    pub bridge_contract_digest: DigestHex,
    pub config_schema_digest: DigestHex,
    pub credential_slots_digest: DigestHex,
    pub resource_contract_digest: DigestHex,
    pub runtime_requirements_digest: DigestHex,
    pub dependency_lock_digest: DigestHex,
}

impl PluginNonUiReleaseFingerprint {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        if let Some(service_run_key) = &self.service_run_key {
            validate_digest(service_run_key, "service_run_key")?;
        }
        for (field, digest) in [
            (
                "manifest_without_ui_digest",
                &self.manifest_without_ui_digest,
            ),
            ("migration_set_digest", &self.migration_set_digest),
            ("contribution_set_digest", &self.contribution_set_digest),
            ("bridge_contract_digest", &self.bridge_contract_digest),
            ("config_schema_digest", &self.config_schema_digest),
            ("credential_slots_digest", &self.credential_slots_digest),
            ("resource_contract_digest", &self.resource_contract_digest),
            (
                "runtime_requirements_digest",
                &self.runtime_requirements_digest,
            ),
            ("dependency_lock_digest", &self.dependency_lock_digest),
        ] {
            validate_digest(digest, field)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginUiOnlyAutoPublishAuthorization {
    pub authorization_id: PluginUserAuthorizationId,
    pub plugin_product_id: PluginProductId,
    pub enabled: bool,
    pub authorization_revision: u64,
    pub user_authorized_at_ms: i64,
}

impl PluginUiOnlyAutoPublishAuthorization {
    fn validate_for(&self, plugin_product_id: &PluginProductId) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.authorization_id.as_ref(), "authorization_id")?;
        if &self.plugin_product_id != plugin_product_id || !self.enabled {
            return Err(invalid(
                "authorization",
                "auto Publish requires enabled user authorization for the exact Plugin Product",
            ));
        }
        if self.authorization_revision == 0 || self.user_authorized_at_ms <= 0 {
            return Err(invalid(
                "authorization",
                "authorization revision and time must be positive",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginUiOnlyAutoPublishProof {
    pub current_release: PluginReleaseRef,
    pub target_release: PluginReleaseRef,
    pub current_ui_tree_digest: DigestHex,
    pub target_ui_tree_digest: DigestHex,
    pub current_non_ui: PluginNonUiReleaseFingerprint,
    pub target_non_ui: PluginNonUiReleaseFingerprint,
    pub changed_source_paths: BTreeSet<String>,
    pub changed_output_paths: BTreeSet<String>,
    pub project_head_matches_ready_source: bool,
    pub static_validation_passed: bool,
    pub no_unknown_changes: bool,
}

impl PluginUiOnlyAutoPublishProof {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        self.current_release.validate()?;
        self.target_release.validate()?;
        validate_digest(&self.current_ui_tree_digest, "current_ui_tree_digest")?;
        validate_digest(&self.target_ui_tree_digest, "target_ui_tree_digest")?;
        self.current_non_ui.validate()?;
        self.target_non_ui.validate()?;
        if self.current_release.release_id == self.target_release.release_id
            || self.current_ui_tree_digest == self.target_ui_tree_digest
        {
            return Err(invalid(
                "auto_publish_change",
                "auto Publish requires a real UI Release change",
            ));
        }
        if self.current_non_ui != self.target_non_ui {
            return Err(invalid(
                "auto_publish_change",
                "Service, Migration, Catalog, Bridge, Config, Credential, Resource, Runtime, and dependency inputs must be unchanged",
            ));
        }
        if self.changed_source_paths.is_empty() || self.changed_output_paths.is_empty() {
            return Err(invalid(
                "auto_publish_change",
                "strict UI-only proof requires source and output path evidence",
            ));
        }
        for path in &self.changed_source_paths {
            validate_relative_path(path, "changed_source_paths")?;
            if !path.starts_with("ui/") {
                return Err(invalid(
                    "changed_source_paths",
                    "auto Publish permits UI source changes only",
                ));
            }
        }
        for path in &self.changed_output_paths {
            validate_relative_path(path, "changed_output_paths")?;
            if !path.starts_with("ui/") {
                return Err(invalid(
                    "changed_output_paths",
                    "auto Publish permits ui/** output changes only",
                ));
            }
        }
        if !self.project_head_matches_ready_source
            || !self.static_validation_passed
            || !self.no_unknown_changes
        {
            return Err(invalid(
                "auto_publish_proof",
                "source head, validator, and complete known-diff proofs are required",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginPublishAuthorization {
    ManualUser { actor_id: String },
    AutoUiOnly {
        authorization: PluginUiOnlyAutoPublishAuthorization,
        proof: Box<PluginUiOnlyAutoPublishProof>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginPublishRequest {
    pub plugin_product_id: PluginProductId,
    pub expected: PluginPointerExpectation,
    pub target_ready_release: PluginReleaseRef,
    pub target_catalog_digest: DigestHex,
    pub authorization: PluginPublishAuthorization,
}

impl PluginPublishRequest {
    pub fn validate_for(
        &self,
        state: &PluginReleasePointerState,
    ) -> Result<(), PluginRuntimeContractError> {
        state.validate()?;
        if self.plugin_product_id != state.plugin_product_id || !self.expected.matches(state) {
            return Err(PluginRuntimeContractError::CompareAndSwapConflict);
        }
        self.target_ready_release.validate()?;
        validate_digest(&self.target_catalog_digest, "target_catalog_digest")?;
        let target_ready = PluginReadyReleaseRef {
            release_id: self.target_ready_release.release_id.clone(),
            release_digest: self.target_ready_release.release_digest.clone(),
        };
        if state.ready_release.as_ref() != Some(&target_ready) {
            return Err(invalid(
                "target_ready_release",
                "Publish target must be the exact current Ready Release",
            ));
        }
        match &self.authorization {
            PluginPublishAuthorization::ManualUser { actor_id } => {
                validate_nonempty(actor_id, "authorization.actor_id")
            }
            PluginPublishAuthorization::AutoUiOnly {
                authorization,
                proof,
            } => {
                authorization.validate_for(&self.plugin_product_id)?;
                proof.validate()?;
                let active = state.active_release.as_ref().ok_or_else(|| {
                    invalid(
                        "active_release",
                        "first Publish is always manual",
                    )
                })?;
                if &proof.current_release != active
                    || proof.target_release != self.target_ready_release
                {
                    return Err(invalid(
                        "auto_publish_proof",
                        "proof must bind the exact Active and Ready Releases",
                    ));
                }
                Ok(())
            }
        }
    }

    pub fn next_state(
        &self,
        state: &PluginReleasePointerState,
    ) -> Result<PluginReleasePointerState, PluginRuntimeContractError> {
        self.validate_for(state)?;
        Ok(PluginReleasePointerState {
            plugin_product_id: state.plugin_product_id.clone(),
            pointer_revision: state
                .pointer_revision
                .checked_add(1)
                .ok_or_else(|| invalid("pointer_revision", "pointer revision overflow"))?,
            active_release_epoch: state
                .active_release_epoch
                .checked_add(1)
                .ok_or_else(|| invalid("active_release_epoch", "Release epoch overflow"))?,
            ready_release: None,
            active_release: Some(self.target_ready_release.clone()),
            previous_release: state.active_release.clone(),
            materialized_catalog_digest: self.target_catalog_digest.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginRollbackRequest {
    pub plugin_product_id: PluginProductId,
    pub expected: PluginPointerExpectation,
    pub rollback_target: PluginReleaseRef,
    pub target_catalog_digest: DigestHex,
    pub actor_id: String,
}

impl PluginRollbackRequest {
    pub fn validate_for(
        &self,
        state: &PluginReleasePointerState,
    ) -> Result<(), PluginRuntimeContractError> {
        state.validate()?;
        if self.plugin_product_id != state.plugin_product_id || !self.expected.matches(state) {
            return Err(PluginRuntimeContractError::CompareAndSwapConflict);
        }
        validate_nonempty(&self.actor_id, "actor_id")?;
        self.rollback_target.validate()?;
        validate_digest(&self.target_catalog_digest, "target_catalog_digest")?;
        if state.previous_release.as_ref() != Some(&self.rollback_target) {
            return Err(invalid(
                "rollback_target",
                "Rollback target must be the exact current Previous Release",
            ));
        }
        if state.active_release.is_none() {
            return Err(invalid(
                "active_release",
                "Rollback requires an Active Release",
            ));
        }
        Ok(())
    }

    pub fn next_state(
        &self,
        state: &PluginReleasePointerState,
    ) -> Result<PluginReleasePointerState, PluginRuntimeContractError> {
        self.validate_for(state)?;
        Ok(PluginReleasePointerState {
            plugin_product_id: state.plugin_product_id.clone(),
            pointer_revision: state
                .pointer_revision
                .checked_add(1)
                .ok_or_else(|| invalid("pointer_revision", "pointer revision overflow"))?,
            active_release_epoch: state
                .active_release_epoch
                .checked_add(1)
                .ok_or_else(|| invalid("active_release_epoch", "Release epoch overflow"))?,
            ready_release: state.ready_release.clone(),
            active_release: Some(self.rollback_target.clone()),
            previous_release: state.active_release.clone(),
            materialized_catalog_digest: self.target_catalog_digest.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginPointerCasCommit {
    pub plugin_product_id: PluginProductId,
    pub before: PluginPointerExpectation,
    pub after: PluginReleasePointerState,
    pub committed_at_ms: i64,
}

impl PluginPointerCasCommit {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        self.after.validate()?;
        let expected_revision = self
            .before
            .pointer_revision
            .checked_add(1)
            .ok_or_else(|| invalid("before.pointer_revision", "pointer revision overflow"))?;
        let expected_epoch = self
            .before
            .active_release_epoch
            .checked_add(1)
            .ok_or_else(|| invalid("before.active_release_epoch", "Release epoch overflow"))?;
        if self.plugin_product_id != self.after.plugin_product_id
            || self.after.pointer_revision != expected_revision
            || self.after.active_release_epoch != expected_epoch
            || self.committed_at_ms <= 0
        {
            return Err(invalid(
                "pointer_commit",
                "commit must advance the exact Plugin Product pointer revision and active epoch once",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginPointerCasOutcome {
    Committed {
        commit: Box<PluginPointerCasCommit>,
    },
    Conflict {
        observed_pointer_revision: u64,
        observed_active_release_epoch: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginKvHandleDescriptor {
    pub handle_id: PluginKvHandleId,
    pub plugin_product_id: PluginProductId,
    pub namespace_revision: u64,
}

impl PluginKvHandleDescriptor {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.handle_id.as_ref(), "kv.handle_id")?;
        validate_nonempty(self.plugin_product_id.as_ref(), "kv.plugin_product_id")?;
        if self.namespace_revision == 0 {
            return Err(invalid(
                "kv.namespace_revision",
                "KV namespace revision must be positive",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginKvRequest {
    Get {
        handle: PluginKvHandleDescriptor,
        key: String,
    },
    Set {
        handle: PluginKvHandleDescriptor,
        key: String,
        value: StrictJsonValue,
    },
    Delete {
        handle: PluginKvHandleDescriptor,
        key: String,
    },
    CompareAndSwap {
        handle: PluginKvHandleDescriptor,
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected_revision: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<StrictJsonValue>,
    },
}

impl PluginKvRequest {
    pub fn validate_for(&self, plugin_product_id: &PluginProductId) -> Result<(), PluginRuntimeContractError> {
        let (handle, key) = match self {
            Self::Get { handle, key }
            | Self::Set { handle, key, .. }
            | Self::Delete { handle, key }
            | Self::CompareAndSwap { handle, key, .. } => (handle, key),
        };
        handle.validate()?;
        if &handle.plugin_product_id != plugin_product_id {
            return Err(invalid(
                "kv.plugin_product_id",
                "Host KV handle belongs to another Plugin Product",
            ));
        }
        validate_state_key(key, "kv.key")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginKvResponse {
    Value {
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<StrictJsonValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        revision: Option<u64>,
    },
    Written {
        revision: u64,
    },
    Deleted {
        existed: bool,
    },
    CompareAndSwap {
        applied: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_revision: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginFilesDirDescriptor {
    pub handle_id: PluginFilesHandleId,
    pub plugin_product_id: PluginProductId,
    pub absolute_path: String,
}

impl PluginFilesDirDescriptor {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.handle_id.as_ref(), "files_dir.handle_id")?;
        validate_nonempty(self.plugin_product_id.as_ref(), "files_dir.plugin_product_id")?;
        validate_absolute_path(&self.absolute_path, "files_dir.absolute_path")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginPrivateDatabaseDescriptor {
    pub handle_id: PluginDatabaseHandleId,
    pub plugin_product_id: PluginProductId,
    pub schema_epoch: u64,
    pub migration_ledger_digest: DigestHex,
}

impl PluginPrivateDatabaseDescriptor {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.handle_id.as_ref(), "database.handle_id")?;
        validate_nonempty(self.plugin_product_id.as_ref(), "database.plugin_product_id")?;
        if self.schema_epoch == 0 {
            return Err(invalid(
                "database.schema_epoch",
                "Private Database schema epoch must be positive",
            ));
        }
        validate_digest(
            &self.migration_ledger_digest,
            "database.migration_ledger_digest",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceStorageDescriptor {
    pub kv: PluginKvHandleDescriptor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_dir: Option<PluginFilesDirDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_database: Option<PluginPrivateDatabaseDescriptor>,
}

impl PluginServiceStorageDescriptor {
    fn validate_for(&self, plugin_product_id: &PluginProductId) -> Result<(), PluginRuntimeContractError> {
        self.kv.validate()?;
        if let Some(files_dir) = &self.files_dir {
            files_dir.validate()?;
        }
        if let Some(private_database) = &self.private_database {
            private_database.validate()?;
        }
        if &self.kv.plugin_product_id != plugin_product_id
            || self
                .files_dir
                .as_ref()
                .is_some_and(|descriptor| &descriptor.plugin_product_id != plugin_product_id)
            || self
                .private_database
                .as_ref()
                .is_some_and(|descriptor| &descriptor.plugin_product_id != plugin_product_id)
        {
            return Err(invalid(
                "storage.plugin_product_id",
                "all Service storage handles must belong to the exact Plugin Product",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum PluginServiceRuntimeFingerprint {
    Node {
        runtime_installation_id: RuntimeInstallationId,
        runtime_target: RuntimeTarget,
        runtime_executable_digest: DigestHex,
        node_version: VersionString,
    },
    Native {
        native_target: NativePluginTarget,
        native_executable_digest: DigestHex,
    },
}

impl PluginServiceRuntimeFingerprint {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        let Self::Node { runtime_installation_id, runtime_target, runtime_executable_digest, node_version } = self else {
            return validate_digest(self.executable_digest(), "runtime.native_executable_digest");
        };
        validate_nonempty(
            runtime_installation_id.as_ref(),
            "runtime.runtime_installation_id",
        )?;
        validate_nonempty(runtime_target.as_ref(), "runtime.runtime_target")?;
        validate_digest(
            runtime_executable_digest,
            "runtime.runtime_executable_digest",
        )?;
        validate_nonempty(node_version.as_ref(), "runtime.node_version")
    }

    pub fn executable_digest(&self) -> &DigestHex {
        match self {
            Self::Node { runtime_executable_digest, .. } => runtime_executable_digest,
            Self::Native { native_executable_digest, .. } => native_executable_digest,
        }
    }

    pub fn target(&self) -> RuntimeTarget {
        match self {
            Self::Node { runtime_target, .. } => runtime_target.clone(),
            Self::Native { native_target, .. } => native_target.as_str().into(),
        }
    }
}

#[derive(Serialize)]
struct ResolvedPluginServiceSpecDigestInput<'a> {
    plugin_product_id: &'a PluginProductId,
    service_module_digest: &'a DigestHex,
    lifecycle: PluginServiceLifecycle,
    host_protocol_version: &'a VersionString,
    sdk_contract_version: &'a VersionString,
    runtime: &'a PluginServiceRuntimeFingerprint,
    config_schema_digest: &'a DigestHex,
    config_snapshot_digest: &'a DigestHex,
    credential_slots_digest: &'a DigestHex,
    resource_contract_digest: &'a DigestHex,
    resource_bindings_digest: &'a DigestHex,
    runtime_requirements_digest: &'a DigestHex,
    bridge_contract_digest: &'a DigestHex,
    contribution_set_digest: &'a DigestHex,
    storage: &'a PluginServiceStorageDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPluginServiceSpecInputs {
    pub plugin_product_id: PluginProductId,
    pub release: PluginReleaseRef,
    pub active_release_epoch: u64,
    pub service_module_digest: DigestHex,
    pub lifecycle: PluginServiceLifecycle,
    pub host_protocol_version: VersionString,
    pub sdk_contract_version: VersionString,
    pub runtime: PluginServiceRuntimeFingerprint,
    pub config_schema_digest: DigestHex,
    pub config_snapshot_digest: DigestHex,
    pub credential_slots_digest: DigestHex,
    pub resource_contract_digest: DigestHex,
    pub resource_bindings_digest: DigestHex,
    pub runtime_requirements_digest: DigestHex,
    pub bridge_contract_digest: DigestHex,
    pub contribution_set_digest: DigestHex,
    pub storage: PluginServiceStorageDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPluginServiceSpec {
    pub plugin_product_id: PluginProductId,
    pub release: PluginReleaseRef,
    pub active_release_epoch: u64,
    pub service_module_digest: DigestHex,
    pub lifecycle: PluginServiceLifecycle,
    pub host_protocol_version: VersionString,
    pub sdk_contract_version: VersionString,
    pub runtime: PluginServiceRuntimeFingerprint,
    pub config_schema_digest: DigestHex,
    pub config_snapshot_digest: DigestHex,
    pub credential_slots_digest: DigestHex,
    pub resource_contract_digest: DigestHex,
    pub resource_bindings_digest: DigestHex,
    pub runtime_requirements_digest: DigestHex,
    pub bridge_contract_digest: DigestHex,
    pub contribution_set_digest: DigestHex,
    pub storage: PluginServiceStorageDescriptor,
    pub service_run_key: DigestHex,
}

impl ResolvedPluginServiceSpec {
    pub fn new(
        inputs: ResolvedPluginServiceSpecInputs,
    ) -> Result<Self, PluginRuntimeContractError> {
        let service_run_key = canonical_service_run_key(
            &inputs.plugin_product_id,
            &inputs.service_module_digest,
            inputs.lifecycle,
            &inputs.host_protocol_version,
            &inputs.sdk_contract_version,
            &inputs.runtime,
            &inputs.config_schema_digest,
            &inputs.config_snapshot_digest,
            &inputs.credential_slots_digest,
            &inputs.resource_contract_digest,
            &inputs.resource_bindings_digest,
            &inputs.runtime_requirements_digest,
            &inputs.bridge_contract_digest,
            &inputs.contribution_set_digest,
            &inputs.storage,
        )?;
        let ResolvedPluginServiceSpecInputs {
            plugin_product_id,
            release,
            active_release_epoch,
            service_module_digest,
            lifecycle,
            host_protocol_version,
            sdk_contract_version,
            runtime,
            config_schema_digest,
            config_snapshot_digest,
            credential_slots_digest,
            resource_contract_digest,
            resource_bindings_digest,
            runtime_requirements_digest,
            bridge_contract_digest,
            contribution_set_digest,
            storage,
        } = inputs;
        let value = Self {
            plugin_product_id,
            release,
            active_release_epoch,
            service_module_digest,
            lifecycle,
            host_protocol_version,
            sdk_contract_version,
            runtime,
            config_schema_digest,
            config_snapshot_digest,
            credential_slots_digest,
            resource_contract_digest,
            resource_bindings_digest,
            runtime_requirements_digest,
            bridge_contract_digest,
            contribution_set_digest,
            storage,
            service_run_key,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.plugin_product_id.as_ref(), "plugin_product_id")?;
        self.release.validate()?;
        if self.active_release_epoch == 0 {
            return Err(invalid(
                "active_release_epoch",
                "running Service requires a positive Active Release epoch",
            ));
        }
        validate_digest(&self.service_module_digest, "service_module_digest")?;
        require_version(
            &self.host_protocol_version,
            PLUGIN_SERVICE_HOST_PROTOCOL_VERSION,
            "host_protocol_version",
        )?;
        require_version(
            &self.sdk_contract_version,
            PLUGIN_SERVICE_SDK_CONTRACT_VERSION,
            "sdk_contract_version",
        )?;
        self.runtime.validate()?;
        if matches!(&self.runtime, PluginServiceRuntimeFingerprint::Native { native_executable_digest, .. } if native_executable_digest != &self.service_module_digest) {
            return Err(invalid("runtime.native_executable_digest", "Native runtime must be the exact release executable"));
        }
        validate_digest(&self.config_schema_digest, "config_schema_digest")?;
        validate_digest(&self.config_snapshot_digest, "config_snapshot_digest")?;
        validate_digest(
            &self.credential_slots_digest,
            "credential_slots_digest",
        )?;
        validate_digest(
            &self.resource_contract_digest,
            "resource_contract_digest",
        )?;
        validate_digest(
            &self.resource_bindings_digest,
            "resource_bindings_digest",
        )?;
        validate_digest(
            &self.runtime_requirements_digest,
            "runtime_requirements_digest",
        )?;
        validate_digest(&self.bridge_contract_digest, "bridge_contract_digest")?;
        validate_digest(&self.contribution_set_digest, "contribution_set_digest")?;
        self.storage.validate_for(&self.plugin_product_id)?;
        validate_digest(&self.service_run_key, "service_run_key")?;
        let expected = canonical_service_run_key(
            &self.plugin_product_id,
            &self.service_module_digest,
            self.lifecycle,
            &self.host_protocol_version,
            &self.sdk_contract_version,
            &self.runtime,
            &self.config_schema_digest,
            &self.config_snapshot_digest,
            &self.credential_slots_digest,
            &self.resource_contract_digest,
            &self.resource_bindings_digest,
            &self.runtime_requirements_digest,
            &self.bridge_contract_digest,
            &self.contribution_set_digest,
            &self.storage,
        )?;
        if expected != self.service_run_key {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "service_run_key",
            });
        }
        Ok(())
    }

    pub fn validate_for_release(
        &self,
        manifest: &PluginReleaseV1Manifest,
    ) -> Result<(), PluginRuntimeContractError> {
        self.validate()?;
        manifest.validate()?;
        let service = manifest.service.as_ref().ok_or_else(|| {
            invalid(
                "service",
                "UI-only Release cannot produce a Service specification",
            )
        })?;
        if self.service_module_digest != service.module_digest
            || self.lifecycle != service.lifecycle
            || self.host_protocol_version != service.host_protocol_version
            || self.sdk_contract_version != service.sdk_contract_version
            || self.config_schema_digest != manifest.config_schema_digest
            || self.credential_slots_digest != manifest.credential_slots_digest
            || self.resource_contract_digest != manifest.resource_contract_digest
            || self.runtime_requirements_digest != service.runtime_requirements_digest
            || self.bridge_contract_digest != manifest.bridge_contract_digest
            || self.contribution_set_digest != manifest.contribution_set_digest()?
            || self.storage.files_dir.is_some() != service.uses_files
            || self.storage.private_database.is_some()
                != service.uses_private_database
        {
            return Err(invalid(
                "resolved_service_spec",
                "Service specification does not match the exact Release contract",
            ));
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn canonical_service_run_key(
    plugin_product_id: &PluginProductId,
    service_module_digest: &DigestHex,
    lifecycle: PluginServiceLifecycle,
    host_protocol_version: &VersionString,
    sdk_contract_version: &VersionString,
    runtime: &PluginServiceRuntimeFingerprint,
    config_schema_digest: &DigestHex,
    config_snapshot_digest: &DigestHex,
    credential_slots_digest: &DigestHex,
    resource_contract_digest: &DigestHex,
    resource_bindings_digest: &DigestHex,
    runtime_requirements_digest: &DigestHex,
    bridge_contract_digest: &DigestHex,
    contribution_set_digest: &DigestHex,
    storage: &PluginServiceStorageDescriptor,
) -> Result<DigestHex, PluginRuntimeContractError> {
    Ok(digest_payload(&ResolvedPluginServiceSpecDigestInput {
        plugin_product_id,
        service_module_digest,
        lifecycle,
        host_protocol_version,
        sdk_contract_version,
        runtime,
        config_schema_digest,
        config_snapshot_digest,
        credential_slots_digest,
        resource_contract_digest,
        resource_bindings_digest,
        runtime_requirements_digest,
        bridge_contract_digest,
        contribution_set_digest,
        storage,
    })?)
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginBridgeTransport {
    MessageChannelV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginBridgeSession {
    pub bridge_contract_version: VersionString,
    pub bridge_session_id: PluginBridgeSessionId,
    pub surface_session_id: PluginSurfaceSessionId,
    pub plugin_product_id: PluginProductId,
    pub active_release: PluginReleaseRef,
    pub active_release_epoch: u64,
    pub transport: PluginBridgeTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_run_key: Option<DigestHex>,
}

impl PluginBridgeSession {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        require_version(
            &self.bridge_contract_version,
            PLUGIN_BRIDGE_CONTRACT_VERSION,
            "bridge_contract_version",
        )?;
        validate_nonempty(self.bridge_session_id.as_ref(), "bridge_session_id")?;
        validate_nonempty(self.surface_session_id.as_ref(), "surface_session_id")?;
        validate_nonempty(self.plugin_product_id.as_ref(), "plugin_product_id")?;
        self.active_release.validate()?;
        if self.active_release_epoch == 0 {
            return Err(invalid(
                "active_release_epoch",
                "Bridge sessions require a positive Active Release epoch",
            ));
        }
        if let Some(service_run_key) = &self.service_run_key {
            validate_digest(service_run_key, "service_run_key")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "target", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginBridgeTarget {
    /// A host-selected Session, never an arbitrary Session ID supplied by the frame.
    AgentSession {
        request: PluginAgentSessionRequest,
    },
    HostKv {
        request: PluginBridgeKvRequest,
    },
    Service {
        method: String,
        payload: StrictJsonValue,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginBridgeKvRequest {
    Get {
        key: String,
    },
    Set {
        key: String,
        value: StrictJsonValue,
    },
    Delete {
        key: String,
    },
    CompareAndSwap {
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected_revision: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<StrictJsonValue>,
    },
}

/// Public UI commands. The Surface grant supplies identity and scope separately.
/// Observe is a durable message projection, not a replayable token event stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginAgentSessionRequest {
    Observe { after_seq: u64, limit: u32 },
    Turn { input: StrictJsonValue, idempotency_key: String },
    // Empty struct variant keeps serde's unknown-field rejection active.
    Cancel {},
}

impl PluginAgentSessionRequest {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        match self {
            Self::Observe { limit, .. } if !(1..=200).contains(limit) => {
                Err(invalid("limit", "Session page limit must be between 1 and 200"))
            }
            Self::Turn { input, idempotency_key } => {
                validate_nonempty(idempotency_key, "idempotency_key")?;
                if idempotency_key.len() > 256 || !input.0.is_object() {
                    return Err(invalid("input", "Session turn requires an object and a key of at most 256 bytes"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

impl PluginBridgeKvRequest {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        let key = match self {
            Self::Get { key }
            | Self::Set { key, .. }
            | Self::Delete { key }
            | Self::CompareAndSwap { key, .. } => key,
        };
        validate_state_key(key, "bridge.kv.key")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginBridgeRequest {
    pub call_id: PluginBridgeCallId,
    pub target: PluginBridgeTarget,
}

impl PluginBridgeRequest {
    pub fn validate_for(
        &self,
        session: &PluginBridgeSession,
        current: &PluginReleasePointerState,
    ) -> Result<(), PluginRuntimeContractError> {
        session.validate()?;
        current.validate()?;
        validate_nonempty(self.call_id.as_ref(), "call_id")?;
        if session.plugin_product_id != current.plugin_product_id
            || current.active_release.as_ref() != Some(&session.active_release)
            || current.active_release_epoch != session.active_release_epoch
        {
            return Err(PluginRuntimeContractError::StaleBridgeSession);
        }
        match &self.target {
            PluginBridgeTarget::AgentSession { request } => request.validate(),
            PluginBridgeTarget::HostKv { request } => request.validate(),
            PluginBridgeTarget::Service { method, payload } => {
                if session.service_run_key.is_none() {
                    return Err(invalid(
                        "target",
                        "UI-only Plugin Product Bridge cannot target a Node Service",
                    ));
                }
                validate_machine_key(method, "service.method")?;
                if !payload.0.is_object() {
                    return Err(invalid(
                        "service.payload",
                        "Service Bridge payload must be a JSON object",
                    ));
                }
                Ok(())
            }
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginServiceTestOutcome {
    Passed,
    Failed,
    NeedsTestInput,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginServiceTestCredentialMode {
    None,
    OneShotCurrentBindings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginServiceTestReceipt {
    pub receipt_id: PluginServiceTestReceiptId,
    pub plugin_product_id: PluginProductId,
    pub release: PluginReleaseRef,
    pub service_run_key: DigestHex,
    pub outcome: PluginServiceTestOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<CanonicalErrorCode>,
    pub runtime: PluginServiceRuntimeFingerprint,
    pub host_target: RuntimeTarget,
    pub host_protocol_version: VersionString,
    pub sdk_contract_version: VersionString,
    pub test_contract_version: VersionString,
    pub resolved_test_input_digest: DigestHex,
    pub copied_kv_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copied_private_database_digest: Option<DigestHex>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_files_dir: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration_ledger_digest: Option<DigestHex>,
    pub credential_mode: PluginServiceTestCredentialMode,
    pub host_generation: u64,
    pub issued_at_ms: i64,
}

impl PluginServiceTestReceipt {
    pub fn validate_for(
        &self,
        ready: &PluginReadyRelease,
        resolved_spec: &ResolvedPluginServiceSpec,
    ) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.receipt_id.as_ref(), "receipt_id")?;
        validate_nonempty(self.plugin_product_id.as_ref(), "plugin_product_id")?;
        self.release.validate()?;
        validate_digest(&self.service_run_key, "service_run_key")?;
        match (self.outcome, self.error_code.as_ref()) {
            (PluginServiceTestOutcome::Failed, Some(error_code)) => {
                validate_machine_key(error_code.as_ref(), "error_code")?;
            }
            (PluginServiceTestOutcome::Failed, None) => {
                return Err(invalid(
                    "error_code",
                    "failed Service Test receipts require an error code",
                ));
            }
            (_, Some(_)) => {
                return Err(invalid(
                    "error_code",
                    "non-failed Service Test receipts cannot carry an error code",
                ));
            }
            (_, None) => {}
        }
        self.runtime.validate()?;
        validate_nonempty(self.host_target.as_ref(), "host_target")?;
        if self.host_target != self.runtime.target() {
            return Err(invalid(
                "host_target",
                "Service Test target must equal the selected Runtime target",
            ));
        }
        require_version(
            &self.host_protocol_version,
            PLUGIN_SERVICE_HOST_PROTOCOL_VERSION,
            "host_protocol_version",
        )?;
        require_version(
            &self.sdk_contract_version,
            PLUGIN_SERVICE_SDK_CONTRACT_VERSION,
            "sdk_contract_version",
        )?;
        require_version(
            &self.test_contract_version,
            PLUGIN_SERVICE_TEST_CONTRACT_VERSION,
            "test_contract_version",
        )?;
        for (field, digest) in [
            ("resolved_test_input_digest", &self.resolved_test_input_digest),
            ("copied_kv_digest", &self.copied_kv_digest),
        ] {
            validate_digest(digest, field)?;
        }
        if let Some(digest) = &self.copied_private_database_digest {
            validate_digest(digest, "copied_private_database_digest")?;
        }
        if let Some(digest) = &self.migration_ledger_digest {
            validate_digest(digest, "migration_ledger_digest")?;
        }
        if self.empty_files_dir == Some(false) {
            return Err(invalid(
                "empty_files_dir",
                "Service Test must use an empty temporary filesDir",
            ));
        }
        if self.host_generation == 0 || self.issued_at_ms <= 0 {
            return Err(invalid(
                "test_host_identity",
                "Service Test Host generation and receipt time must be positive",
            ));
        }
        resolved_spec.validate()?;
        if self.plugin_product_id != ready.plugin_product_id
            || self.release != ready.release
            || self.plugin_product_id != resolved_spec.plugin_product_id
            || self.release != resolved_spec.release
            || self.service_run_key != resolved_spec.service_run_key
            || self.runtime != resolved_spec.runtime
        {
            return Err(invalid(
                "test_receipt",
                "Service Test receipt must bind the exact Ready Release and Resolved Service Spec",
            ));
        }
        if self.empty_files_dir.is_some() != resolved_spec.storage.files_dir.is_some()
            || self.copied_private_database_digest.is_some()
                != resolved_spec.storage.private_database.is_some()
            || self.migration_ledger_digest.is_some()
                != resolved_spec.storage.private_database.is_some()
        {
            return Err(invalid(
                "test_storage",
                "Service Test copied state must match the declared optional storage",
            ));
        }
        Ok(())
    }

    pub fn reference(&self) -> PluginServiceTestReceiptRef {
        PluginServiceTestReceiptRef {
            receipt_id: self.receipt_id.clone(),
            release_id: self.release.release_id.clone(),
            release_digest: self.release.release_digest.clone(),
            service_run_key: self.service_run_key.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginSourceBundle {
    pub project_id: PluginProjectId,
    pub source_archive_artifact_id: ArtifactId,
    pub source_snapshot_digest: DigestHex,
    pub dependency_lock_artifact_id: ArtifactId,
    pub dependency_lock_digest: DigestHex,
    pub build_profile_version: VersionString,
}

impl PluginSourceBundle {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.project_id.as_ref(), "source.project_id")?;
        validate_nonempty(
            self.source_archive_artifact_id.as_ref(),
            "source.source_archive_artifact_id",
        )?;
        validate_digest(
            &self.source_snapshot_digest,
            "source.source_snapshot_digest",
        )?;
        validate_nonempty(
            self.dependency_lock_artifact_id.as_ref(),
            "source.dependency_lock_artifact_id",
        )?;
        validate_digest(
            &self.dependency_lock_digest,
            "source.dependency_lock_digest",
        )?;
        require_version(
            &self.build_profile_version,
            PLUGIN_RELEASE_PROFILE_VERSION,
            "source.build_profile_version",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginImportedTestProvenance {
    pub outcome: PluginServiceTestOutcome,
    pub release_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_run_key: Option<DigestHex>,
    pub runtime_target: RuntimeTarget,
    pub runtime_digest: DigestHex,
    pub test_contract_version: VersionString,
    pub issued_at_ms: i64,
}

impl PluginImportedTestProvenance {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_digest(&self.release_digest, "test.release_digest")?;
        if let Some(service_run_key) = &self.service_run_key {
            validate_digest(service_run_key, "test.service_run_key")?;
        }
        validate_nonempty(self.runtime_target.as_ref(), "test.runtime_target")?;
        validate_digest(&self.runtime_digest, "test.runtime_digest")?;
        require_version(
            &self.test_contract_version,
            PLUGIN_SERVICE_TEST_CONTRACT_VERSION,
            "test.test_contract_version",
        )?;
        if self.issued_at_ms <= 0 {
            return Err(invalid(
                "test.issued_at_ms",
                "test provenance time must be positive",
            ));
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct PluginShareBundleDigestInput<'a> {
    bundle_id: &'a PluginShareBundleId,
    source_plugin_product_id: &'a Option<PluginProductId>,
    release: &'a PluginReleaseArtifactV1,
    source: &'a Option<PluginSourceBundle>,
    test_provenance: &'a Option<PluginImportedTestProvenance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginShareBundleV1 {
    pub schema_version: VersionString,
    pub bundle_version: VersionString,
    pub bundle_id: PluginShareBundleId,
    pub bundle_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_plugin_product_id: Option<PluginProductId>,
    pub release: PluginReleaseArtifactV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginSourceBundle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_provenance: Option<PluginImportedTestProvenance>,
}

impl PluginShareBundleV1 {
    pub fn new(
        bundle_id: PluginShareBundleId,
        source_plugin_product_id: Option<PluginProductId>,
        release: PluginReleaseArtifactV1,
        source: Option<PluginSourceBundle>,
        test_provenance: Option<PluginImportedTestProvenance>,
    ) -> Result<Self, PluginRuntimeContractError> {
        let bundle_digest = digest_payload(&PluginShareBundleDigestInput {
            bundle_id: &bundle_id,
            source_plugin_product_id: &source_plugin_product_id,
            release: &release,
            source: &source,
            test_provenance: &test_provenance,
        })?;
        let value = Self {
            schema_version: PLUGIN_RUNTIME_SCHEMA_VERSION.into(),
            bundle_version: PLUGIN_SHARE_BUNDLE_VERSION.into(),
            bundle_id,
            bundle_digest,
            source_plugin_product_id,
            release,
            source,
            test_provenance,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        require_version(
            &self.schema_version,
            PLUGIN_RUNTIME_SCHEMA_VERSION,
            "schema_version",
        )?;
        require_version(
            &self.bundle_version,
            PLUGIN_SHARE_BUNDLE_VERSION,
            "bundle_version",
        )?;
        validate_nonempty(self.bundle_id.as_ref(), "bundle_id")?;
        validate_digest(&self.bundle_digest, "bundle_digest")?;
        self.release.validate()?;
        if let Some(source_plugin_product_id) = &self.source_plugin_product_id {
            validate_nonempty(source_plugin_product_id.as_ref(), "source_plugin_product_id")?;
        }
        if let Some(source) = &self.source {
            source.validate()?;
            if source.dependency_lock_digest
                != self.release.manifest.payload.dependency_lock_digest
                || source.build_profile_version
                    != self.release.manifest.payload.build_profile_version
            {
                return Err(invalid(
                    "source",
                    "Share Source, dependency lock, Build Profile, and Release must form one exact lineage",
                ));
            }
        }
        if let Some(test) = &self.test_provenance {
            test.validate()?;
            if test.release_digest != self.release.artifact_digest {
                return Err(invalid(
                    "test_provenance",
                    "imported Test provenance must bind the bundled Release digest",
                ));
            }
        }
        let expected = digest_payload(&PluginShareBundleDigestInput {
            bundle_id: &self.bundle_id,
            source_plugin_product_id: &self.source_plugin_product_id,
            release: &self.release,
            source: &self.source,
            test_provenance: &self.test_provenance,
        })?;
        if expected != self.bundle_digest {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "bundle_digest",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginShareImportAsNewRequest {
    pub bundle_id: PluginShareBundleId,
    pub bundle_digest: DigestHex,
    pub new_plugin_product_id: PluginProductId,
    pub expected_release_digest: DigestHex,
}

impl PluginShareImportAsNewRequest {
    pub fn validate_for(
        &self,
        bundle: &PluginShareBundleV1,
    ) -> Result<(), PluginRuntimeContractError> {
        bundle.validate()?;
        validate_nonempty(self.new_plugin_product_id.as_ref(), "new_plugin_product_id")?;
        validate_digest(&self.bundle_digest, "bundle_digest")?;
        validate_digest(
            &self.expected_release_digest,
            "expected_release_digest",
        )?;
        if self.bundle_id != bundle.bundle_id
            || self.bundle_digest != bundle.bundle_digest
            || self.expected_release_digest != bundle.release.artifact_digest
        {
            return Err(invalid(
                "share_import",
                "import must bind the exact Share Bundle and Release",
            ));
        }
        if bundle.source_plugin_product_id.as_ref() == Some(&self.new_plugin_product_id) {
            return Err(invalid(
                "new_plugin_product_id",
                "Share import always creates a new Plugin Product identity",
            ));
        }
        Ok(())
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginBackupSourceState {
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginProductBackupMetadataV1 {
    pub schema_version: VersionString,
    pub backup_version: VersionString,
    pub backup_id: PluginBackupId,
    pub source_plugin_product_id: PluginProductId,
    pub source_state: PluginBackupSourceState,
    pub owner_quiescent: bool,
    pub product_metadata_digest: DigestHex,
    pub source_archive_digest: DigestHex,
    pub release_inventory_digest: DigestHex,
    pub config_digest: DigestHex,
    pub kv_digest: DigestHex,
    pub files_digest: DigestHex,
    pub private_database_digest: DigestHex,
    pub migration_ledger_digest: DigestHex,
    pub credential_slot_keys: BTreeSet<String>,
    pub created_at_ms: i64,
}

impl PluginProductBackupMetadataV1 {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        require_version(
            &self.schema_version,
            PLUGIN_RUNTIME_SCHEMA_VERSION,
            "schema_version",
        )?;
        require_version(
            &self.backup_version,
            PLUGIN_PRODUCT_BACKUP_VERSION,
            "backup_version",
        )?;
        validate_nonempty(self.backup_id.as_ref(), "backup_id")?;
        validate_nonempty(self.source_plugin_product_id.as_ref(), "source_plugin_product_id")?;
        if !self.owner_quiescent {
            return Err(invalid(
                "owner_quiescent",
                "Plugin Product Backup requires zero Service, Build, Test, Migration, and owner writers",
            ));
        }
        for (field, digest) in [
            ("product_metadata_digest", &self.product_metadata_digest),
            ("source_archive_digest", &self.source_archive_digest),
            ("release_inventory_digest", &self.release_inventory_digest),
            ("config_digest", &self.config_digest),
            ("kv_digest", &self.kv_digest),
            ("files_digest", &self.files_digest),
            ("private_database_digest", &self.private_database_digest),
            ("migration_ledger_digest", &self.migration_ledger_digest),
        ] {
            validate_digest(digest, field)?;
        }
        for slot_key in &self.credential_slot_keys {
            validate_machine_key(slot_key, "credential_slot_keys")?;
        }
        if self.created_at_ms <= 0 {
            return Err(invalid(
                "created_at_ms",
                "backup creation time must be positive",
            ));
        }
        Ok(())
    }

    pub fn metadata_digest(&self) -> Result<DigestHex, PluginRuntimeContractError> {
        self.validate()?;
        Ok(digest_payload(self)?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginBackupImportAsNewRequest {
    pub backup_id: PluginBackupId,
    pub backup_metadata_digest: DigestHex,
    pub new_plugin_product_id: PluginProductId,
}

impl PluginBackupImportAsNewRequest {
    pub fn validate_for(
        &self,
        metadata: &PluginProductBackupMetadataV1,
    ) -> Result<(), PluginRuntimeContractError> {
        metadata.validate()?;
        validate_nonempty(self.new_plugin_product_id.as_ref(), "new_plugin_product_id")?;
        validate_digest(
            &self.backup_metadata_digest,
            "backup_metadata_digest",
        )?;
        if self.backup_id != metadata.backup_id
            || self.backup_metadata_digest != metadata.metadata_digest()?
        {
            return Err(invalid(
                "backup_import",
                "import must bind the exact Plugin Product Backup metadata",
            ));
        }
        if self.new_plugin_product_id == metadata.source_plugin_product_id {
            return Err(invalid(
                "new_plugin_product_id",
                "Plugin Product Backup import always creates a new Plugin Product identity",
            ));
        }
        Ok(())
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginProductLifecycleState {
    Enabled,
    Disabled,
    Trashed,
    Deleting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginProductDeletingIntent {
    pub plugin_product_id: PluginProductId,
    pub operation_id: OperationId,
    pub started_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<CanonicalErrorCode>,
}

impl PluginProductDeletingIntent {
    fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.plugin_product_id.as_ref(), "deleting.plugin_product_id")?;
        validate_nonempty(self.operation_id.as_ref(), "deleting.operation_id")?;
        if self.started_at_ms <= 0 {
            return Err(invalid(
                "deleting.started_at_ms",
                "delete start time must be positive",
            ));
        }
        if let Some(last_error) = &self.last_error {
            validate_nonempty(last_error.as_ref(), "deleting.last_error")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginProductLifecycleRecord {
    pub plugin_product_id: PluginProductId,
    pub state: PluginProductLifecycleState,
    pub pointer_state: PluginReleasePointerState,
    pub surface_available: bool,
    pub catalog_published: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleting_intent: Option<PluginProductDeletingIntent>,
}

impl PluginProductLifecycleRecord {
    pub fn validate(&self) -> Result<(), PluginRuntimeContractError> {
        validate_nonempty(self.plugin_product_id.as_ref(), "plugin_product_id")?;
        self.pointer_state.validate()?;
        if self.pointer_state.plugin_product_id != self.plugin_product_id {
            return Err(invalid(
                "pointer_state.plugin_product_id",
                "pointer state belongs to another Plugin Product",
            ));
        }
        match (&self.state, &self.deleting_intent) {
            (PluginProductLifecycleState::Deleting, Some(intent)) => {
                intent.validate()?;
                if intent.plugin_product_id != self.plugin_product_id {
                    return Err(invalid(
                        "deleting_intent.plugin_product_id",
                        "deleting intent belongs to another Plugin Product",
                    ));
                }
            }
            (PluginProductLifecycleState::Deleting, None) => {
                return Err(invalid(
                    "deleting_intent",
                    "Deleting state requires a durable deleting intent",
                ));
            }
            (_, Some(_)) => {
                return Err(invalid(
                    "deleting_intent",
                    "deleting intent exists only while the product is deleting",
                ));
            }
            (_, None) => {}
        }
        match self.state {
            PluginProductLifecycleState::Enabled => {
                if self.pointer_state.active_release.is_none()
                    || !self.surface_available
                    || !self.catalog_published
                {
                    return Err(invalid(
                        "enabled_state",
                        "enabled Plugin Product requires Active Release, Surface, and materialized Catalog",
                    ));
                }
            }
            PluginProductLifecycleState::Disabled
            | PluginProductLifecycleState::Trashed
            | PluginProductLifecycleState::Deleting => {
                if self.surface_available || self.catalog_published {
                    return Err(invalid(
                        "inactive_state",
                        "disabled, trashed, or deleting Plugin Product cannot expose Surface or Catalog",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum PluginRuntimeContractError {
    #[error("{field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: String,
    },
    #[error("duplicate {field}: {value}")]
    DuplicateIdentity {
        field: &'static str,
        value: String,
    },
    #[error("{field} digest mismatch")]
    DigestMismatch { field: &'static str },
    #[error("pointer compare-and-swap conflict")]
    CompareAndSwapConflict,
    #[error("stale or foreign Bridge session")]
    StaleBridgeSession,
    #[error(transparent)]
    Digest(#[from] crate::CanonicalDigestError),
}

fn invalid(field: &'static str, reason: impl Into<String>) -> PluginRuntimeContractError {
    PluginRuntimeContractError::InvalidField {
        field,
        reason: reason.into(),
    }
}

fn duplicate(field: &'static str, value: &str) -> PluginRuntimeContractError {
    PluginRuntimeContractError::DuplicateIdentity {
        field,
        value: value.to_owned(),
    }
}

fn require_version(
    value: &VersionString,
    expected: &str,
    field: &'static str,
) -> Result<(), PluginRuntimeContractError> {
    if value.as_ref() == expected {
        Ok(())
    } else {
        Err(invalid(
            field,
            format!("expected {expected}, observed {}", value.as_ref()),
        ))
    }
}

fn validate_nonempty(value: &str, field: &'static str) -> Result<(), PluginRuntimeContractError> {
    if value.is_empty() || value.trim() != value {
        Err(invalid(field, "value must be non-empty and trimmed"))
    } else {
        Ok(())
    }
}

fn validate_display(value: &LocalizedMetadata) -> Result<(), PluginRuntimeContractError> {
    validate_nonempty(&value.name, "display.name")?;
    validate_nonempty(&value.description, "display.description")
}

fn validate_digest(
    digest: &DigestHex,
    field: &'static str,
) -> Result<(), PluginRuntimeContractError> {
    let value = digest.as_ref();
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Err(invalid(
            field,
            "digest must be 64 lowercase hexadecimal characters",
        ))
    } else {
        Ok(())
    }
}

fn validate_relative_path(
    value: &str,
    field: &'static str,
) -> Result<(), PluginRuntimeContractError> {
    validate_nonempty(value, field)?;
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.split('/').any(|segment| {
            segment.is_empty() || segment == "." || segment == ".."
        })
    {
        Err(invalid(
            field,
            "path must be normalized, relative, slash-separated, and traversal-free",
        ))
    } else {
        Ok(())
    }
}

fn validate_absolute_path(
    value: &str,
    field: &'static str,
) -> Result<(), PluginRuntimeContractError> {
    validate_nonempty(value, field)?;
    let bytes = value.as_bytes();
    let windows_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let windows_unc = value.starts_with("\\\\");
    let unix_root = value.starts_with('/');
    if windows_drive || windows_unc || unix_root {
        Ok(())
    } else {
        Err(invalid(field, "path must be absolute"))
    }
}

fn validate_machine_key(
    value: &str,
    field: &'static str,
) -> Result<(), PluginRuntimeContractError> {
    validate_nonempty(value, field)?;
    if value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
    {
        Err(invalid(
            field,
            "machine key contains unsupported characters or exceeds 128 bytes",
        ))
    } else {
        Ok(())
    }
}

fn validate_state_key(
    value: &str,
    field: &'static str,
) -> Result<(), PluginRuntimeContractError> {
    validate_nonempty(value, field)?;
    if value.len() > 512 || value.bytes().any(|byte| byte.is_ascii_control()) {
        Err(invalid(
            field,
            "state key exceeds 512 bytes or contains control characters",
        ))
    } else {
        Ok(())
    }
}

fn validate_sql_identifier(
    value: &str,
    field: &'static str,
) -> Result<(), PluginRuntimeContractError> {
    validate_nonempty(value, field)?;
    let mut bytes = value.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        || value.len() > 128
    {
        Err(invalid(
            field,
            "SQL identifier must use ASCII letters, digits, and underscore",
        ))
    } else {
        Ok(())
    }
}

fn reject_sql_control_tokens(
    value: &str,
    field: &'static str,
) -> Result<(), PluginRuntimeContractError> {
    let uppercase = value.to_ascii_uppercase();
    if value.contains(';')
        || value.contains("--")
        || value.contains("/*")
        || [
            " DROP ",
            " DELETE ",
            " UPDATE ",
            " INSERT ",
            " ALTER ",
            " ATTACH ",
            " PRAGMA ",
        ]
        .iter()
        .any(|token| format!(" {uppercase} ").contains(token))
    {
        Err(invalid(
            field,
            "migration fragments cannot contain SQL control or destructive tokens",
        ))
    } else {
        Ok(())
    }
}

fn validate_config_schema(
    config_schema: &StrictJsonValue,
) -> Result<(), PluginRuntimeContractError> {
    if config_schema.0.is_object() {
        Ok(())
    } else {
        Err(invalid(
            "config_schema",
            "Plugin Product config schema must be a JSON object",
        ))
    }
}

fn validate_credential_slots(
    credential_slots: &[CredentialSlotDeclaration],
) -> Result<(), PluginRuntimeContractError> {
    let mut previous: Option<&str> = None;
    for slot in credential_slots {
        validate_machine_key(
            slot.slot_key.as_ref(),
            "credential_slots.slot_key",
        )?;
        validate_nonempty(&slot.display_name, "credential_slots.display_name")?;
        if previous.is_some_and(|previous| previous >= slot.slot_key.as_ref()) {
            return Err(invalid(
                "credential_slots",
                "Credential slots must be sorted by slot_key and unique",
            ));
        }
        previous = Some(slot.slot_key.as_ref());
    }
    Ok(())
}

pub fn validate_release_schema_registry(
    contributions: &PackageContributions,
    schemas: &BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
) -> Result<(), PluginRuntimeContractError> {
    let referenced = contributions
        .capabilities
        .iter()
        .flat_map(|capability| {
            capability
                .contributions
                .actions
                .iter()
                .flat_map(|action| [&action.input_schema, &action.output_schema])
                .chain(capability.contributions.context_schema_refs.iter())
                .chain(capability.contributions.event_schema_refs.iter())
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let provided = schemas.keys().cloned().collect::<BTreeSet<_>>();
    if referenced != provided {
        let missing = referenced
            .difference(&provided)
            .map(AsRef::as_ref)
            .collect::<Vec<_>>();
        let extra = provided
            .difference(&referenced)
            .map(AsRef::as_ref)
            .collect::<Vec<_>>();
        return Err(invalid(
            "schemas",
            format!(
                "schema registry must exactly match contribution refs; missing={missing:?}, extra={extra:?}"
            ),
        ));
    }
    for (reference, schema) in schemas {
        if !reference.as_ref().starts_with("schema://") {
            return Err(invalid(
                "schemas.key",
                format!(
                    "schema {} must use the canonical schema:// namespace",
                    reference.as_ref()
                ),
            ));
        }
        if !schema.0.is_object() {
            return Err(invalid(
                "schemas.value",
                format!(
                    "schema {} must contain a JSON object",
                    reference.as_ref()
                ),
            ));
        }
        let (logical_ref, expected_digest) =
            reference.as_ref().rsplit_once('#').ok_or_else(|| {
                invalid(
                    "schemas.key",
                    format!(
                        "schema {} must end in a content digest fragment",
                        reference.as_ref()
                    ),
                )
            })?;
        if logical_ref.contains('#') || logical_ref == "schema://" {
            return Err(invalid(
                "schemas.key",
                format!("schema {} is not a canonical reference", reference.as_ref()),
            ));
        }
        validate_digest(
            &DigestHex::from(expected_digest.to_owned()),
            "schemas.key.digest",
        )?;
        let observed = digest_payload(&schema.0)?;
        if observed.as_ref() != expected_digest {
            return Err(PluginRuntimeContractError::DigestMismatch {
                field: "schemas.value",
            });
        }
    }
    Ok(())
}

fn validate_contributions(
    package: &PackageRef,
    contributions: &PackageContributions,
) -> Result<(), PluginRuntimeContractError> {
    if !contributions.skills.is_empty()
        || !contributions.role_contracts.is_empty()
        || !contributions.role_providers.is_empty()
    {
        return Err(invalid(
            "contributions",
            "Plugin runtime publishes executable Capability or MCP contributions only",
        ));
    }
    let mut capability_ids = BTreeSet::new();
    let mut contribution_ids = BTreeSet::new();
    for capability in &contributions.capabilities {
        crate::model_middleware::validate_manifest(capability)
            .map_err(|reason| invalid("contributions.before_model", reason))?;
        if capability.kind == crate::CapabilityKind::UiContribution || capability.contributions.ui_slot.is_some() {
            if capability.kind != crate::CapabilityKind::UiContribution
                || capability.contributions.ui_slot.is_none()
                || capability.supported_consumers().map_err(|reason| invalid("capability.supported_surfaces", reason))? != BTreeSet::from([crate::CapabilityConsumer::Ui])
                || !capability.requires.is_empty() || !capability.conflicts.is_empty()
                || !capability.requires_runtime_features.is_empty()
                || !capability.contributions.actions.is_empty()
                || !capability.contributions.context_schema_refs.is_empty()
                || !capability.contributions.event_schema_refs.is_empty()
                || !capability.contributions.resource_kinds.is_empty()
                || !capability.contributions.host_ports.is_empty()
            {
                return Err(invalid("contributions.ui_slot", "UI contributions declare a UI-only slot; executable dependencies and host resources require their own supported consumer"));
            }
        }
        validate_nonempty(capability.id.as_ref(), "capability.id")?;
        validate_nonempty(
            capability.contribution_id.as_ref(),
            "capability.contribution_id",
        )?;
        validate_nonempty(capability.version.as_ref(), "capability.version")?;
        if &capability.package != package {
            return Err(invalid(
                "capability.package",
                "Capability must use the Release contribution package",
            ));
        }
        if !capability_ids.insert(capability.id.clone()) {
            return Err(duplicate("capability.id", capability.id.as_ref()));
        }
        if !contribution_ids.insert(capability.contribution_id.clone()) {
            return Err(duplicate(
                "capability.contribution_id",
                capability.contribution_id.as_ref(),
            ));
        }
        if capability.supported_consumers().map_err(|reason| {
            invalid("capability.supported_surfaces", reason)
        })?.is_empty()
            || capability.supported_platforms.is_empty()
        {
            return Err(invalid(
                "capability.availability",
                "Capability requires at least one consumer and platform",
            ));
        }
        if !capability.config_schema.0.is_object() {
            return Err(invalid(
                "capability.config_schema",
                "Capability config schema must be a JSON object",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use schemars::schema_for;
    use serde_json::json;

    use super::*;
    use crate::{
        ActionId, CapabilityActionDescriptor, CapabilityConsumer,
        CapabilityContributions, CapabilityId, CapabilityKind,
        CapabilityManifest, CredentialSlotKey, CredentialSlotKind,
        EffectClass, PackageId, PlatformConstraint, ToolPresentationKind,
        capability_surface_declarations, digest_bytes,
    };

    fn digest(seed: &str) -> DigestHex {
        digest_bytes(seed.as_bytes())
    }

    fn release_file(path: &str, seed: &str) -> PluginReleaseFile {
        PluginReleaseFile {
            normalized_relative_path: path.into(),
            digest: digest(seed),
            size_bytes: seed.len() as u64,
        }
    }

    fn config_schema() -> StrictJsonValue {
        StrictJsonValue(json!({
            "additionalProperties": false,
            "properties": {
                "workspace": {"type": "string"}
            },
            "type": "object"
        }))
    }

    fn credential_slots(with_service: bool) -> Vec<CredentialSlotDeclaration> {
        with_service
            .then(|| CredentialSlotDeclaration {
                slot_key: CredentialSlotKey::from("api_key"),
                kind: CredentialSlotKind::SecretText,
                display_name: "API key".into(),
                required: true,
            })
            .into_iter()
            .collect()
    }

    fn resource_contract(with_service: bool) -> PluginResourceContract {
        PluginResourceContract {
            required_resource_kinds: with_service
                .then(|| ResourceKind::from("knowledge.base"))
                .into_iter()
                .collect(),
        }
    }

    fn manifest(files: &[PluginReleaseFile], with_service: bool) -> PluginReleaseV1Manifest {
        let entrypoint = files
            .iter()
            .find(|file| file.normalized_relative_path == "ui/index.html")
            .unwrap();
        let service = with_service.then(|| {
            let service = files
                .iter()
                .find(|file| file.normalized_relative_path == "service/main.mjs")
                .unwrap();
            PluginServiceReleaseDescriptor {
                execution: Default::default(),
                entrypoint: "service/main.mjs".into(),
                module_digest: service.digest.clone(),
                lifecycle: PluginServiceLifecycle::OnDemand,
                uses_files: true,
                uses_private_database: true,
                service_contract_digest: digest("service-contract"),
                host_protocol_version: PLUGIN_SERVICE_HOST_PROTOCOL_VERSION.into(),
                sdk_contract_version: PLUGIN_SERVICE_SDK_CONTRACT_VERSION.into(),
                runtime_requirements_digest: digest("runtime-requirements"),
            }
        });
        let config_schema = config_schema();
        let credential_slots = credential_slots(with_service);
        let resource_contract = resource_contract(with_service);
        PluginReleaseV1Manifest {
            schema_version: PLUGIN_RUNTIME_SCHEMA_VERSION.into(),
        build_profile: JavaScriptBuildProfile::PluginReleaseV1,
            build_profile_version: PLUGIN_RELEASE_PROFILE_VERSION.into(),
            display: LocalizedMetadata {
                name: "Example".into(),
                description: "Example Plugin Product".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            ui: Some(PluginUiReleaseDescriptor {
                entrypoint: "ui/index.html".into(),
                entrypoint_digest: entrypoint.digest.clone(),
                ui_tree_digest: canonical_ui_tree_digest(files).unwrap(),
            }),
            service,
            dependency_lock_digest: digest("lock"),
            dependency_graph_digest: digest("graph"),
            config_schema_digest: digest_payload(&config_schema.0).unwrap(),
            config_schema,
            credential_slots_digest: digest_payload(&credential_slots).unwrap(),
            credential_slots,
            resource_contract_digest: digest_payload(&resource_contract).unwrap(),
            resource_contract,
            schemas: BTreeMap::new(),
            bridge_contract_digest: digest("bridge"),
            contribution_package: PackageRef {
                id: PackageId::from("plugin.example.release"),
                version: VersionString::from("1.0.0"),
            },
            contributions: PackageContributions::default(),
            migrations: Vec::new(),
        }
    }

    #[test]
    fn native_release_requires_exact_target_entrypoint_and_keeps_node_default_bytes() {
        let mut files = vec![release_file("ui/index.html", "ui"), release_file("service/main.mjs", "service")];
        let mut manifest = manifest(&files, true);
        let node = manifest.service.as_ref().unwrap();
        let serialized = serde_json::to_value(node).unwrap();
        assert!(serialized.get("execution").is_none());
        assert_eq!(serde_json::from_value::<PluginServiceReleaseDescriptor>(serialized).unwrap(), *node);
        let service = manifest.service.as_mut().unwrap();
        service.execution = PluginServiceExecution::Native { target: NativePluginTarget::WindowsX64 };
        assert!(service.validate().is_err());
        service.entrypoint = "service/plugin.exe".into();
        assert!(service.validate().is_ok());
        assert!(PluginReleaseArtifactV1::new("native".into(), manifest.clone(), files.clone()).is_err());
        files[1].normalized_relative_path = "service/plugin.exe".into();
        let artifact = PluginReleaseArtifactV1::new("native".into(), manifest, files).unwrap();
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn before_model_publication_requires_real_middleware_kind_and_exact_contract() {
        let files = vec![release_file("ui/index.html", "ui"), release_file("service/main.mjs", "service")];
        let mut manifest = manifest_with_tool(&files);
        let capability = &mut manifest.contributions.capabilities[0];
        capability.kind = CapabilityKind::TurnMiddleware;
        capability.contributions = CapabilityContributions {
            actions: vec![crate::model_middleware::action()], ..Default::default()
        };
        capability.supported_surfaces = capability_surface_declarations(
            ["desktop", "headless"], [CapabilityConsumer::Agent, CapabilityConsumer::PluginService]);
        assert!(validate_contributions(&manifest.contribution_package, &manifest.contributions).is_ok());
        for mutation in 0..5 {
            let mut invalid_manifest = manifest.clone();
            let c = &mut invalid_manifest.contributions.capabilities[0];
            match mutation {
                0 => c.kind = CapabilityKind::Tool,
                1 => c.contributions.actions[0].presentation = ToolPresentationKind::FunctionTool,
                2 => c.contributions.actions[0].output_schema = "schema://wrong@1".into(),
                3 => { c.contributions.resource_kinds.insert("native.object".into()); },
                _ => c.supported_surfaces = capability_surface_declarations(["desktop"], [CapabilityConsumer::Ui]),
            }
            assert!(validate_contributions(&invalid_manifest.contribution_package, &invalid_manifest.contributions).is_err(), "mutation {mutation}");
        }
    }

    fn manifest_with_tool(
        files: &[PluginReleaseFile],
    ) -> PluginReleaseV1Manifest {
        let mut manifest = manifest(files, true);
        let input_schema = StrictJsonValue(json!({
            "additionalProperties": false,
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
            "type": "object"
        }));
        let output_schema = StrictJsonValue(json!({
            "additionalProperties": false,
            "properties": {"matches": {"type": "array"}},
            "required": ["matches"],
            "type": "object"
        }));
        let input_ref = CanonicalSchemaRef::from(format!(
            "schema://plugin.example/search-input@1#{}",
            digest_payload(&input_schema.0).unwrap().as_ref()
        ));
        let output_ref = CanonicalSchemaRef::from(format!(
            "schema://plugin.example/search-output@1#{}",
            digest_payload(&output_schema.0).unwrap().as_ref()
        ));
        let capability = CapabilityManifest {
            id: CapabilityId::from("plugin.example.search"),
            contribution_id: "capability:plugin.example.search".into(),
            version: "1.0.0".into(),
            kind: CapabilityKind::Tool,
            package: manifest.contribution_package.clone(),
            display: LocalizedMetadata {
                name: "Search".into(),
                description: "Search the selected knowledge base.".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent, CapabilityConsumer::PluginService],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({"type": "object"})),
            contributions: CapabilityContributions {
                actions: vec![CapabilityActionDescriptor {
                    action_id: ActionId::from("plugin.example.search.invoke"),
                    input_schema: input_ref.clone(),
                    output_schema: output_ref.clone(),
                    effect_class: EffectClass::ReadSensitive,
                    presentation: ToolPresentationKind::FunctionTool,
                }],
                resource_kinds: BTreeSet::from([ResourceKind::from(
                    "knowledge.base",
                )]),
                ..Default::default()
            },
        };
        manifest.contributions = PackageContributions {
            capabilities: vec![capability],
            ..Default::default()
        };
        manifest.schemas =
            BTreeMap::from([(input_ref, input_schema), (output_ref, output_schema)]);
        manifest
    }

    fn artifact(with_service: bool) -> PluginReleaseArtifactV1 {
        let mut files = vec![
            release_file("ui/index.html", "index"),
            release_file("ui/app.js", "app"),
        ];
        if with_service {
            files.push(release_file("service/main.mjs", "service"));
        }
        PluginReleaseArtifactV1::new(
            ArtifactId::from("artifact-1"),
            manifest(&files, with_service),
            files,
        )
        .unwrap()
    }

    fn artifact_with_tool() -> PluginReleaseArtifactV1 {
        let files = vec![
            release_file("ui/index.html", "index"),
            release_file("ui/app.js", "app"),
            release_file("service/main.mjs", "service"),
        ];
        PluginReleaseArtifactV1::new(
            ArtifactId::from("artifact-tool"),
            manifest_with_tool(&files),
            files,
        )
        .unwrap()
    }

    fn release_ref(seed: &str) -> PluginReleaseRef {
        PluginReleaseRef {
            release_id: PluginReleaseId::from(format!("release-{seed}")),
            artifact_id: ArtifactId::from(format!("artifact-{seed}")),
            release_digest: digest(&format!("release-{seed}")),
            manifest_digest: digest(&format!("manifest-{seed}")),
        }
    }

    fn pointer_state() -> PluginReleasePointerState {
        PluginReleasePointerState {
            plugin_product_id: PluginProductId::from("plugin-1"),
            pointer_revision: 7,
            active_release_epoch: 3,
            ready_release: Some(PluginReadyReleaseRef {
                release_id: PluginReleaseId::from("release-ready"),
                release_digest: digest("release-ready"),
            }),
            active_release: Some(release_ref("active")),
            previous_release: Some(release_ref("previous")),
            materialized_catalog_digest: digest("catalog-active"),
        }
    }

    fn runtime() -> PluginServiceRuntimeFingerprint {
        PluginServiceRuntimeFingerprint::Node {
            runtime_installation_id: RuntimeInstallationId::from("runtime-1"),
            runtime_target: RuntimeTarget::from("windows-x86_64"),
            runtime_executable_digest: digest("node"),
            node_version: VersionString::from("24.1.0"),
        }
    }

    fn storage() -> PluginServiceStorageDescriptor {
        PluginServiceStorageDescriptor {
            kv: PluginKvHandleDescriptor {
                handle_id: PluginKvHandleId::from("kv-1"),
                plugin_product_id: PluginProductId::from("plugin-1"),
                namespace_revision: 1,
            },
            files_dir: Some(PluginFilesDirDescriptor {
                handle_id: PluginFilesHandleId::from("files-1"),
                plugin_product_id: PluginProductId::from("plugin-1"),
                absolute_path: "C:\\NomiFun\\plugins\\plugin-1\\files".into(),
            }),
            private_database: Some(PluginPrivateDatabaseDescriptor {
                handle_id: PluginDatabaseHandleId::from("db-1"),
                plugin_product_id: PluginProductId::from("plugin-1"),
                schema_epoch: 2,
                migration_ledger_digest: digest("ledger"),
            }),
        }
    }

    fn resolved_spec() -> ResolvedPluginServiceSpec {
        ResolvedPluginServiceSpec::new(ResolvedPluginServiceSpecInputs {
            plugin_product_id: PluginProductId::from("plugin-1"),
            release: release_ref("ready"),
            active_release_epoch: 4,
            service_module_digest: digest("service"),
            lifecycle: PluginServiceLifecycle::OnDemand,
            host_protocol_version: PLUGIN_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: PLUGIN_SERVICE_SDK_CONTRACT_VERSION.into(),
            runtime: runtime(),
            config_schema_digest: digest("config"),
            config_snapshot_digest: digest("config-snapshot"),
            credential_slots_digest: digest("credentials"),
            resource_contract_digest: digest("resources"),
            resource_bindings_digest: digest("resource-bindings"),
            runtime_requirements_digest: digest("runtime-requirements"),
            bridge_contract_digest: digest("bridge"),
            contribution_set_digest: digest("contributions"),
            storage: storage(),
        })
        .unwrap()
    }

    fn non_ui_fingerprint() -> PluginNonUiReleaseFingerprint {
        PluginNonUiReleaseFingerprint {
            manifest_without_ui_digest: digest("manifest-without-ui"),
            service_run_key: Some(digest("run-key")),
            migration_set_digest: digest("migrations"),
            contribution_set_digest: digest("contributions"),
            bridge_contract_digest: digest("bridge"),
            config_schema_digest: digest("config"),
            credential_slots_digest: digest("credentials"),
            resource_contract_digest: digest("resources"),
            runtime_requirements_digest: digest("runtime"),
            dependency_lock_digest: digest("lock"),
        }
    }

    #[test]
    fn canonical_contract_freezes_independent_runtime_boundaries() {
        let contract = PluginRuntimeContractManifest::canonical();
        contract.validate().unwrap();
        assert_eq!(contract.max_services_per_plugin_product, 1);
        assert!(!contract.ui_only_starts_node);
        assert!(!contract.exposes_localhost_bridge);
        assert_eq!(contract.service_lifecycles.len(), 2);
        assert_eq!(
            contract.bridge_transports,
            BTreeSet::from([PluginBridgeTransport::MessageChannelV1])
        );
    }

    #[test]
    fn agent_view_requires_html_ui_only_slot_and_no_unconsumed_execution_graph() {
        let mut view = artifact(false).manifest.payload;
        let mut cap = artifact_with_tool().manifest.payload.contributions.capabilities[0].clone();
        cap.id = "plugin.example.ui.agent-session".into();
        cap.contribution_id = "capability:plugin.example.ui.agent-session".into();
        cap.kind = CapabilityKind::UiContribution;
        cap.supported_surfaces = capability_surface_declarations(["desktop"], [CapabilityConsumer::Ui]);
        cap.contributions = CapabilityContributions { ui_slot: Some(crate::UiContributionSlot::AgentSession), ..Default::default() };
        view.contributions.capabilities.push(cap.clone());
        view.validate().unwrap();
        let mut combined = artifact_with_tool().manifest.payload;
        combined.contributions.capabilities.push(cap);
        combined.validate().unwrap();
        let mut missing_html = combined.clone();
        missing_html.ui = None;
        assert!(missing_html.validate().is_err());
        let mut wrong_kind = view.clone();
        wrong_kind.contributions.capabilities[0].kind = CapabilityKind::Tool;
        assert!(wrong_kind.validate().is_err());
        let mut wrong_consumer = view.clone();
        wrong_consumer.contributions.capabilities[0].supported_surfaces = capability_surface_declarations(["desktop"], [CapabilityConsumer::Ui, CapabilityConsumer::Agent]);
        assert!(wrong_consumer.validate().is_err());
        let mut no_slot = view.clone();
        no_slot.contributions.capabilities[0].contributions.ui_slot = None;
        assert!(no_slot.validate().is_err());
        let mut unused_graph = view.clone();
        unused_graph.contributions.capabilities[0].requires.push(crate::CapabilityRef { id: "another.tool".into(), version: "1.0.0".into() });
        assert!(unused_graph.validate().is_err());
    }

    #[test]
    fn release_artifact_is_independent_and_canonical() {
        let artifact = artifact(true);
        artifact.validate().unwrap();
        assert_eq!(
            artifact.artifact_digest,
            canonical_release_artifact_digest(&artifact.manifest, &artifact.files).unwrap()
        );
        assert!(artifact
            .files
            .iter()
            .all(|file| file.normalized_relative_path.starts_with("ui/")
                || file.normalized_relative_path == "service/main.mjs"));
    }

    #[test]
    fn source_less_release_restores_workshop_and_runtime_contracts() {
        let artifact = artifact_with_tool();
        let encoded = serde_json::to_value(&artifact).unwrap();
        assert!(encoded.pointer("/manifest/payload/config_schema").is_some());
        assert!(encoded
            .pointer("/manifest/payload/credential_slots")
            .is_some());
        assert!(encoded
            .pointer("/manifest/payload/resource_contract")
            .is_some());
        assert!(encoded.pointer("/manifest/payload/schemas").is_some());
        assert!(encoded.get("source").is_none());

        let restored: PluginReleaseArtifactV1 =
            serde_json::from_value(encoded).unwrap();
        restored.validate().unwrap();
        let manifest = &restored.manifest.payload;
        assert_eq!(manifest.credential_slots.len(), 1);
        assert_eq!(
            manifest.resource_contract.required_resource_kinds,
            BTreeSet::from([ResourceKind::from("knowledge.base")])
        );
        assert_eq!(manifest.schemas.len(), 2);
    }

    #[test]
    fn release_contract_content_and_digests_are_exact() {
        let valid = artifact_with_tool().manifest.payload;
        valid.validate().unwrap();

        let mut config_tamper = valid.clone();
        config_tamper.config_schema =
            StrictJsonValue(json!({"type": "object", "properties": {}}));
        assert!(matches!(
            config_tamper.validate(),
            Err(PluginRuntimeContractError::DigestMismatch {
                field: "config_schema_digest"
            })
        ));

        let mut credential_tamper = valid.clone();
        credential_tamper.credential_slots[0].display_name =
            "Changed credential".into();
        assert!(matches!(
            credential_tamper.validate(),
            Err(PluginRuntimeContractError::DigestMismatch {
                field: "credential_slots_digest"
            })
        ));

        let mut resource_tamper = valid;
        resource_tamper
            .resource_contract
            .required_resource_kinds
            .insert(ResourceKind::from("workspace"));
        assert!(matches!(
            resource_tamper.validate(),
            Err(PluginRuntimeContractError::DigestMismatch {
                field: "resource_contract_digest"
            })
        ));
    }

    #[test]
    fn release_schema_registry_is_exact_and_content_addressed() {
        let valid = artifact_with_tool().manifest.payload;
        valid.validate().unwrap();

        let mut missing = valid.clone();
        let removed = missing.schemas.keys().next().cloned().unwrap();
        missing.schemas.remove(&removed);
        assert!(matches!(
            missing.validate(),
            Err(PluginRuntimeContractError::InvalidField {
                field: "schemas",
                ..
            })
        ));

        let extra_schema = StrictJsonValue(json!({"type": "string"}));
        let extra_ref = CanonicalSchemaRef::from(format!(
            "schema://plugin.example/unused@1#{}",
            digest_payload(&extra_schema.0).unwrap().as_ref()
        ));
        let mut extra = valid.clone();
        extra.schemas.insert(extra_ref, extra_schema);
        assert!(matches!(
            extra.validate(),
            Err(PluginRuntimeContractError::InvalidField {
                field: "schemas",
                ..
            })
        ));

        let mut tampered = valid;
        *tampered.schemas.get_mut(&removed).unwrap() =
            StrictJsonValue(json!({"type": "null"}));
        assert!(matches!(
            tampered.validate(),
            Err(PluginRuntimeContractError::DigestMismatch {
                field: "schemas.value"
            })
        ));
    }

    #[test]
    fn release_contract_deserialization_rejects_missing_and_extra_content() {
        let encoded = serde_json::to_value(
            artifact_with_tool().manifest.payload,
        )
        .unwrap();

        for field in [
            "config_schema",
            "credential_slots",
            "resource_contract",
            "schemas",
        ] {
            let mut missing = encoded.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<PluginReleaseV1Manifest>(missing)
                    .is_err(),
                "missing {field} must fail closed"
            );
        }

        let mut extra = encoded;
        extra
            .as_object_mut()
            .unwrap()
            .insert("source_contracts".into(), json!({}));
        assert!(
            serde_json::from_value::<PluginReleaseV1Manifest>(extra).is_err()
        );
    }

    #[test]
    fn release_rejects_path_traversal_and_unannounced_service() {
        let mut traversal = artifact(false);
        traversal.files[0].normalized_relative_path = "ui/../secret".into();
        assert!(traversal.validate().is_err());

        let mut ui_only = artifact(false);
        ui_only.files.push(release_file("service/main.mjs", "service"));
        ui_only.files.sort_by(|left, right| {
            left.normalized_relative_path
                .cmp(&right.normalized_relative_path)
        });
        ui_only.artifact_digest =
            canonical_release_artifact_digest(&ui_only.manifest, &ui_only.files).unwrap();
        assert!(ui_only.validate().is_err());
    }

    #[test]
    fn release_rejects_unsorted_or_tampered_files() {
        let mut unsorted = artifact(true);
        unsorted.files.reverse();
        assert!(unsorted.validate().is_err());

        let mut tampered = artifact(true);
        tampered.files[0].digest = digest("tampered");
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn migration_digest_and_additive_allowlist_are_enforced() {
        let migration = PluginMigration::new(
            PluginMigrationId::from("001_create_notes"),
            vec![PluginAdditiveMigrationAction::CreateTable {
                table_name: "notes".into(),
                columns: vec![PluginMigrationColumn {
                    name: "id".into(),
                    declared_type: "TEXT".into(),
                    nullable: false,
                    default_literal: None,
                }],
                primary_key_columns: vec!["id".into()],
            }],
        )
        .unwrap();
        migration.validate().unwrap();

        let mut tampered = migration.clone();
        tampered.actions.push(PluginAdditiveMigrationAction::AddColumn {
            table_name: "notes".into(),
            column: PluginMigrationColumn {
                name: "title".into(),
                declared_type: "TEXT".into(),
                nullable: true,
                default_literal: None,
            },
        });
        assert!(matches!(
            tampered.validate(),
            Err(PluginRuntimeContractError::DigestMismatch { .. })
        ));

        let destructive_fragment = PluginMigrationColumn {
            name: "payload".into(),
            declared_type: "TEXT; DROP TABLE notes".into(),
            nullable: true,
            default_literal: None,
        };
        assert!(destructive_fragment.validate().is_err());
    }

    #[test]
    fn service_run_key_covers_start_inputs_but_not_release_epoch() {
        let spec = resolved_spec();
        spec.validate().unwrap();
        let mut changed = spec.clone();
        changed.lifecycle = PluginServiceLifecycle::Continuous;
        assert!(matches!(
            changed.validate(),
            Err(PluginRuntimeContractError::DigestMismatch {
                field: "service_run_key"
            })
        ));
        let changed = ResolvedPluginServiceSpec::new(ResolvedPluginServiceSpecInputs {
            plugin_product_id: spec.plugin_product_id.clone(),
            release: release_ref("ui-only-change"),
            active_release_epoch: spec.active_release_epoch + 1,
            service_module_digest: spec.service_module_digest.clone(),
            lifecycle: spec.lifecycle,
            host_protocol_version: spec.host_protocol_version.clone(),
            sdk_contract_version: spec.sdk_contract_version.clone(),
            runtime: spec.runtime.clone(),
            config_schema_digest: spec.config_schema_digest.clone(),
            config_snapshot_digest: spec.config_snapshot_digest.clone(),
            credential_slots_digest: spec.credential_slots_digest.clone(),
            resource_contract_digest: spec.resource_contract_digest.clone(),
            resource_bindings_digest: spec.resource_bindings_digest.clone(),
            runtime_requirements_digest: spec.runtime_requirements_digest.clone(),
            bridge_contract_digest: spec.bridge_contract_digest.clone(),
            contribution_set_digest: spec.contribution_set_digest.clone(),
            storage: spec.storage.clone(),
        })
        .unwrap();
        assert_eq!(changed.service_run_key, spec.service_run_key);
    }

    #[test]
    fn service_spec_matches_the_exact_release_contract() {
        let artifact = artifact(true);
        let manifest = &artifact.manifest.payload;
        let service = manifest.service.as_ref().unwrap();
        let spec = ResolvedPluginServiceSpec::new(ResolvedPluginServiceSpecInputs {
            plugin_product_id: PluginProductId::from("plugin-1"),
            release: PluginReleaseRef {
                release_id: PluginReleaseId::from("release-1"),
                artifact_id: artifact.artifact_id.clone(),
                release_digest: artifact.artifact_digest.clone(),
                manifest_digest: artifact.manifest.payload_digest.clone(),
            },
            active_release_epoch: 1,
            service_module_digest: service.module_digest.clone(),
            lifecycle: service.lifecycle,
            host_protocol_version: service.host_protocol_version.clone(),
            sdk_contract_version: service.sdk_contract_version.clone(),
            runtime: runtime(),
            config_schema_digest: manifest.config_schema_digest.clone(),
            config_snapshot_digest: digest("config-snapshot"),
            credential_slots_digest: manifest.credential_slots_digest.clone(),
            resource_contract_digest: manifest.resource_contract_digest.clone(),
            resource_bindings_digest: digest("resource-bindings"),
            runtime_requirements_digest: service.runtime_requirements_digest.clone(),
            bridge_contract_digest: manifest.bridge_contract_digest.clone(),
            contribution_set_digest: manifest.contribution_set_digest().unwrap(),
            storage: storage(),
        })
        .unwrap();
        spec.validate_for_release(manifest).unwrap();

        let mut drifted = spec;
        drifted.resource_contract_digest = digest("drifted-resource-contract");
        assert!(drifted.validate_for_release(manifest).is_err());
    }

    #[test]
    fn bridge_rejects_old_epoch_and_ui_only_service_calls() {
        let session = PluginBridgeSession {
            bridge_contract_version: PLUGIN_BRIDGE_CONTRACT_VERSION.into(),
            bridge_session_id: PluginBridgeSessionId::from("bridge-1"),
            surface_session_id: PluginSurfaceSessionId::from("surface-1"),
            plugin_product_id: PluginProductId::from("plugin-1"),
            active_release: release_ref("active"),
            active_release_epoch: 4,
            transport: PluginBridgeTransport::MessageChannelV1,
            service_run_key: None,
        };
        let mut current = pointer_state();
        current.active_release = Some(session.active_release.clone());
        current.active_release_epoch = session.active_release_epoch;
        let request = PluginBridgeRequest {
            call_id: PluginBridgeCallId::from("call-1"),
            target: PluginBridgeTarget::Service {
                method: "notes.list".into(),
                payload: StrictJsonValue(json!({})),
            },
        };
        assert!(request.validate_for(&session, &current).is_err());
        current.active_release_epoch -= 1;
        assert!(matches!(
            request.validate_for(&session, &current),
            Err(PluginRuntimeContractError::StaleBridgeSession)
        ));
    }

    #[test]
    fn bridge_wire_contains_only_call_and_payload() {
        let request = PluginBridgeRequest {
            call_id: PluginBridgeCallId::from("call-1"),
            target: PluginBridgeTarget::HostKv {
                request: PluginBridgeKvRequest::Get {
                    key: "notes/current".into(),
                },
            },
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(
            value.as_object().unwrap().keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from(["call_id".into(), "target".into()])
        );
        let wire = value.to_string();
        for forbidden in [
            "plugin_product_id",
            "active_release",
            "bridge_session_id",
            "surface_session_id",
            "localhost",
        ] {
            assert!(!wire.contains(forbidden));
        }
    }

    #[test]
    fn agent_session_bridge_validates_commands_without_accepting_identity() {
        for command in [
            json!({"operation": "observe", "after_seq": 0, "limit": 200}),
            json!({"operation": "turn", "input": {"content": "hello"}, "idempotency_key": "stable"}),
            json!({"operation": "cancel"}),
        ] {
            let request: PluginAgentSessionRequest = serde_json::from_value(command.clone()).unwrap();
            request.validate().unwrap();
            for field in ["agent_session_id", "owner_user_id", "conversation_id", "extra"] {
                let mut spoofed = command.clone();
                spoofed[field] = json!("forged");
                assert!(serde_json::from_value::<PluginAgentSessionRequest>(spoofed).is_err(), "{field}: {command}");
            }
        }
        for command in [
            json!({"operation": "observe", "after_seq": 0, "limit": 0}),
            json!({"operation": "observe", "after_seq": 0, "limit": 201}),
            json!({"operation": "turn", "input": [], "idempotency_key": "stable"}),
            json!({"operation": "turn", "input": {}, "idempotency_key": " "}),
            json!({"operation": "turn", "input": {}, "idempotency_key": "字".repeat(86)}),
        ] {
            let request: PluginAgentSessionRequest = serde_json::from_value(command).unwrap();
            assert!(request.validate().is_err());
        }
        let target = json!({"target": "agent_session", "request": {"operation": "cancel"}, "agent_session_id": "forged"});
        assert!(serde_json::from_value::<PluginBridgeTarget>(target).is_err());
    }

    #[test]
    fn host_kv_handle_cannot_cross_owner() {
        let request = PluginKvRequest::Get {
            handle: PluginKvHandleDescriptor {
                handle_id: PluginKvHandleId::from("kv-1"),
                plugin_product_id: PluginProductId::from("plugin-other"),
                namespace_revision: 1,
            },
            key: "notes/current".into(),
        };
        assert!(request
            .validate_for(&PluginProductId::from("plugin-1"))
            .is_err());
    }

    #[test]
    fn strict_auto_publish_rejects_first_publish_and_non_ui_change() {
        let state = pointer_state();
        let target = PluginReleaseRef {
            release_id: PluginReleaseId::from("release-ready"),
            artifact_id: ArtifactId::from("artifact-ready"),
            release_digest: digest("release-ready"),
            manifest_digest: digest("manifest-ready"),
        };
        let proof = PluginUiOnlyAutoPublishProof {
            current_release: state.active_release.clone().unwrap(),
            target_release: target.clone(),
            current_ui_tree_digest: digest("ui-old"),
            target_ui_tree_digest: digest("ui-new"),
            current_non_ui: non_ui_fingerprint(),
            target_non_ui: non_ui_fingerprint(),
            changed_source_paths: BTreeSet::from(["ui/src/app.ts".into()]),
            changed_output_paths: BTreeSet::from(["ui/app.js".into()]),
            project_head_matches_ready_source: true,
            static_validation_passed: true,
            no_unknown_changes: true,
        };
        let request = PluginPublishRequest {
            plugin_product_id: state.plugin_product_id.clone(),
            expected: PluginPointerExpectation::from_state(&state),
            target_ready_release: target,
            target_catalog_digest: digest("catalog-ready"),
            authorization: PluginPublishAuthorization::AutoUiOnly {
                authorization: PluginUiOnlyAutoPublishAuthorization {
                    authorization_id: PluginUserAuthorizationId::from("auth-1"),
                    plugin_product_id: state.plugin_product_id.clone(),
                    enabled: true,
                    authorization_revision: 1,
                    user_authorized_at_ms: 1,
                },
                proof: Box::new(proof),
            },
        };
        request.validate_for(&state).unwrap();

        let mut first = state.clone();
        first.active_release = None;
        first.previous_release = None;
        first.active_release_epoch = 0;
        let mut first_request = request.clone();
        first_request.expected = PluginPointerExpectation::from_state(&first);
        assert!(first_request.validate_for(&first).is_err());

        let mut changed_service = request;
        let PluginPublishAuthorization::AutoUiOnly { proof, .. } =
            &mut changed_service.authorization
        else {
            unreachable!()
        };
        proof.target_non_ui.service_run_key = Some(digest("changed-run-key"));
        assert!(changed_service.validate_for(&state).is_err());
    }

    #[test]
    fn publish_and_rollback_use_exact_pointer_cas() {
        let state = pointer_state();
        let target = PluginReleaseRef {
            release_id: PluginReleaseId::from("release-ready"),
            artifact_id: ArtifactId::from("artifact-ready"),
            release_digest: digest("release-ready"),
            manifest_digest: digest("manifest-ready"),
        };
        let publish = PluginPublishRequest {
            plugin_product_id: state.plugin_product_id.clone(),
            expected: PluginPointerExpectation::from_state(&state),
            target_ready_release: target.clone(),
            target_catalog_digest: digest("catalog-ready"),
            authorization: PluginPublishAuthorization::ManualUser {
                actor_id: "user-1".into(),
            },
        };
        let published = publish.next_state(&state).unwrap();
        assert_eq!(published.active_release.as_ref(), Some(&target));
        assert_eq!(published.previous_release, state.active_release);
        assert!(published.ready_release.is_none());

        let rollback = PluginRollbackRequest {
            plugin_product_id: published.plugin_product_id.clone(),
            expected: PluginPointerExpectation::from_state(&published),
            rollback_target: published.previous_release.clone().unwrap(),
            target_catalog_digest: digest("catalog-rollback"),
            actor_id: "user-1".into(),
        };
        let rolled_back = rollback.next_state(&published).unwrap();
        assert_eq!(rolled_back.active_release, published.previous_release);
        assert_eq!(rolled_back.previous_release, published.active_release);

        let mut stale = publish;
        stale.expected.pointer_revision -= 1;
        assert!(matches!(
            stale.validate_for(&state),
            Err(PluginRuntimeContractError::CompareAndSwapConflict)
        ));
    }

    #[test]
    fn service_test_receipt_binds_exact_ready_and_spec() {
        let spec = resolved_spec();
        let ready = PluginReadyRelease {
            plugin_product_id: spec.plugin_product_id.clone(),
            release: spec.release.clone(),
            origin_operation_id: OperationId::from("build-1"),
            origin: PluginReadyOrigin::Build,
            source_lineage: PluginReleaseSourceLineage::Managed {
                project_id: PluginProjectId::from("project-1"),
                source_snapshot_digest: digest("source"),
                dependency_lock_digest: digest("lock"),
                build_profile_version: PLUGIN_RELEASE_PROFILE_VERSION.into(),
                build_generation: 1,
            },
            matching_service_test_receipt: None,
            created_at_ms: 1,
        };
        let mut receipt = PluginServiceTestReceipt {
            receipt_id: PluginServiceTestReceiptId::from("receipt-1"),
            plugin_product_id: spec.plugin_product_id.clone(),
            release: spec.release.clone(),
            service_run_key: spec.service_run_key.clone(),
            outcome: PluginServiceTestOutcome::Passed,
            error_code: None,
            runtime: spec.runtime.clone(),
            host_target: spec.runtime.target(),
            host_protocol_version: PLUGIN_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: PLUGIN_SERVICE_SDK_CONTRACT_VERSION.into(),
            test_contract_version: PLUGIN_SERVICE_TEST_CONTRACT_VERSION.into(),
            resolved_test_input_digest: digest("inputs"),
            copied_kv_digest: digest("kv"),
            copied_private_database_digest: Some(digest("db")),
            empty_files_dir: Some(true),
            migration_ledger_digest: Some(digest("ledger")),
            credential_mode: PluginServiceTestCredentialMode::None,
            host_generation: 1,
            issued_at_ms: 1,
        };
        receipt.validate_for(&ready, &spec).unwrap();
        receipt.empty_files_dir = Some(false);
        assert!(receipt.validate_for(&ready, &spec).is_err());
    }

    #[test]
    fn share_bundle_has_exact_source_chain_and_imports_as_new() {
        let release = artifact(false);
        let source = PluginSourceBundle {
            project_id: PluginProjectId::from("project-1"),
            source_archive_artifact_id: ArtifactId::from("source-1"),
            source_snapshot_digest: digest("source"),
            dependency_lock_artifact_id: ArtifactId::from("lock-1"),
            dependency_lock_digest: release.manifest.payload.dependency_lock_digest.clone(),
            build_profile_version: release.manifest.payload.build_profile_version.clone(),
        };
        let bundle = PluginShareBundleV1::new(
            PluginShareBundleId::from("share-1"),
            Some(PluginProductId::from("plugin-source")),
            release,
            Some(source),
            None,
        )
        .unwrap();
        let request = PluginShareImportAsNewRequest {
            bundle_id: bundle.bundle_id.clone(),
            bundle_digest: bundle.bundle_digest.clone(),
            new_plugin_product_id: PluginProductId::from("plugin-new"),
            expected_release_digest: bundle.release.artifact_digest.clone(),
        };
        request.validate_for(&bundle).unwrap();

        let same_identity = PluginShareImportAsNewRequest {
            new_plugin_product_id: PluginProductId::from("plugin-source"),
            ..request
        };
        assert!(same_identity.validate_for(&bundle).is_err());
    }

    #[test]
    fn plugin_product_backup_is_separate_disabled_quiescent_metadata() {
        let mut metadata = PluginProductBackupMetadataV1 {
            schema_version: PLUGIN_RUNTIME_SCHEMA_VERSION.into(),
            backup_version: PLUGIN_PRODUCT_BACKUP_VERSION.into(),
            backup_id: PluginBackupId::from("backup-1"),
            source_plugin_product_id: PluginProductId::from("plugin-1"),
            source_state: PluginBackupSourceState::Disabled,
            owner_quiescent: true,
            product_metadata_digest: digest("product"),
            source_archive_digest: digest("source"),
            release_inventory_digest: digest("releases"),
            config_digest: digest("config"),
            kv_digest: digest("kv"),
            files_digest: digest("files"),
            private_database_digest: digest("db"),
            migration_ledger_digest: digest("ledger"),
            credential_slot_keys: BTreeSet::from(["api_key".into()]),
            created_at_ms: 1,
        };
        metadata.validate().unwrap();
        metadata.owner_quiescent = false;
        assert!(metadata.validate().is_err());
    }

    #[test]
    fn deleting_intent_revokes_surface_and_catalog() {
        let mut state = pointer_state();
        state.ready_release = None;
        let record = PluginProductLifecycleRecord {
            plugin_product_id: state.plugin_product_id.clone(),
            state: PluginProductLifecycleState::Deleting,
            pointer_state: state,
            surface_available: false,
            catalog_published: false,
            deleting_intent: Some(PluginProductDeletingIntent {
                plugin_product_id: PluginProductId::from("plugin-1"),
                operation_id: OperationId::from("delete-1"),
                started_at_ms: 1,
                last_error: None,
            }),
        };
        record.validate().unwrap();
        let mut leaked = record;
        leaked.catalog_published = true;
        assert!(leaked.validate().is_err());
    }

    #[test]
    fn serde_rejects_unknown_fields_and_schema_is_available() {
        let value = json!({
            "entrypoint": "ui/index.html",
            "entrypoint_digest": digest("index"),
            "ui_tree_digest": digest("tree"),
            "localhost_port": 3211
        });
        assert!(serde_json::from_value::<PluginUiReleaseDescriptor>(value).is_err());
        let schema = schema_for!(PluginReleaseV1Manifest);
        assert!(serde_json::to_value(schema).unwrap().is_object());
    }
}
