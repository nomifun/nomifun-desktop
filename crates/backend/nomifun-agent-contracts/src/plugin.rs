//! Canonical Unified Plugin Core author and host contract.
//!
//! This is intentionally the only Plugin package contract. It models a small
//! package manifest, content-addressed artifacts, stable Actions and Bindings;
//! build candidates, releases, mounts, products and publish state do not exist
//! in this contract.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{CanonicalDigestError, DigestHex, PluginId, StrictJsonValue, digest_payload};

pub const PLUGIN_MANIFEST_SCHEMA: &str = "nomifun.plugin/v1";
pub const PLUGIN_MANIFEST_PATH: &str = "nomifun.plugin.json";
pub const PLUGIN_HOST_API_MAJOR: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnifiedPluginContractManifest {
    pub manifest_schema: String,
    pub manifest_path: String,
    pub host_api_major: u64,
    pub action_identity: String,
    pub binding_points: BTreeSet<PluginBindingPoint>,
}

impl UnifiedPluginContractManifest {
    pub fn canonical() -> Self {
        Self {
            manifest_schema: PLUGIN_MANIFEST_SCHEMA.into(),
            manifest_path: PLUGIN_MANIFEST_PATH.into(),
            host_api_major: PLUGIN_HOST_API_MAJOR,
            action_identity: "plugin:<plugin_id>/<action_id>".into(),
            binding_points: BTreeSet::from([
                PluginBindingPoint::AgentTool,
                PluginBindingPoint::AgentContext,
                PluginBindingPoint::AgentBeforeModel,
                PluginBindingPoint::AgentBeforeTool,
                PluginBindingPoint::DesktopCommand,
                PluginBindingPoint::DesktopEvent,
                PluginBindingPoint::AutomationAction,
            ]),
        }
    }

