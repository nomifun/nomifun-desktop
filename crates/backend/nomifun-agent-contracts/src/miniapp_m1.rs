//! Phase M1 MiniApp machine contracts.
//!
//! This module freezes immutable Release, Service, Bridge, storage, publish,
//! sharing, backup, and deletion shapes. It intentionally contains no runtime,
//! database, filesystem, or product-service implementation.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ArtifactEnvelope, ArtifactId, CanonicalErrorCode, DigestHex,
    JavaScriptBuildProfile, LocalizedMetadata, MiniAppId, MiniAppReleaseId,
    OperationId, PackageContributions, PackageRef, RuntimeInstallationId,
    RuntimeTarget, StrictJsonValue, VersionString, digest_payload,
};

pub const MINIAPP_M1_SCHEMA_VERSION: &str = "1.0.0";
pub const MINIAPP_RELEASE_PROFILE_VERSION: &str = "1.0.0";
pub const MINIAPP_SERVICE_HOST_PROTOCOL_VERSION: &str = "1.0.0";
pub const MINIAPP_SERVICE_SDK_CONTRACT_VERSION: &str = "1.0.0";
pub const MINIAPP_BRIDGE_CONTRACT_VERSION: &str = "1.0.0";
pub const MINIAPP_SERVICE_TEST_CONTRACT_VERSION: &str = "1.0.0";
pub const MINIAPP_SHARE_BUNDLE_VERSION: &str = "1.0.0";
pub const MINIAPP_WHOLE_APP_BACKUP_VERSION: &str = "1.0.0";

pub type MiniAppM1ContractArtifact = ArtifactEnvelope<MiniAppM1ContractManifest>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppM1ContractManifest {
    pub schema_version: VersionString,
    pub release_profile_version: VersionString,
    pub service_host_protocol_version: VersionString,
    pub service_sdk_contract_version: VersionString,
    pub bridge_contract_version: VersionString,
    pub service_test_contract_version: VersionString,
    pub share_bundle_version: VersionString,
    pub whole_app_backup_version: VersionString,
    pub service_lifecycles: BTreeSet<MiniAppServiceLifecycle>,
    pub bridge_transports: BTreeSet<MiniAppBridgeTransport>,
    pub max_services_per_miniapp: u8,
    pub ui_only_starts_node: bool,
    pub exposes_localhost_bridge: bool,
}

impl MiniAppM1ContractManifest {
    pub fn canonical() -> Self {
        Self {
            schema_version: MINIAPP_M1_SCHEMA_VERSION.into(),
            release_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
            service_host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
            service_sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
            bridge_contract_version: MINIAPP_BRIDGE_CONTRACT_VERSION.into(),
            service_test_contract_version: MINIAPP_SERVICE_TEST_CONTRACT_VERSION.into(),
            share_bundle_version: MINIAPP_SHARE_BUNDLE_VERSION.into(),
            whole_app_backup_version: MINIAPP_WHOLE_APP_BACKUP_VERSION.into(),
            service_lifecycles: BTreeSet::from([
                MiniAppServiceLifecycle::OnDemand,
                MiniAppServiceLifecycle::Continuous,
            ]),
            bridge_transports: BTreeSet::from([
                MiniAppBridgeTransport::MessageChannelV1,
            ]),
            max_services_per_miniapp: 1,
            ui_only_starts_node: false,
            exposes_localhost_bridge: false,
        }
    }

    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        if self != &Self::canonical() {
            return Err(invalid(
                "miniapp_m1_contract",
                "manifest differs from the frozen Phase M1 exact set",
            ));
        }
        Ok(())
    }
}

macro_rules! local_string_newtype {
    ($name:ident) => {
        #[derive(
            Clone,
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
        #[serde(transparent)]
        pub struct $name(pub String);

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

local_string_newtype!(MiniAppProjectId);
local_string_newtype!(MiniAppServiceTestReceiptId);
local_string_newtype!(MiniAppBridgeSessionId);
local_string_newtype!(MiniAppSurfaceSessionId);
local_string_newtype!(MiniAppBridgeCallId);
local_string_newtype!(MiniAppKvHandleId);
local_string_newtype!(MiniAppFilesHandleId);
local_string_newtype!(MiniAppDatabaseHandleId);
local_string_newtype!(MiniAppMigrationId);
local_string_newtype!(MiniAppBackupId);
local_string_newtype!(MiniAppShareBundleId);
local_string_newtype!(MiniAppUserAuthorizationId);

pub type MiniAppReleaseManifestArtifact = ArtifactEnvelope<MiniAppReleaseV1Manifest>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppUiReleaseDescriptor {
    pub entrypoint: String,
    pub entrypoint_digest: DigestHex,
    pub ui_tree_digest: DigestHex,
}

impl MiniAppUiReleaseDescriptor {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        if self.entrypoint != "ui/index.html" {
            return Err(invalid(
                "ui.entrypoint",
                "MiniApp UI entrypoint must be ui/index.html",
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
pub enum MiniAppServiceLifecycle {
    OnDemand,
    Continuous,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppServiceReleaseDescriptor {
    pub entrypoint: String,
    pub module_digest: DigestHex,
    pub lifecycle: MiniAppServiceLifecycle,
    pub uses_files: bool,
    pub uses_private_database: bool,
    pub service_contract_digest: DigestHex,
    pub host_protocol_version: VersionString,
    pub sdk_contract_version: VersionString,
    pub runtime_requirements_digest: DigestHex,
}

impl MiniAppServiceReleaseDescriptor {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        if self.entrypoint != "service/main.mjs" {
            return Err(invalid(
                "service.entrypoint",
                "MiniApp Service entrypoint must be service/main.mjs",
            ));
        }
        validate_digest(&self.module_digest, "service.module_digest")?;
        validate_digest(
            &self.service_contract_digest,
            "service.service_contract_digest",
        )?;
        require_version(
            &self.host_protocol_version,
            MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
            "service.host_protocol_version",
        )?;
        require_version(
            &self.sdk_contract_version,
            MINIAPP_SERVICE_SDK_CONTRACT_VERSION,
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
pub struct MiniAppMigrationColumn {
    pub name: String,
    pub declared_type: String,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_literal: Option<String>,
}

impl MiniAppMigrationColumn {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
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
pub enum MiniAppAdditiveMigrationAction {
    CreateTable {
        table_name: String,
        columns: Vec<MiniAppMigrationColumn>,
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
        column: MiniAppMigrationColumn,
    },
}

impl MiniAppAdditiveMigrationAction {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
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
struct MiniAppMigrationDigestInput<'a> {
    migration_id: &'a MiniAppMigrationId,
    actions: &'a [MiniAppAdditiveMigrationAction],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppMigration {
    pub migration_id: MiniAppMigrationId,
    pub migration_digest: DigestHex,
    pub actions: Vec<MiniAppAdditiveMigrationAction>,
}

impl MiniAppMigration {
    pub fn new(
        migration_id: MiniAppMigrationId,
        actions: Vec<MiniAppAdditiveMigrationAction>,
    ) -> Result<Self, MiniAppM1ContractError> {
        let migration_digest = digest_payload(&MiniAppMigrationDigestInput {
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

    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
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
        let expected = digest_payload(&MiniAppMigrationDigestInput {
            migration_id: &self.migration_id,
            actions: &self.actions,
        })?;
        if expected != self.migration_digest {
            return Err(MiniAppM1ContractError::DigestMismatch {
                field: "migration_digest",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppReleaseV1Manifest {
    pub schema_version: VersionString,
    pub build_profile: JavaScriptBuildProfile,
    pub build_profile_version: VersionString,
    pub display: LocalizedMetadata,
    pub ui: MiniAppUiReleaseDescriptor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<MiniAppServiceReleaseDescriptor>,
    pub dependency_lock_digest: DigestHex,
    pub dependency_graph_digest: DigestHex,
    pub config_schema_digest: DigestHex,
    pub credential_slots_digest: DigestHex,
    pub resource_contract_digest: DigestHex,
    pub bridge_contract_digest: DigestHex,
    pub contribution_package: PackageRef,
    pub contributions: PackageContributions,
    pub migrations: Vec<MiniAppMigration>,
}

impl MiniAppReleaseV1Manifest {
    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        require_version(
            &self.schema_version,
            MINIAPP_M1_SCHEMA_VERSION,
            "schema_version",
        )?;
        if self.build_profile != JavaScriptBuildProfile::MiniAppReleaseV1 {
            return Err(invalid(
                "build_profile",
                "MiniApp Release must use miniapp_release_v1",
            ));
        }
        require_version(
            &self.build_profile_version,
            MINIAPP_RELEASE_PROFILE_VERSION,
            "build_profile_version",
        )?;
        validate_display(&self.display)?;
        self.ui.validate()?;
        if let Some(service) = &self.service {
            service.validate()?;
            if !self.migrations.is_empty() && !service.uses_private_database {
                return Err(invalid(
                    "migrations",
                    "MiniApp migrations require the managed Private Database",
                ));
            }
        } else if !self.migrations.is_empty()
            || !self.contributions.capabilities.is_empty()
            || !self.contributions.mcp_tools.is_empty()
            || !self.contributions.role_contracts.is_empty()
            || !self.contributions.role_providers.is_empty()
        {
            return Err(invalid(
                "service",
                "UI-only MiniApps cannot declare Service migrations or executable contributions",
            ));
        }
        validate_digest(&self.dependency_lock_digest, "dependency_lock_digest")?;
        validate_digest(&self.dependency_graph_digest, "dependency_graph_digest")?;
        validate_digest(&self.config_schema_digest, "config_schema_digest")?;
        validate_digest(&self.credential_slots_digest, "credential_slots_digest")?;
        validate_digest(&self.resource_contract_digest, "resource_contract_digest")?;
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

    pub fn migration_set_digest(&self) -> Result<DigestHex, MiniAppM1ContractError> {
        Ok(digest_payload(&self.migrations)?)
    }

    pub fn contribution_set_digest(&self) -> Result<DigestHex, MiniAppM1ContractError> {
        Ok(digest_payload(&self.contributions)?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppReleaseFile {
    pub normalized_relative_path: String,
    pub digest: DigestHex,
    pub size_bytes: u64,
}

impl MiniAppReleaseFile {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_relative_path(
            &self.normalized_relative_path,
            "files.normalized_relative_path",
        )?;
        validate_digest(&self.digest, "files.digest")
    }
}

#[derive(Serialize)]
struct MiniAppReleaseArtifactDigestInput<'a> {
    manifest: &'a MiniAppReleaseManifestArtifact,
    files: &'a [MiniAppReleaseFile],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppReleaseArtifactV1 {
    pub artifact_id: ArtifactId,
    pub artifact_digest: DigestHex,
    pub manifest: MiniAppReleaseManifestArtifact,
    pub files: Vec<MiniAppReleaseFile>,
}

impl MiniAppReleaseArtifactV1 {
    pub fn new(
        artifact_id: ArtifactId,
        manifest: MiniAppReleaseV1Manifest,
        mut files: Vec<MiniAppReleaseFile>,
    ) -> Result<Self, MiniAppM1ContractError> {
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

    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.artifact_id.as_ref(), "artifact_id")?;
        validate_digest(&self.artifact_digest, "artifact_digest")?;
        if !self.manifest.verify()? {
            return Err(MiniAppM1ContractError::DigestMismatch {
                field: "manifest.payload_digest",
            });
        }
        self.manifest.payload.validate()?;
        if self.files.is_empty() {
            return Err(invalid(
                "files",
                "MiniApp Release must contain ui/index.html",
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
                            "MiniApp UI entrypoint must not be empty",
                        ));
                    }
                    ui_entrypoint_digest = Some(&file.digest);
                }
            } else if path == "service/main.mjs" {
                if file.size_bytes == 0 {
                    return Err(invalid(
                        "files.service/main.mjs",
                        "MiniApp Service entrypoint must not be empty",
                    ));
                }
                service_digest = Some(&file.digest);
            } else {
                return Err(invalid(
                    "files.normalized_relative_path",
                    "miniapp-release-v1 permits ui/** and optional service/main.mjs only",
                ));
            }
            previous = Some(path);
        }
        if ui_entrypoint_digest != Some(&self.manifest.payload.ui.entrypoint_digest) {
            return Err(MiniAppM1ContractError::DigestMismatch {
                field: "ui.entrypoint_digest",
            });
        }
        let expected_ui_tree = canonical_ui_tree_digest(&self.files)?;
        if expected_ui_tree != self.manifest.payload.ui.ui_tree_digest {
            return Err(MiniAppM1ContractError::DigestMismatch {
                field: "ui.ui_tree_digest",
            });
        }
        match (&self.manifest.payload.service, service_digest) {
            (None, None) => {}
            (Some(service), Some(observed)) if &service.module_digest == observed => {}
            (Some(_), None) => {
                return Err(invalid(
                    "files",
                    "Service manifest requires service/main.mjs",
                ));
            }
            (None, Some(_)) => {
                return Err(invalid(
                    "files",
                    "UI-only manifest cannot carry service/main.mjs",
                ));
            }
            (Some(_), Some(_)) => {
                return Err(MiniAppM1ContractError::DigestMismatch {
                    field: "service.module_digest",
                });
            }
        }
        let expected = canonical_release_artifact_digest(&self.manifest, &self.files)?;
        if expected != self.artifact_digest {
            return Err(MiniAppM1ContractError::DigestMismatch {
                field: "artifact_digest",
            });
        }
        Ok(())
    }
}

pub fn canonical_release_artifact_digest(
    manifest: &MiniAppReleaseManifestArtifact,
    files: &[MiniAppReleaseFile],
) -> Result<DigestHex, MiniAppM1ContractError> {
    Ok(digest_payload(&MiniAppReleaseArtifactDigestInput {
        manifest,
        files,
    })?)
}

pub fn canonical_ui_tree_digest(
    files: &[MiniAppReleaseFile],
) -> Result<DigestHex, MiniAppM1ContractError> {
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
pub struct MiniAppReleaseRef {
    pub release_id: MiniAppReleaseId,
    pub artifact_id: ArtifactId,
    pub release_digest: DigestHex,
    pub manifest_digest: DigestHex,
}

impl MiniAppReleaseRef {
    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.release_id.as_ref(), "release_id")?;
        validate_nonempty(self.artifact_id.as_ref(), "artifact_id")?;
        validate_digest(&self.release_digest, "release_digest")?;
        validate_digest(&self.manifest_digest, "manifest_digest")
    }

    pub fn validate_artifact(
        &self,
        artifact: &MiniAppReleaseArtifactV1,
    ) -> Result<(), MiniAppM1ContractError> {
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
pub enum MiniAppSourceLineage {
    Managed {
        project_id: MiniAppProjectId,
        source_snapshot_digest: DigestHex,
        dependency_lock_digest: DigestHex,
        build_profile_version: VersionString,
        build_generation: u64,
    },
    RuntimeOnly,
}

impl MiniAppSourceLineage {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
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
                    MINIAPP_RELEASE_PROFILE_VERSION,
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
pub enum MiniAppReadyOrigin {
    Build,
    Import,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppServiceTestReceiptRef {
    pub receipt_id: MiniAppServiceTestReceiptId,
    pub release_id: MiniAppReleaseId,
    pub release_digest: DigestHex,
    pub service_run_key: DigestHex,
}

impl MiniAppServiceTestReceiptRef {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.receipt_id.as_ref(), "test_receipt.receipt_id")?;
        validate_nonempty(self.release_id.as_ref(), "test_receipt.release_id")?;
        validate_digest(&self.release_digest, "test_receipt.release_digest")?;
        validate_digest(&self.service_run_key, "test_receipt.service_run_key")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppReadyRelease {
    pub miniapp_id: MiniAppId,
    pub release: MiniAppReleaseRef,
    pub origin_operation_id: OperationId,
    pub origin: MiniAppReadyOrigin,
    pub source_lineage: MiniAppSourceLineage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_service_test_receipt: Option<MiniAppServiceTestReceiptRef>,
    pub created_at_ms: i64,
}

impl MiniAppReadyRelease {
    pub fn validate_for_artifact(
        &self,
        artifact: &MiniAppReleaseArtifactV1,
    ) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.miniapp_id.as_ref(), "miniapp_id")?;
        validate_nonempty(
            self.origin_operation_id.as_ref(),
            "origin_operation_id",
        )?;
        self.release.validate_artifact(artifact)?;
        self.source_lineage.validate()?;
        if self.origin == MiniAppReadyOrigin::Build && !self.source_lineage.is_managed() {
            return Err(invalid(
                "source_lineage",
                "Build Ready Releases require managed source lineage",
            ));
        }
        if let MiniAppSourceLineage::Managed {
            dependency_lock_digest,
            ..
        } = &self.source_lineage
            && dependency_lock_digest != &artifact.manifest.payload.dependency_lock_digest
        {
            return Err(MiniAppM1ContractError::DigestMismatch {
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
pub struct MiniAppReadyReleaseRef {
    pub release_id: MiniAppReleaseId,
    pub release_digest: DigestHex,
}

impl MiniAppReadyReleaseRef {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.release_id.as_ref(), "ready_release.release_id")?;
        validate_digest(&self.release_digest, "ready_release.release_digest")
    }
}

impl From<&MiniAppReadyRelease> for MiniAppReadyReleaseRef {
    fn from(value: &MiniAppReadyRelease) -> Self {
        Self {
            release_id: value.release.release_id.clone(),
            release_digest: value.release.release_digest.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppReleasePointerState {
    pub miniapp_id: MiniAppId,
    pub pointer_revision: u64,
    pub active_release_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_release: Option<MiniAppReadyReleaseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_release: Option<MiniAppReleaseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_release: Option<MiniAppReleaseRef>,
    pub materialized_catalog_digest: DigestHex,
}

impl MiniAppReleasePointerState {
    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.miniapp_id.as_ref(), "miniapp_id")?;
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
                .is_some_and(|previous| previous.release_digest == active.release_digest)
                || self
                    .ready_release
                    .as_ref()
                    .is_some_and(|ready| ready.release_digest == active.release_digest)
        }) {
            return Err(invalid(
                "release_pointers",
                "Ready, Active, and Previous must not point to the same Release digest",
            ));
        }
        if self.previous_release.as_ref().is_some_and(|previous| {
            self.ready_release
                .as_ref()
                .is_some_and(|ready| ready.release_digest == previous.release_digest)
        }) {
            return Err(invalid(
                "release_pointers",
                "Ready, Active, and Previous must not point to the same Release digest",
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
pub struct MiniAppPointerExpectation {
    pub pointer_revision: u64,
    pub active_release_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_release: Option<MiniAppReadyReleaseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_release: Option<MiniAppReleaseRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_release: Option<MiniAppReleaseRef>,
    pub materialized_catalog_digest: DigestHex,
}

impl MiniAppPointerExpectation {
    pub fn from_state(state: &MiniAppReleasePointerState) -> Self {
        Self {
            pointer_revision: state.pointer_revision,
            active_release_epoch: state.active_release_epoch,
            ready_release: state.ready_release.clone(),
            active_release: state.active_release.clone(),
            previous_release: state.previous_release.clone(),
            materialized_catalog_digest: state.materialized_catalog_digest.clone(),
        }
    }

    fn matches(&self, state: &MiniAppReleasePointerState) -> bool {
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
pub struct MiniAppNonUiReleaseFingerprint {
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

impl MiniAppNonUiReleaseFingerprint {
    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        if let Some(service_run_key) = &self.service_run_key {
            validate_digest(service_run_key, "service_run_key")?;
        }
        for (field, digest) in [
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
pub struct MiniAppUiOnlyAutoPublishAuthorization {
    pub authorization_id: MiniAppUserAuthorizationId,
    pub miniapp_id: MiniAppId,
    pub enabled: bool,
    pub authorization_revision: u64,
    pub user_authorized_at_ms: i64,
}

impl MiniAppUiOnlyAutoPublishAuthorization {
    fn validate_for(&self, miniapp_id: &MiniAppId) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.authorization_id.as_ref(), "authorization_id")?;
        if &self.miniapp_id != miniapp_id || !self.enabled {
            return Err(invalid(
                "authorization",
                "auto Publish requires enabled user authorization for the exact MiniApp",
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
pub struct MiniAppUiOnlyAutoPublishProof {
    pub current_release: MiniAppReleaseRef,
    pub target_release: MiniAppReleaseRef,
    pub current_ui_tree_digest: DigestHex,
    pub target_ui_tree_digest: DigestHex,
    pub current_non_ui: MiniAppNonUiReleaseFingerprint,
    pub target_non_ui: MiniAppNonUiReleaseFingerprint,
    pub changed_source_paths: BTreeSet<String>,
    pub changed_output_paths: BTreeSet<String>,
    pub project_head_matches_ready_source: bool,
    pub static_validation_passed: bool,
    pub no_unknown_changes: bool,
}

impl MiniAppUiOnlyAutoPublishProof {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        self.current_release.validate()?;
        self.target_release.validate()?;
        validate_digest(&self.current_ui_tree_digest, "current_ui_tree_digest")?;
        validate_digest(&self.target_ui_tree_digest, "target_ui_tree_digest")?;
        self.current_non_ui.validate()?;
        self.target_non_ui.validate()?;
        if self.current_release.release_digest == self.target_release.release_digest
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
pub enum MiniAppPublishAuthorization {
    ManualUser { actor_id: String },
    AutoUiOnly {
        authorization: MiniAppUiOnlyAutoPublishAuthorization,
        proof: Box<MiniAppUiOnlyAutoPublishProof>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppPublishRequest {
    pub miniapp_id: MiniAppId,
    pub expected: MiniAppPointerExpectation,
    pub target_ready_release: MiniAppReleaseRef,
    pub target_catalog_digest: DigestHex,
    pub authorization: MiniAppPublishAuthorization,
}

impl MiniAppPublishRequest {
    pub fn validate_for(
        &self,
        state: &MiniAppReleasePointerState,
    ) -> Result<(), MiniAppM1ContractError> {
        state.validate()?;
        if self.miniapp_id != state.miniapp_id || !self.expected.matches(state) {
            return Err(MiniAppM1ContractError::CompareAndSwapConflict);
        }
        self.target_ready_release.validate()?;
        validate_digest(&self.target_catalog_digest, "target_catalog_digest")?;
        let target_ready = MiniAppReadyReleaseRef {
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
            MiniAppPublishAuthorization::ManualUser { actor_id } => {
                validate_nonempty(actor_id, "authorization.actor_id")
            }
            MiniAppPublishAuthorization::AutoUiOnly {
                authorization,
                proof,
            } => {
                authorization.validate_for(&self.miniapp_id)?;
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
        state: &MiniAppReleasePointerState,
    ) -> Result<MiniAppReleasePointerState, MiniAppM1ContractError> {
        self.validate_for(state)?;
        Ok(MiniAppReleasePointerState {
            miniapp_id: state.miniapp_id.clone(),
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
pub struct MiniAppRollbackRequest {
    pub miniapp_id: MiniAppId,
    pub expected: MiniAppPointerExpectation,
    pub rollback_target: MiniAppReleaseRef,
    pub target_catalog_digest: DigestHex,
    pub actor_id: String,
}

impl MiniAppRollbackRequest {
    pub fn validate_for(
        &self,
        state: &MiniAppReleasePointerState,
    ) -> Result<(), MiniAppM1ContractError> {
        state.validate()?;
        if self.miniapp_id != state.miniapp_id || !self.expected.matches(state) {
            return Err(MiniAppM1ContractError::CompareAndSwapConflict);
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
        state: &MiniAppReleasePointerState,
    ) -> Result<MiniAppReleasePointerState, MiniAppM1ContractError> {
        self.validate_for(state)?;
        Ok(MiniAppReleasePointerState {
            miniapp_id: state.miniapp_id.clone(),
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
pub struct MiniAppPointerCasCommit {
    pub miniapp_id: MiniAppId,
    pub before: MiniAppPointerExpectation,
    pub after: MiniAppReleasePointerState,
    pub committed_at_ms: i64,
}

impl MiniAppPointerCasCommit {
    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
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
        if self.miniapp_id != self.after.miniapp_id
            || self.after.pointer_revision != expected_revision
            || self.after.active_release_epoch != expected_epoch
            || self.committed_at_ms <= 0
        {
            return Err(invalid(
                "pointer_commit",
                "commit must advance the exact MiniApp pointer revision and active epoch once",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum MiniAppPointerCasOutcome {
    Committed {
        commit: Box<MiniAppPointerCasCommit>,
    },
    Conflict {
        observed_pointer_revision: u64,
        observed_active_release_epoch: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppKvHandleDescriptor {
    pub handle_id: MiniAppKvHandleId,
    pub miniapp_id: MiniAppId,
    pub namespace_revision: u64,
}

impl MiniAppKvHandleDescriptor {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.handle_id.as_ref(), "kv.handle_id")?;
        validate_nonempty(self.miniapp_id.as_ref(), "kv.miniapp_id")?;
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
pub enum MiniAppKvRequest {
    Get {
        handle: MiniAppKvHandleDescriptor,
        key: String,
    },
    Set {
        handle: MiniAppKvHandleDescriptor,
        key: String,
        value: StrictJsonValue,
    },
    Delete {
        handle: MiniAppKvHandleDescriptor,
        key: String,
    },
    CompareAndSwap {
        handle: MiniAppKvHandleDescriptor,
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected_revision: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<StrictJsonValue>,
    },
}

impl MiniAppKvRequest {
    pub fn validate_for(&self, miniapp_id: &MiniAppId) -> Result<(), MiniAppM1ContractError> {
        let (handle, key) = match self {
            Self::Get { handle, key }
            | Self::Set { handle, key, .. }
            | Self::Delete { handle, key }
            | Self::CompareAndSwap { handle, key, .. } => (handle, key),
        };
        handle.validate()?;
        if &handle.miniapp_id != miniapp_id {
            return Err(invalid(
                "kv.miniapp_id",
                "Host KV handle belongs to another MiniApp",
            ));
        }
        validate_state_key(key, "kv.key")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum MiniAppKvResponse {
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
pub struct MiniAppFilesDirDescriptor {
    pub handle_id: MiniAppFilesHandleId,
    pub miniapp_id: MiniAppId,
    pub absolute_path: String,
}

impl MiniAppFilesDirDescriptor {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.handle_id.as_ref(), "files_dir.handle_id")?;
        validate_nonempty(self.miniapp_id.as_ref(), "files_dir.miniapp_id")?;
        validate_absolute_path(&self.absolute_path, "files_dir.absolute_path")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppPrivateDatabaseDescriptor {
    pub handle_id: MiniAppDatabaseHandleId,
    pub miniapp_id: MiniAppId,
    pub schema_epoch: u64,
    pub migration_ledger_digest: DigestHex,
}

impl MiniAppPrivateDatabaseDescriptor {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.handle_id.as_ref(), "database.handle_id")?;
        validate_nonempty(self.miniapp_id.as_ref(), "database.miniapp_id")?;
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
pub struct MiniAppServiceStorageDescriptor {
    pub kv: MiniAppKvHandleDescriptor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_dir: Option<MiniAppFilesDirDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_database: Option<MiniAppPrivateDatabaseDescriptor>,
}

impl MiniAppServiceStorageDescriptor {
    fn validate_for(&self, miniapp_id: &MiniAppId) -> Result<(), MiniAppM1ContractError> {
        self.kv.validate()?;
        if let Some(files_dir) = &self.files_dir {
            files_dir.validate()?;
        }
        if let Some(private_database) = &self.private_database {
            private_database.validate()?;
        }
        if &self.kv.miniapp_id != miniapp_id
            || self
                .files_dir
                .as_ref()
                .is_some_and(|descriptor| &descriptor.miniapp_id != miniapp_id)
            || self
                .private_database
                .as_ref()
                .is_some_and(|descriptor| &descriptor.miniapp_id != miniapp_id)
        {
            return Err(invalid(
                "storage.miniapp_id",
                "all Service storage handles must belong to the exact MiniApp",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppServiceRuntimeFingerprint {
    pub runtime_installation_id: RuntimeInstallationId,
    pub runtime_target: RuntimeTarget,
    pub runtime_executable_digest: DigestHex,
    pub node_version: VersionString,
}

impl MiniAppServiceRuntimeFingerprint {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(
            self.runtime_installation_id.as_ref(),
            "runtime.runtime_installation_id",
        )?;
        validate_nonempty(self.runtime_target.as_ref(), "runtime.runtime_target")?;
        validate_digest(
            &self.runtime_executable_digest,
            "runtime.runtime_executable_digest",
        )?;
        validate_nonempty(self.node_version.as_ref(), "runtime.node_version")
    }
}

#[derive(Serialize)]
struct ResolvedMiniAppServiceSpecDigestInput<'a> {
    miniapp_id: &'a MiniAppId,
    service_module_digest: &'a DigestHex,
    lifecycle: MiniAppServiceLifecycle,
    host_protocol_version: &'a VersionString,
    sdk_contract_version: &'a VersionString,
    runtime: &'a MiniAppServiceRuntimeFingerprint,
    config_schema_digest: &'a DigestHex,
    config_snapshot_digest: &'a DigestHex,
    credential_slots_digest: &'a DigestHex,
    resource_contract_digest: &'a DigestHex,
    resource_bindings_digest: &'a DigestHex,
    runtime_requirements_digest: &'a DigestHex,
    bridge_contract_digest: &'a DigestHex,
    contribution_set_digest: &'a DigestHex,
    storage: &'a MiniAppServiceStorageDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedMiniAppServiceSpecInputs {
    pub miniapp_id: MiniAppId,
    pub release: MiniAppReleaseRef,
    pub active_release_epoch: u64,
    pub service_module_digest: DigestHex,
    pub lifecycle: MiniAppServiceLifecycle,
    pub host_protocol_version: VersionString,
    pub sdk_contract_version: VersionString,
    pub runtime: MiniAppServiceRuntimeFingerprint,
    pub config_schema_digest: DigestHex,
    pub config_snapshot_digest: DigestHex,
    pub credential_slots_digest: DigestHex,
    pub resource_contract_digest: DigestHex,
    pub resource_bindings_digest: DigestHex,
    pub runtime_requirements_digest: DigestHex,
    pub bridge_contract_digest: DigestHex,
    pub contribution_set_digest: DigestHex,
    pub storage: MiniAppServiceStorageDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedMiniAppServiceSpec {
    pub miniapp_id: MiniAppId,
    pub release: MiniAppReleaseRef,
    pub active_release_epoch: u64,
    pub service_module_digest: DigestHex,
    pub lifecycle: MiniAppServiceLifecycle,
    pub host_protocol_version: VersionString,
    pub sdk_contract_version: VersionString,
    pub runtime: MiniAppServiceRuntimeFingerprint,
    pub config_schema_digest: DigestHex,
    pub config_snapshot_digest: DigestHex,
    pub credential_slots_digest: DigestHex,
    pub resource_contract_digest: DigestHex,
    pub resource_bindings_digest: DigestHex,
    pub runtime_requirements_digest: DigestHex,
    pub bridge_contract_digest: DigestHex,
    pub contribution_set_digest: DigestHex,
    pub storage: MiniAppServiceStorageDescriptor,
    pub service_run_key: DigestHex,
}

impl ResolvedMiniAppServiceSpec {
    pub fn new(
        inputs: ResolvedMiniAppServiceSpecInputs,
    ) -> Result<Self, MiniAppM1ContractError> {
        let service_run_key = canonical_service_run_key(
            &inputs.miniapp_id,
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
        let ResolvedMiniAppServiceSpecInputs {
            miniapp_id,
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
            miniapp_id,
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

    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.miniapp_id.as_ref(), "miniapp_id")?;
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
            MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
            "host_protocol_version",
        )?;
        require_version(
            &self.sdk_contract_version,
            MINIAPP_SERVICE_SDK_CONTRACT_VERSION,
            "sdk_contract_version",
        )?;
        self.runtime.validate()?;
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
        self.storage.validate_for(&self.miniapp_id)?;
        validate_digest(&self.service_run_key, "service_run_key")?;
        let expected = canonical_service_run_key(
            &self.miniapp_id,
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
            return Err(MiniAppM1ContractError::DigestMismatch {
                field: "service_run_key",
            });
        }
        Ok(())
    }

    pub fn validate_for_release(
        &self,
        manifest: &MiniAppReleaseV1Manifest,
    ) -> Result<(), MiniAppM1ContractError> {
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
    miniapp_id: &MiniAppId,
    service_module_digest: &DigestHex,
    lifecycle: MiniAppServiceLifecycle,
    host_protocol_version: &VersionString,
    sdk_contract_version: &VersionString,
    runtime: &MiniAppServiceRuntimeFingerprint,
    config_schema_digest: &DigestHex,
    config_snapshot_digest: &DigestHex,
    credential_slots_digest: &DigestHex,
    resource_contract_digest: &DigestHex,
    resource_bindings_digest: &DigestHex,
    runtime_requirements_digest: &DigestHex,
    bridge_contract_digest: &DigestHex,
    contribution_set_digest: &DigestHex,
    storage: &MiniAppServiceStorageDescriptor,
) -> Result<DigestHex, MiniAppM1ContractError> {
    Ok(digest_payload(&ResolvedMiniAppServiceSpecDigestInput {
        miniapp_id,
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
pub enum MiniAppBridgeTransport {
    MessageChannelV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppBridgeSession {
    pub bridge_contract_version: VersionString,
    pub bridge_session_id: MiniAppBridgeSessionId,
    pub surface_session_id: MiniAppSurfaceSessionId,
    pub miniapp_id: MiniAppId,
    pub active_release: MiniAppReleaseRef,
    pub active_release_epoch: u64,
    pub transport: MiniAppBridgeTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_run_key: Option<DigestHex>,
}

impl MiniAppBridgeSession {
    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        require_version(
            &self.bridge_contract_version,
            MINIAPP_BRIDGE_CONTRACT_VERSION,
            "bridge_contract_version",
        )?;
        validate_nonempty(self.bridge_session_id.as_ref(), "bridge_session_id")?;
        validate_nonempty(self.surface_session_id.as_ref(), "surface_session_id")?;
        validate_nonempty(self.miniapp_id.as_ref(), "miniapp_id")?;
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
pub enum MiniAppBridgeTarget {
    HostKv {
        request: MiniAppBridgeKvRequest,
    },
    Service {
        method: String,
        payload: StrictJsonValue,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum MiniAppBridgeKvRequest {
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

impl MiniAppBridgeKvRequest {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
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
pub struct MiniAppBridgeRequest {
    pub call_id: MiniAppBridgeCallId,
    pub target: MiniAppBridgeTarget,
}

impl MiniAppBridgeRequest {
    pub fn validate_for(
        &self,
        session: &MiniAppBridgeSession,
        current: &MiniAppReleasePointerState,
    ) -> Result<(), MiniAppM1ContractError> {
        session.validate()?;
        current.validate()?;
        validate_nonempty(self.call_id.as_ref(), "call_id")?;
        if session.miniapp_id != current.miniapp_id
            || current.active_release.as_ref() != Some(&session.active_release)
            || current.active_release_epoch != session.active_release_epoch
        {
            return Err(MiniAppM1ContractError::StaleBridgeSession);
        }
        match &self.target {
            MiniAppBridgeTarget::HostKv { request } => request.validate(),
            MiniAppBridgeTarget::Service { method, payload } => {
                if session.service_run_key.is_none() {
                    return Err(invalid(
                        "target",
                        "UI-only MiniApp Bridge cannot target a Node Service",
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
pub enum MiniAppServiceTestOutcome {
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
pub enum MiniAppServiceTestCredentialMode {
    None,
    OneShotCurrentBindings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppServiceTestReceipt {
    pub receipt_id: MiniAppServiceTestReceiptId,
    pub miniapp_id: MiniAppId,
    pub release: MiniAppReleaseRef,
    pub service_run_key: DigestHex,
    pub outcome: MiniAppServiceTestOutcome,
    pub runtime: MiniAppServiceRuntimeFingerprint,
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
    pub credential_mode: MiniAppServiceTestCredentialMode,
    pub host_generation: u64,
    pub issued_at_ms: i64,
}

impl MiniAppServiceTestReceipt {
    pub fn validate_for(
        &self,
        ready: &MiniAppReadyRelease,
        resolved_spec: &ResolvedMiniAppServiceSpec,
    ) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.receipt_id.as_ref(), "receipt_id")?;
        validate_nonempty(self.miniapp_id.as_ref(), "miniapp_id")?;
        self.release.validate()?;
        validate_digest(&self.service_run_key, "service_run_key")?;
        self.runtime.validate()?;
        validate_nonempty(self.host_target.as_ref(), "host_target")?;
        if self.host_target != self.runtime.runtime_target {
            return Err(invalid(
                "host_target",
                "Service Test target must equal the selected Runtime target",
            ));
        }
        require_version(
            &self.host_protocol_version,
            MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
            "host_protocol_version",
        )?;
        require_version(
            &self.sdk_contract_version,
            MINIAPP_SERVICE_SDK_CONTRACT_VERSION,
            "sdk_contract_version",
        )?;
        require_version(
            &self.test_contract_version,
            MINIAPP_SERVICE_TEST_CONTRACT_VERSION,
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
        if self.miniapp_id != ready.miniapp_id
            || self.release != ready.release
            || self.miniapp_id != resolved_spec.miniapp_id
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

    pub fn reference(&self) -> MiniAppServiceTestReceiptRef {
        MiniAppServiceTestReceiptRef {
            receipt_id: self.receipt_id.clone(),
            release_id: self.release.release_id.clone(),
            release_digest: self.release.release_digest.clone(),
            service_run_key: self.service_run_key.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppSourceBundle {
    pub project_id: MiniAppProjectId,
    pub source_archive_artifact_id: ArtifactId,
    pub source_snapshot_digest: DigestHex,
    pub dependency_lock_artifact_id: ArtifactId,
    pub dependency_lock_digest: DigestHex,
    pub build_profile_version: VersionString,
}

impl MiniAppSourceBundle {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
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
            MINIAPP_RELEASE_PROFILE_VERSION,
            "source.build_profile_version",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppImportedTestProvenance {
    pub outcome: MiniAppServiceTestOutcome,
    pub release_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_run_key: Option<DigestHex>,
    pub runtime_target: RuntimeTarget,
    pub runtime_digest: DigestHex,
    pub test_contract_version: VersionString,
    pub issued_at_ms: i64,
}

impl MiniAppImportedTestProvenance {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_digest(&self.release_digest, "test.release_digest")?;
        if let Some(service_run_key) = &self.service_run_key {
            validate_digest(service_run_key, "test.service_run_key")?;
        }
        validate_nonempty(self.runtime_target.as_ref(), "test.runtime_target")?;
        validate_digest(&self.runtime_digest, "test.runtime_digest")?;
        require_version(
            &self.test_contract_version,
            MINIAPP_SERVICE_TEST_CONTRACT_VERSION,
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
struct MiniAppShareBundleDigestInput<'a> {
    bundle_id: &'a MiniAppShareBundleId,
    source_miniapp_id: &'a Option<MiniAppId>,
    release: &'a MiniAppReleaseArtifactV1,
    source: &'a Option<MiniAppSourceBundle>,
    test_provenance: &'a Option<MiniAppImportedTestProvenance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppShareBundleV1 {
    pub schema_version: VersionString,
    pub bundle_version: VersionString,
    pub bundle_id: MiniAppShareBundleId,
    pub bundle_digest: DigestHex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_miniapp_id: Option<MiniAppId>,
    pub release: MiniAppReleaseArtifactV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<MiniAppSourceBundle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_provenance: Option<MiniAppImportedTestProvenance>,
}

impl MiniAppShareBundleV1 {
    pub fn new(
        bundle_id: MiniAppShareBundleId,
        source_miniapp_id: Option<MiniAppId>,
        release: MiniAppReleaseArtifactV1,
        source: Option<MiniAppSourceBundle>,
        test_provenance: Option<MiniAppImportedTestProvenance>,
    ) -> Result<Self, MiniAppM1ContractError> {
        let bundle_digest = digest_payload(&MiniAppShareBundleDigestInput {
            bundle_id: &bundle_id,
            source_miniapp_id: &source_miniapp_id,
            release: &release,
            source: &source,
            test_provenance: &test_provenance,
        })?;
        let value = Self {
            schema_version: MINIAPP_M1_SCHEMA_VERSION.into(),
            bundle_version: MINIAPP_SHARE_BUNDLE_VERSION.into(),
            bundle_id,
            bundle_digest,
            source_miniapp_id,
            release,
            source,
            test_provenance,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        require_version(
            &self.schema_version,
            MINIAPP_M1_SCHEMA_VERSION,
            "schema_version",
        )?;
        require_version(
            &self.bundle_version,
            MINIAPP_SHARE_BUNDLE_VERSION,
            "bundle_version",
        )?;
        validate_nonempty(self.bundle_id.as_ref(), "bundle_id")?;
        validate_digest(&self.bundle_digest, "bundle_digest")?;
        self.release.validate()?;
        if let Some(source_miniapp_id) = &self.source_miniapp_id {
            validate_nonempty(source_miniapp_id.as_ref(), "source_miniapp_id")?;
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
        let expected = digest_payload(&MiniAppShareBundleDigestInput {
            bundle_id: &self.bundle_id,
            source_miniapp_id: &self.source_miniapp_id,
            release: &self.release,
            source: &self.source,
            test_provenance: &self.test_provenance,
        })?;
        if expected != self.bundle_digest {
            return Err(MiniAppM1ContractError::DigestMismatch {
                field: "bundle_digest",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppShareImportAsNewRequest {
    pub bundle_id: MiniAppShareBundleId,
    pub bundle_digest: DigestHex,
    pub new_miniapp_id: MiniAppId,
    pub expected_release_digest: DigestHex,
}

impl MiniAppShareImportAsNewRequest {
    pub fn validate_for(
        &self,
        bundle: &MiniAppShareBundleV1,
    ) -> Result<(), MiniAppM1ContractError> {
        bundle.validate()?;
        validate_nonempty(self.new_miniapp_id.as_ref(), "new_miniapp_id")?;
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
        if bundle.source_miniapp_id.as_ref() == Some(&self.new_miniapp_id) {
            return Err(invalid(
                "new_miniapp_id",
                "Share import always creates a new MiniApp identity",
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
pub enum MiniAppBackupSourceState {
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppWholeAppBackupMetadataV1 {
    pub schema_version: VersionString,
    pub backup_version: VersionString,
    pub backup_id: MiniAppBackupId,
    pub source_miniapp_id: MiniAppId,
    pub source_state: MiniAppBackupSourceState,
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

impl MiniAppWholeAppBackupMetadataV1 {
    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        require_version(
            &self.schema_version,
            MINIAPP_M1_SCHEMA_VERSION,
            "schema_version",
        )?;
        require_version(
            &self.backup_version,
            MINIAPP_WHOLE_APP_BACKUP_VERSION,
            "backup_version",
        )?;
        validate_nonempty(self.backup_id.as_ref(), "backup_id")?;
        validate_nonempty(self.source_miniapp_id.as_ref(), "source_miniapp_id")?;
        if !self.owner_quiescent {
            return Err(invalid(
                "owner_quiescent",
                "Whole-App Backup requires zero Service, Build, Test, Migration, and owner writers",
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

    pub fn metadata_digest(&self) -> Result<DigestHex, MiniAppM1ContractError> {
        self.validate()?;
        Ok(digest_payload(self)?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppBackupImportAsNewRequest {
    pub backup_id: MiniAppBackupId,
    pub backup_metadata_digest: DigestHex,
    pub new_miniapp_id: MiniAppId,
}

impl MiniAppBackupImportAsNewRequest {
    pub fn validate_for(
        &self,
        metadata: &MiniAppWholeAppBackupMetadataV1,
    ) -> Result<(), MiniAppM1ContractError> {
        metadata.validate()?;
        validate_nonempty(self.new_miniapp_id.as_ref(), "new_miniapp_id")?;
        validate_digest(
            &self.backup_metadata_digest,
            "backup_metadata_digest",
        )?;
        if self.backup_id != metadata.backup_id
            || self.backup_metadata_digest != metadata.metadata_digest()?
        {
            return Err(invalid(
                "backup_import",
                "import must bind the exact Whole-App Backup metadata",
            ));
        }
        if self.new_miniapp_id == metadata.source_miniapp_id {
            return Err(invalid(
                "new_miniapp_id",
                "Whole-App Backup import always creates a new MiniApp identity",
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
pub enum MiniAppProductLifecycleState {
    Enabled,
    Disabled,
    Trashed,
    Deleting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MiniAppDeletingIntent {
    pub miniapp_id: MiniAppId,
    pub operation_id: OperationId,
    pub started_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<CanonicalErrorCode>,
}

impl MiniAppDeletingIntent {
    fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.miniapp_id.as_ref(), "deleting.miniapp_id")?;
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
pub struct MiniAppProductLifecycleRecord {
    pub miniapp_id: MiniAppId,
    pub state: MiniAppProductLifecycleState,
    pub pointer_state: MiniAppReleasePointerState,
    pub surface_available: bool,
    pub catalog_published: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleting_intent: Option<MiniAppDeletingIntent>,
}

impl MiniAppProductLifecycleRecord {
    pub fn validate(&self) -> Result<(), MiniAppM1ContractError> {
        validate_nonempty(self.miniapp_id.as_ref(), "miniapp_id")?;
        self.pointer_state.validate()?;
        if self.pointer_state.miniapp_id != self.miniapp_id {
            return Err(invalid(
                "pointer_state.miniapp_id",
                "pointer state belongs to another MiniApp",
            ));
        }
        match (&self.state, &self.deleting_intent) {
            (MiniAppProductLifecycleState::Deleting, Some(intent)) => {
                intent.validate()?;
                if intent.miniapp_id != self.miniapp_id {
                    return Err(invalid(
                        "deleting_intent.miniapp_id",
                        "deleting intent belongs to another MiniApp",
                    ));
                }
            }
            (MiniAppProductLifecycleState::Deleting, None) => {
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
            MiniAppProductLifecycleState::Enabled => {
                if self.pointer_state.active_release.is_none()
                    || !self.surface_available
                    || !self.catalog_published
                {
                    return Err(invalid(
                        "enabled_state",
                        "enabled MiniApp requires Active Release, Surface, and materialized Catalog",
                    ));
                }
            }
            MiniAppProductLifecycleState::Disabled
            | MiniAppProductLifecycleState::Trashed
            | MiniAppProductLifecycleState::Deleting => {
                if self.surface_available || self.catalog_published {
                    return Err(invalid(
                        "inactive_state",
                        "disabled, trashed, or deleting MiniApp cannot expose Surface or Catalog",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum MiniAppM1ContractError {
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

fn invalid(field: &'static str, reason: impl Into<String>) -> MiniAppM1ContractError {
    MiniAppM1ContractError::InvalidField {
        field,
        reason: reason.into(),
    }
}

fn duplicate(field: &'static str, value: &str) -> MiniAppM1ContractError {
    MiniAppM1ContractError::DuplicateIdentity {
        field,
        value: value.to_owned(),
    }
}

fn require_version(
    value: &VersionString,
    expected: &str,
    field: &'static str,
) -> Result<(), MiniAppM1ContractError> {
    if value.as_ref() == expected {
        Ok(())
    } else {
        Err(invalid(
            field,
            format!("expected {expected}, observed {}", value.as_ref()),
        ))
    }
}

fn validate_nonempty(value: &str, field: &'static str) -> Result<(), MiniAppM1ContractError> {
    if value.is_empty() || value.trim() != value {
        Err(invalid(field, "value must be non-empty and trimmed"))
    } else {
        Ok(())
    }
}

fn validate_display(value: &LocalizedMetadata) -> Result<(), MiniAppM1ContractError> {
    validate_nonempty(&value.name, "display.name")?;
    validate_nonempty(&value.description, "display.description")
}

fn validate_digest(
    digest: &DigestHex,
    field: &'static str,
) -> Result<(), MiniAppM1ContractError> {
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
) -> Result<(), MiniAppM1ContractError> {
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
) -> Result<(), MiniAppM1ContractError> {
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
) -> Result<(), MiniAppM1ContractError> {
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
) -> Result<(), MiniAppM1ContractError> {
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
) -> Result<(), MiniAppM1ContractError> {
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
) -> Result<(), MiniAppM1ContractError> {
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

fn validate_contributions(
    package: &PackageRef,
    contributions: &PackageContributions,
) -> Result<(), MiniAppM1ContractError> {
    if !contributions.skills.is_empty()
        || !contributions.role_contracts.is_empty()
        || !contributions.role_providers.is_empty()
    {
        return Err(invalid(
            "contributions",
            "MiniApp M1 publishes executable Capability or MCP contributions only",
        ));
    }
    let mut capability_ids = BTreeSet::new();
    let mut contribution_ids = BTreeSet::new();
    for capability in &contributions.capabilities {
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
    use crate::{PackageId, digest_bytes};

    fn digest(seed: &str) -> DigestHex {
        digest_bytes(seed.as_bytes())
    }

    fn release_file(path: &str, seed: &str) -> MiniAppReleaseFile {
        MiniAppReleaseFile {
            normalized_relative_path: path.into(),
            digest: digest(seed),
            size_bytes: seed.len() as u64,
        }
    }

    fn manifest(files: &[MiniAppReleaseFile], with_service: bool) -> MiniAppReleaseV1Manifest {
        let entrypoint = files
            .iter()
            .find(|file| file.normalized_relative_path == "ui/index.html")
            .unwrap();
        let service = with_service.then(|| {
            let service = files
                .iter()
                .find(|file| file.normalized_relative_path == "service/main.mjs")
                .unwrap();
            MiniAppServiceReleaseDescriptor {
                entrypoint: "service/main.mjs".into(),
                module_digest: service.digest.clone(),
                lifecycle: MiniAppServiceLifecycle::OnDemand,
                uses_files: true,
                uses_private_database: true,
                service_contract_digest: digest("service-contract"),
                host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
                sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
                runtime_requirements_digest: digest("runtime-requirements"),
            }
        });
        MiniAppReleaseV1Manifest {
            schema_version: MINIAPP_M1_SCHEMA_VERSION.into(),
            build_profile: JavaScriptBuildProfile::MiniAppReleaseV1,
            build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
            display: LocalizedMetadata {
                name: "Example".into(),
                description: "Example MiniApp".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            ui: MiniAppUiReleaseDescriptor {
                entrypoint: "ui/index.html".into(),
                entrypoint_digest: entrypoint.digest.clone(),
                ui_tree_digest: canonical_ui_tree_digest(files).unwrap(),
            },
            service,
            dependency_lock_digest: digest("lock"),
            dependency_graph_digest: digest("graph"),
            config_schema_digest: digest("config"),
            credential_slots_digest: digest("credentials"),
            resource_contract_digest: digest("resources"),
            bridge_contract_digest: digest("bridge"),
            contribution_package: PackageRef {
                id: PackageId::from("miniapp.example.release"),
                version: VersionString::from("1.0.0"),
            },
            contributions: PackageContributions::default(),
            migrations: Vec::new(),
        }
    }

    fn artifact(with_service: bool) -> MiniAppReleaseArtifactV1 {
        let mut files = vec![
            release_file("ui/index.html", "index"),
            release_file("ui/app.js", "app"),
        ];
        if with_service {
            files.push(release_file("service/main.mjs", "service"));
        }
        MiniAppReleaseArtifactV1::new(
            ArtifactId::from("artifact-1"),
            manifest(&files, with_service),
            files,
        )
        .unwrap()
    }

    fn release_ref(seed: &str) -> MiniAppReleaseRef {
        MiniAppReleaseRef {
            release_id: MiniAppReleaseId::from(format!("release-{seed}")),
            artifact_id: ArtifactId::from(format!("artifact-{seed}")),
            release_digest: digest(&format!("release-{seed}")),
            manifest_digest: digest(&format!("manifest-{seed}")),
        }
    }

    fn pointer_state() -> MiniAppReleasePointerState {
        MiniAppReleasePointerState {
            miniapp_id: MiniAppId::from("miniapp-1"),
            pointer_revision: 7,
            active_release_epoch: 3,
            ready_release: Some(MiniAppReadyReleaseRef {
                release_id: MiniAppReleaseId::from("release-ready"),
                release_digest: digest("release-ready"),
            }),
            active_release: Some(release_ref("active")),
            previous_release: Some(release_ref("previous")),
            materialized_catalog_digest: digest("catalog-active"),
        }
    }

    fn runtime() -> MiniAppServiceRuntimeFingerprint {
        MiniAppServiceRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from("runtime-1"),
            runtime_target: RuntimeTarget::from("windows-x86_64"),
            runtime_executable_digest: digest("node"),
            node_version: VersionString::from("24.1.0"),
        }
    }

    fn storage() -> MiniAppServiceStorageDescriptor {
        MiniAppServiceStorageDescriptor {
            kv: MiniAppKvHandleDescriptor {
                handle_id: MiniAppKvHandleId::from("kv-1"),
                miniapp_id: MiniAppId::from("miniapp-1"),
                namespace_revision: 1,
            },
            files_dir: Some(MiniAppFilesDirDescriptor {
                handle_id: MiniAppFilesHandleId::from("files-1"),
                miniapp_id: MiniAppId::from("miniapp-1"),
                absolute_path: "C:\\NomiFun\\miniapps\\miniapp-1\\files".into(),
            }),
            private_database: Some(MiniAppPrivateDatabaseDescriptor {
                handle_id: MiniAppDatabaseHandleId::from("db-1"),
                miniapp_id: MiniAppId::from("miniapp-1"),
                schema_epoch: 2,
                migration_ledger_digest: digest("ledger"),
            }),
        }
    }

    fn resolved_spec() -> ResolvedMiniAppServiceSpec {
        ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
            miniapp_id: MiniAppId::from("miniapp-1"),
            release: release_ref("ready"),
            active_release_epoch: 4,
            service_module_digest: digest("service"),
            lifecycle: MiniAppServiceLifecycle::OnDemand,
            host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
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

    fn non_ui_fingerprint() -> MiniAppNonUiReleaseFingerprint {
        MiniAppNonUiReleaseFingerprint {
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
    fn canonical_contract_freezes_independent_m1_boundaries() {
        let contract = MiniAppM1ContractManifest::canonical();
        contract.validate().unwrap();
        assert_eq!(contract.max_services_per_miniapp, 1);
        assert!(!contract.ui_only_starts_node);
        assert!(!contract.exposes_localhost_bridge);
        assert_eq!(contract.service_lifecycles.len(), 2);
        assert_eq!(
            contract.bridge_transports,
            BTreeSet::from([MiniAppBridgeTransport::MessageChannelV1])
        );
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
        let migration = MiniAppMigration::new(
            MiniAppMigrationId::from("001_create_notes"),
            vec![MiniAppAdditiveMigrationAction::CreateTable {
                table_name: "notes".into(),
                columns: vec![MiniAppMigrationColumn {
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
        tampered.actions.push(MiniAppAdditiveMigrationAction::AddColumn {
            table_name: "notes".into(),
            column: MiniAppMigrationColumn {
                name: "title".into(),
                declared_type: "TEXT".into(),
                nullable: true,
                default_literal: None,
            },
        });
        assert!(matches!(
            tampered.validate(),
            Err(MiniAppM1ContractError::DigestMismatch { .. })
        ));

        let destructive_fragment = MiniAppMigrationColumn {
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
        changed.lifecycle = MiniAppServiceLifecycle::Continuous;
        assert!(matches!(
            changed.validate(),
            Err(MiniAppM1ContractError::DigestMismatch {
                field: "service_run_key"
            })
        ));
        let changed = ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
            miniapp_id: spec.miniapp_id.clone(),
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
        let spec = ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
            miniapp_id: MiniAppId::from("miniapp-1"),
            release: MiniAppReleaseRef {
                release_id: MiniAppReleaseId::from("release-1"),
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
        let session = MiniAppBridgeSession {
            bridge_contract_version: MINIAPP_BRIDGE_CONTRACT_VERSION.into(),
            bridge_session_id: MiniAppBridgeSessionId::from("bridge-1"),
            surface_session_id: MiniAppSurfaceSessionId::from("surface-1"),
            miniapp_id: MiniAppId::from("miniapp-1"),
            active_release: release_ref("active"),
            active_release_epoch: 4,
            transport: MiniAppBridgeTransport::MessageChannelV1,
            service_run_key: None,
        };
        let mut current = pointer_state();
        current.active_release = Some(session.active_release.clone());
        current.active_release_epoch = session.active_release_epoch;
        let request = MiniAppBridgeRequest {
            call_id: MiniAppBridgeCallId::from("call-1"),
            target: MiniAppBridgeTarget::Service {
                method: "notes.list".into(),
                payload: StrictJsonValue(json!({})),
            },
        };
        assert!(request.validate_for(&session, &current).is_err());
        current.active_release_epoch -= 1;
        assert!(matches!(
            request.validate_for(&session, &current),
            Err(MiniAppM1ContractError::StaleBridgeSession)
        ));
    }

    #[test]
    fn bridge_wire_contains_only_call_and_payload() {
        let request = MiniAppBridgeRequest {
            call_id: MiniAppBridgeCallId::from("call-1"),
            target: MiniAppBridgeTarget::HostKv {
                request: MiniAppBridgeKvRequest::Get {
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
            "miniapp_id",
            "active_release",
            "bridge_session_id",
            "surface_session_id",
            "localhost",
        ] {
            assert!(!wire.contains(forbidden));
        }
    }

    #[test]
    fn host_kv_handle_cannot_cross_owner() {
        let request = MiniAppKvRequest::Get {
            handle: MiniAppKvHandleDescriptor {
                handle_id: MiniAppKvHandleId::from("kv-1"),
                miniapp_id: MiniAppId::from("miniapp-other"),
                namespace_revision: 1,
            },
            key: "notes/current".into(),
        };
        assert!(request
            .validate_for(&MiniAppId::from("miniapp-1"))
            .is_err());
    }

    #[test]
    fn strict_auto_publish_rejects_first_publish_and_non_ui_change() {
        let state = pointer_state();
        let target = MiniAppReleaseRef {
            release_id: MiniAppReleaseId::from("release-ready"),
            artifact_id: ArtifactId::from("artifact-ready"),
            release_digest: digest("release-ready"),
            manifest_digest: digest("manifest-ready"),
        };
        let proof = MiniAppUiOnlyAutoPublishProof {
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
        let request = MiniAppPublishRequest {
            miniapp_id: state.miniapp_id.clone(),
            expected: MiniAppPointerExpectation::from_state(&state),
            target_ready_release: target,
            target_catalog_digest: digest("catalog-ready"),
            authorization: MiniAppPublishAuthorization::AutoUiOnly {
                authorization: MiniAppUiOnlyAutoPublishAuthorization {
                    authorization_id: MiniAppUserAuthorizationId::from("auth-1"),
                    miniapp_id: state.miniapp_id.clone(),
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
        first_request.expected = MiniAppPointerExpectation::from_state(&first);
        assert!(first_request.validate_for(&first).is_err());

        let mut changed_service = request;
        let MiniAppPublishAuthorization::AutoUiOnly { proof, .. } =
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
        let target = MiniAppReleaseRef {
            release_id: MiniAppReleaseId::from("release-ready"),
            artifact_id: ArtifactId::from("artifact-ready"),
            release_digest: digest("release-ready"),
            manifest_digest: digest("manifest-ready"),
        };
        let publish = MiniAppPublishRequest {
            miniapp_id: state.miniapp_id.clone(),
            expected: MiniAppPointerExpectation::from_state(&state),
            target_ready_release: target.clone(),
            target_catalog_digest: digest("catalog-ready"),
            authorization: MiniAppPublishAuthorization::ManualUser {
                actor_id: "user-1".into(),
            },
        };
        let published = publish.next_state(&state).unwrap();
        assert_eq!(published.active_release.as_ref(), Some(&target));
        assert_eq!(published.previous_release, state.active_release);
        assert!(published.ready_release.is_none());

        let rollback = MiniAppRollbackRequest {
            miniapp_id: published.miniapp_id.clone(),
            expected: MiniAppPointerExpectation::from_state(&published),
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
            Err(MiniAppM1ContractError::CompareAndSwapConflict)
        ));
    }

    #[test]
    fn service_test_receipt_binds_exact_ready_and_spec() {
        let spec = resolved_spec();
        let ready = MiniAppReadyRelease {
            miniapp_id: spec.miniapp_id.clone(),
            release: spec.release.clone(),
            origin_operation_id: OperationId::from("build-1"),
            origin: MiniAppReadyOrigin::Build,
            source_lineage: MiniAppSourceLineage::Managed {
                project_id: MiniAppProjectId::from("project-1"),
                source_snapshot_digest: digest("source"),
                dependency_lock_digest: digest("lock"),
                build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
                build_generation: 1,
            },
            matching_service_test_receipt: None,
            created_at_ms: 1,
        };
        let mut receipt = MiniAppServiceTestReceipt {
            receipt_id: MiniAppServiceTestReceiptId::from("receipt-1"),
            miniapp_id: spec.miniapp_id.clone(),
            release: spec.release.clone(),
            service_run_key: spec.service_run_key.clone(),
            outcome: MiniAppServiceTestOutcome::Passed,
            runtime: spec.runtime.clone(),
            host_target: spec.runtime.runtime_target.clone(),
            host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
            test_contract_version: MINIAPP_SERVICE_TEST_CONTRACT_VERSION.into(),
            resolved_test_input_digest: digest("inputs"),
            copied_kv_digest: digest("kv"),
            copied_private_database_digest: Some(digest("db")),
            empty_files_dir: Some(true),
            migration_ledger_digest: Some(digest("ledger")),
            credential_mode: MiniAppServiceTestCredentialMode::None,
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
        let source = MiniAppSourceBundle {
            project_id: MiniAppProjectId::from("project-1"),
            source_archive_artifact_id: ArtifactId::from("source-1"),
            source_snapshot_digest: digest("source"),
            dependency_lock_artifact_id: ArtifactId::from("lock-1"),
            dependency_lock_digest: release.manifest.payload.dependency_lock_digest.clone(),
            build_profile_version: release.manifest.payload.build_profile_version.clone(),
        };
        let bundle = MiniAppShareBundleV1::new(
            MiniAppShareBundleId::from("share-1"),
            Some(MiniAppId::from("miniapp-source")),
            release,
            Some(source),
            None,
        )
        .unwrap();
        let request = MiniAppShareImportAsNewRequest {
            bundle_id: bundle.bundle_id.clone(),
            bundle_digest: bundle.bundle_digest.clone(),
            new_miniapp_id: MiniAppId::from("miniapp-new"),
            expected_release_digest: bundle.release.artifact_digest.clone(),
        };
        request.validate_for(&bundle).unwrap();

        let same_identity = MiniAppShareImportAsNewRequest {
            new_miniapp_id: MiniAppId::from("miniapp-source"),
            ..request
        };
        assert!(same_identity.validate_for(&bundle).is_err());
    }

    #[test]
    fn whole_app_backup_is_separate_disabled_quiescent_metadata() {
        let mut metadata = MiniAppWholeAppBackupMetadataV1 {
            schema_version: MINIAPP_M1_SCHEMA_VERSION.into(),
            backup_version: MINIAPP_WHOLE_APP_BACKUP_VERSION.into(),
            backup_id: MiniAppBackupId::from("backup-1"),
            source_miniapp_id: MiniAppId::from("miniapp-1"),
            source_state: MiniAppBackupSourceState::Disabled,
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
        let record = MiniAppProductLifecycleRecord {
            miniapp_id: state.miniapp_id.clone(),
            state: MiniAppProductLifecycleState::Deleting,
            pointer_state: state,
            surface_available: false,
            catalog_published: false,
            deleting_intent: Some(MiniAppDeletingIntent {
                miniapp_id: MiniAppId::from("miniapp-1"),
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
        assert!(serde_json::from_value::<MiniAppUiReleaseDescriptor>(value).is_err());
        let schema = schema_for!(MiniAppReleaseV1Manifest);
        assert!(serde_json::to_value(schema).unwrap().is_object());
    }
}