    pub fn validate(&self) -> Result<(), PluginContractError> {
        if self != &Self::canonical() {
            return Err(PluginContractError::InvalidField {
                field: "unifiedPluginContract",
                reason: "must equal the canonical Unified Plugin Core contract".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PluginServiceMode {
    #[default]
    OnDemand,
    Continuous,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginEntrypoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(rename = "serviceMode", default, skip_serializing_if = "Option::is_none")]
    pub service_mode: Option<PluginServiceMode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginActionEffect {
    Read,
    Write,
    External,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginActionManifest {
    pub name: String,
    pub description: String,
    pub input: StrictJsonValue,
    pub output: StrictJsonValue,
    pub effect: PluginActionEffect,
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
#[serde(rename_all = "snake_case")]
pub enum PluginBindingPoint {
    #[serde(rename = "agent.tool")]
    AgentTool,
    #[serde(rename = "agent.context")]
    AgentContext,
    #[serde(rename = "agent.before_model")]
    AgentBeforeModel,
    #[serde(rename = "agent.before_tool")]
    AgentBeforeTool,
    #[serde(rename = "desktop.command")]
    DesktopCommand,
    #[serde(rename = "desktop.event")]
    DesktopEvent,
    #[serde(rename = "automation.action")]
    AutomationAction,
}

impl PluginBindingPoint {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentTool => "agent.tool",
            Self::AgentContext => "agent.context",
            Self::AgentBeforeModel => "agent.before_model",
            Self::AgentBeforeTool => "agent.before_tool",
            Self::DesktopCommand => "desktop.command",
            Self::DesktopEvent => "desktop.event",
            Self::AutomationAction => "automation.action",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginBindingManifest {
    pub point: PluginBindingPoint,
    pub action: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginMigrationManifest {
    pub id: String,
    pub from: u32,
    pub to: u32,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub schema: String,
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "hostApi")]
    pub host_api: String,
    pub entrypoints: PluginEntrypoints,
    #[serde(default)]
    pub actions: BTreeMap<String, PluginActionManifest>,
    #[serde(default)]
    pub bindings: Vec<PluginBindingManifest>,
    #[serde(rename = "dataVersion", default)]
    pub data_version: u32,
    #[serde(default)]
    pub migrations: Vec<PluginMigrationManifest>,
    #[serde(rename = "configSchema", default = "object_schema")]
    pub config_schema: StrictJsonValue,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub permissions: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

impl PluginManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, PluginContractError> {
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| PluginContractError::Json(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), PluginContractError> {
        if self.schema != PLUGIN_MANIFEST_SCHEMA {
            return Err(PluginContractError::InvalidField {
                field: "schema",
                reason: format!("must equal {PLUGIN_MANIFEST_SCHEMA}"),
            });
        }
        validate_machine_id(&self.id, "id", 160, true)?;
        Version::parse(&self.version).map_err(|error| PluginContractError::InvalidField {
            field: "version",
            reason: format!("must be SemVer: {error}"),
        })?;
        validate_text(&self.name, "name", 160)?;
        validate_text(&self.description, "description", 4_096)?;
        validate_host_api(&self.host_api)?;

        if self.entrypoints.ui.is_none() && self.entrypoints.service.is_none() {
            return Err(PluginContractError::InvalidField {
                field: "entrypoints",
                reason: "ui or service is required".into(),
            });
        }
        if self.entrypoints.service.is_none() && self.entrypoints.service_mode.is_some() {
            return Err(PluginContractError::InvalidField {
                field: "entrypoints.serviceMode",
                reason: "requires a service entrypoint".into(),
            });
        }
        if let Some(path) = &self.entrypoints.ui {
            validate_package_path(path, "entrypoints.ui")?;
            if path != "ui/index.html" {
                return Err(PluginContractError::InvalidField {
                    field: "entrypoints.ui",
                    reason: "must equal ui/index.html".into(),
                });
            }
        }
        if let Some(path) = &self.entrypoints.service {
            validate_package_path(path, "entrypoints.service")?;
            if path != "service/main.mjs" {
                return Err(PluginContractError::InvalidField {
                    field: "entrypoints.service",
                    reason: "must equal service/main.mjs".into(),
                });
            }
        }
        if !self.actions.is_empty() && self.entrypoints.service.is_none() {
            return Err(PluginContractError::InvalidField {
                field: "actions",
                reason: "Actions require service/main.mjs".into(),
            });
        }

        for (id, action) in &self.actions {
            validate_machine_id(id, "actions.<id>", 96, false)?;
            validate_text(&action.name, "actions.<id>.name", 160)?;
            validate_text(&action.description, "actions.<id>.description", 2_048)?;
            validate_schema_object(&action.input, "actions.<id>.input")?;
            validate_schema_object(&action.output, "actions.<id>.output")?;
        }
        let mut bindings = BTreeSet::new();
        for binding in &self.bindings {
            if !self.actions.contains_key(&binding.action) {
                return Err(PluginContractError::InvalidField {
                    field: "bindings.action",
                    reason: format!("references unknown Action {}", binding.action),
                });
            }
            if !bindings.insert((binding.point, binding.action.as_str())) {
                return Err(PluginContractError::InvalidField {
                    field: "bindings",
                    reason: format!(
                        "duplicates {} -> {}",
                        binding.point.as_str(),
                        binding.action
                    ),
                });
            }
        }

        let mut migration_ids = BTreeSet::new();
        let mut migration_steps = BTreeSet::new();
        for migration in &self.migrations {
            validate_machine_id(&migration.id, "migrations.id", 128, false)?;
            validate_package_path(&migration.path, "migrations.path")?;
            if !migration.path.starts_with("migrations/") || !migration.path.ends_with(".mjs") {
                return Err(PluginContractError::InvalidField {
                    field: "migrations.path",
                    reason: "must be migrations/*.mjs".into(),
                });
            }
            if migration.to != migration.from.saturating_add(1)
                || migration.to > self.data_version
            {
                return Err(PluginContractError::InvalidField {
                    field: "migrations",
                    reason: "each migration must advance one version and not exceed dataVersion"
                        .into(),
                });
            }
            if !migration_ids.insert(&migration.id)
                || !migration_steps.insert((migration.from, migration.to))
            {
                return Err(PluginContractError::InvalidField {
                    field: "migrations",
                    reason: "migration ids and version steps must be unique".into(),
                });
            }
        }
        validate_schema_object(&self.config_schema, "configSchema")?;

        let mut secrets = BTreeSet::new();
        for secret in &self.secrets {
            validate_machine_id(secret, "secrets", 96, false)?;
            if !secrets.insert(secret) {
                return Err(PluginContractError::InvalidField {
                    field: "secrets",
                    reason: format!("duplicates slot {secret}"),
                });
            }
        }
        if schema_declares_secret_property(&self.config_schema.0, &secrets) {
            return Err(PluginContractError::InvalidField {
                field: "configSchema",
                reason: "Credential slots cannot also be Config properties".into(),
            });
        }
        for permission in &self.permissions {
            validate_machine_id(permission, "permissions", 160, true)?;
        }
        for namespace in self.extensions.keys() {
            if !namespace.contains('.') {
                return Err(PluginContractError::InvalidField {
                    field: "extensions",
                    reason: format!("extension key {namespace} must be namespaced"),
                });
            }
            validate_machine_id(namespace, "extensions", 160, true)?;
        }
        Ok(())
    }

    pub fn has_ui(&self) -> bool {
        self.entrypoints.ui.is_some()
    }

    pub fn has_service(&self) -> bool {
        self.entrypoints.service.is_some()
    }

    pub fn service_mode(&self) -> PluginServiceMode {
        self.entrypoints.service_mode.unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginArtifactFile {
    #[serde(rename = "path")]
    pub normalized_relative_path: String,
    pub digest: DigestHex,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginArtifact {
    pub artifact_digest: DigestHex,
    pub manifest: PluginManifest,
    pub files: Vec<PluginArtifactFile>,
}

#[derive(Serialize)]
struct PluginArtifactDigestInput<'a> {
    manifest: &'a PluginManifest,
    files: &'a [PluginArtifactFile],
}

impl PluginArtifact {
    pub fn new(
        manifest: PluginManifest,
        mut files: Vec<PluginArtifactFile>,
    ) -> Result<Self, PluginContractError> {
        manifest.validate()?;
        files.sort_by(|left, right| {
            left.normalized_relative_path
                .cmp(&right.normalized_relative_path)
        });
        validate_artifact_files(&manifest, &files)?;
        let artifact_digest = digest_payload(&PluginArtifactDigestInput {
            manifest: &manifest,
            files: &files,
        })?;
        Ok(Self {
            artifact_digest,
            manifest,
            files,
        })
    }

    pub fn validate(&self) -> Result<(), PluginContractError> {
        self.manifest.validate()?;
        validate_artifact_files(&self.manifest, &self.files)?;
        let expected = digest_payload(&PluginArtifactDigestInput {
            manifest: &self.manifest,
            files: &self.files,
        })?;
        if expected != self.artifact_digest {
            return Err(PluginContractError::DigestMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginArtifactRef {
    pub digest: DigestHex,
    pub package_id: String,
    pub version: String,
}

impl From<&PluginArtifact> for PluginArtifactRef {
    fn from(artifact: &PluginArtifact) -> Self {
        Self {
            digest: artifact.artifact_digest.clone(),
            package_id: artifact.manifest.id.clone(),
            version: artifact.manifest.version.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginActionPublication {
    pub plugin_id: PluginId,
    pub artifact: PluginArtifactRef,
    pub action_id: String,
    pub action: PluginActionManifest,
    pub bindings: BTreeSet<PluginBindingPoint>,
}

impl PluginActionPublication {
    pub fn stable_id(&self) -> String {
        plugin_action_id(&self.plugin_id, &self.action_id)
    }
}

pub fn plugin_action_id(plugin_id: &PluginId, action_id: &str) -> String {
    format!("plugin:{}/{action_id}", plugin_id.as_ref())
}

#[derive(Debug, Error)]
pub enum PluginContractError {
    #[error("invalid Plugin JSON: {0}")]
    Json(String),
    #[error("invalid Plugin field {field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: String,
    },
    #[error("Plugin Artifact digest does not match its manifest and files")]
    DigestMismatch,
    #[error(transparent)]
    Digest(#[from] CanonicalDigestError),
}

fn object_schema() -> StrictJsonValue {
    StrictJsonValue(serde_json::json!({"type": "object"}))
}

fn validate_schema_object(
    value: &StrictJsonValue,
    field: &'static str,
) -> Result<(), PluginContractError> {
    if !value.0.is_object() {
        return Err(PluginContractError::InvalidField {
            field,
            reason: "must be an inline JSON Schema object".into(),
        });
    }
    if contains_external_schema_reference(&value.0) {
        return Err(PluginContractError::InvalidField {
            field,
            reason: "must not contain an external JSON Schema $ref".into(),
        });
    }
    Ok(())
}

fn contains_external_schema_reference(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "$ref"
                && value
                    .as_str()
                    .is_none_or(|reference| !reference.starts_with('#')))
                || contains_external_schema_reference(value)
        }),
        serde_json::Value::Array(values) => {
            values.iter().any(contains_external_schema_reference)
        }
        _ => false,
    }
}

fn schema_declares_secret_property(
    value: &serde_json::Value,
    secrets: &BTreeSet<&String>,
) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|properties| {
                    properties
                        .keys()
                        .any(|property| secrets.iter().any(|secret| secret.as_str() == property))
                })
                || object
                    .values()
                    .any(|value| schema_declares_secret_property(value, secrets))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| schema_declares_secret_property(value, secrets)),
        _ => false,
    }
}

fn validate_text(
    value: &str,
    field: &'static str,
    maximum: usize,
) -> Result<(), PluginContractError> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(PluginContractError::InvalidField {
            field,
            reason: format!("must be non-empty text no longer than {maximum} bytes"),
        });
    }
    Ok(())
}

fn validate_machine_id(
    value: &str,
    field: &'static str,
    maximum: usize,
    allow_dot: bool,
) -> Result<(), PluginContractError> {
    let valid = !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-')
                || (allow_dot && byte == b'.')
        })
        && value.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase());
    if !valid {
        return Err(PluginContractError::InvalidField {
            field,
            reason: "must be a lowercase machine identifier".into(),
        });
    }
    Ok(())
}

fn validate_host_api(value: &str) -> Result<(), PluginContractError> {
    let compact = value.replace(' ', ",");
    let compact = compact.trim_matches(',');
    let requirement = semver::VersionReq::parse(compact).map_err(|error| {
        PluginContractError::InvalidField {
            field: "hostApi",
            reason: format!("must be a semantic version requirement: {error}"),
        }
    })?;
    if !requirement.matches(&Version::new(PLUGIN_HOST_API_MAJOR, 0, 0)) {
        return Err(PluginContractError::InvalidField {
            field: "hostApi",
            reason: format!("does not include Host API {PLUGIN_HOST_API_MAJOR}.0.0"),
        });
    }
    Ok(())
}

fn validate_package_path(
    value: &str,
    field: &'static str,
) -> Result<(), PluginContractError> {
    let valid = !value.is_empty()
        && value.len() <= 1_024
        && !value.starts_with('/')
        && !value.starts_with('\\')
        && !value.contains('\\')
        && !value.contains(':')
        && !value.chars().any(char::is_control)
        && value.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    if !valid {
        return Err(PluginContractError::InvalidField {
            field,
            reason: "must be a normalized portable relative path".into(),
        });
    }
    Ok(())
}

fn validate_artifact_files(
    manifest: &PluginManifest,
    files: &[PluginArtifactFile],
) -> Result<(), PluginContractError> {
    let mut prior: Option<&str> = None;
    let mut paths = BTreeSet::new();
    for file in files {
        validate_package_path(&file.normalized_relative_path, "files.path")?;
        if file.digest.as_ref().len() != 64
            || !file.digest.as_ref().bytes().all(|byte| byte.is_ascii_hexdigit())
            || file.size_bytes == 0
            || prior.is_some_and(|value| value >= file.normalized_relative_path.as_str())
        {
            return Err(PluginContractError::InvalidField {
                field: "files",
                reason: "must be sorted unique non-empty files with SHA-256 digests".into(),
            });
        }
        prior = Some(&file.normalized_relative_path);
        paths.insert(file.normalized_relative_path.as_str());
    }
    for required in [
        Some(PLUGIN_MANIFEST_PATH),
        manifest.entrypoints.ui.as_deref(),
        manifest.entrypoints.service.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !paths.contains(required) {
            return Err(PluginContractError::InvalidField {
                field: "files",
                reason: format!("missing declared file {required}"),
            });
        }
    }
    for migration in &manifest.migrations {
        if !paths.contains(migration.path.as_str()) {
            return Err(PluginContractError::InvalidField {
                field: "files",
                reason: format!("missing migration file {}", migration.path),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> PluginManifest {
        PluginManifest::parse(
            br#"{
              "schema":"nomifun.plugin/v1",
              "id":"local.todo",
              "version":"1.0.0",
              "name":"Todo",
              "description":"A todo app",
              "hostApi":">=1 <2",
              "entrypoints":{"ui":"ui/index.html","service":"service/main.mjs","serviceMode":"onDemand"},
              "actions":{"add_task":{"name":"Add task","description":"Adds a task","input":{"type":"object"},"output":{"type":"object"},"effect":"write"}},
              "bindings":[{"point":"agent.tool","action":"add_task"}],
              "dataVersion":1,
              "migrations":[],
              "configSchema":{"type":"object"},
              "secrets":["api_key"],
              "permissions":["network"]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn canonical_manifest_matches_the_author_contract() {
        let manifest = manifest();
        assert!(manifest.has_ui());
        assert!(manifest.has_service());
        assert_eq!(manifest.service_mode(), PluginServiceMode::OnDemand);
        assert_eq!(manifest.bindings[0].point, PluginBindingPoint::AgentTool);
    }

    #[test]
    fn action_and_binding_contract_is_closed_and_local() {
        let mut invalid_binding = manifest();
        invalid_binding.bindings[0].action = "missing".into();
        assert!(invalid_binding.validate().is_err());
        assert!(PluginManifest::parse(br#"{"schema":"nomifun.plugin/v1","legacy":true}"#).is_err());

        let mut external_schema = manifest();
        external_schema.actions.get_mut("add_task").unwrap().input = StrictJsonValue(
            serde_json::json!({"$ref":"https://schemas.invalid/task.json"}),
        );
        assert!(external_schema.validate().is_err());

        let mut local_schema = manifest();
        local_schema.actions.get_mut("add_task").unwrap().input = StrictJsonValue(
            serde_json::json!({
                "$defs":{"task":{"type":"object"}},
                "$ref":"#/$defs/task"
            }),
        );
        local_schema.validate().unwrap();

        let mut secret_config = manifest();
        secret_config.config_schema = StrictJsonValue(serde_json::json!({
            "type":"object",
            "properties":{"api_key":{"type":"string"}}
        }));
        assert!(secret_config.validate().is_err());
    }

    #[test]
    fn artifact_identity_is_content_addressed_without_artifact_id_or_release_id() {
        let manifest = manifest();
        let files = vec![
            PluginArtifactFile { normalized_relative_path: PLUGIN_MANIFEST_PATH.into(), digest: DigestHex::from("1".repeat(64)), size_bytes: 10 },
            PluginArtifactFile { normalized_relative_path: "service/main.mjs".into(), digest: DigestHex::from("2".repeat(64)), size_bytes: 10 },
            PluginArtifactFile { normalized_relative_path: "ui/index.html".into(), digest: DigestHex::from("3".repeat(64)), size_bytes: 10 },
        ];
        let artifact = PluginArtifact::new(manifest, files).unwrap();
        artifact.validate().unwrap();
        let json = serde_json::to_value(&artifact).unwrap();
        assert!(json.get("artifact_id").is_none());
        assert!(json.get("release_id").is_none());
    }
}
