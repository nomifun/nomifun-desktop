use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    CanonicalSchemaRef, CredentialSlotDeclaration, JavaScriptBuildProfile,
    LocalizedMetadata, PackageContributions, PackageId, RuntimeFeatureRef,
    StrictJsonValue, VersionString, validate_plugin_schema_registry,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::canonical::{canonical_json_bytes, strict_json_from_slice};
use crate::error::AuthoringError;
use crate::path::NormalizedSourcePath;

pub const PLUGIN_SOURCE_MANIFEST_VERSION: &str = "1.0.0";
pub const PLUGIN_SOURCE_MANIFEST_FILE: &str = "nomifun.plugin.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PluginLanguage {
    #[serde(rename = "javascript")]
    JavaScript,
    #[serde(rename = "typescript")]
    TypeScript,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSourceManifest {
    schema_version: String,
    build_profile: JavaScriptBuildProfile,
    package_id: PackageId,
    package_version: VersionString,
    display: LocalizedMetadata,
    language: PluginLanguage,
    entrypoint: NormalizedSourcePath,
    requires_runtime_features: Vec<RuntimeFeatureRef>,
    config_schema: StrictJsonValue,
    contributions: PackageContributions,
    schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
    credential_slots: Vec<CredentialSlotDeclaration>,
}

impl PluginSourceManifest {
    pub fn new(
        package_id: impl Into<PackageId>,
        package_version: impl Into<VersionString>,
        display: LocalizedMetadata,
        language: PluginLanguage,
        entrypoint: NormalizedSourcePath,
    ) -> Result<Self, AuthoringError> {
        let manifest = Self {
            schema_version: PLUGIN_SOURCE_MANIFEST_VERSION.into(),
            build_profile: JavaScriptBuildProfile::PluginPackageV1,
            package_id: package_id.into(),
            package_version: package_version.into(),
            display,
            language,
            entrypoint,
            requires_runtime_features: Vec::new(),
            config_schema: StrictJsonValue(json!({
                "additionalProperties": false,
                "type": "object"
            })),
            contributions: PackageContributions::default(),
            schemas: BTreeMap::new(),
            credential_slots: Vec::new(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, AuthoringError> {
        let manifest: Self = strict_json_from_slice(bytes)
            .map_err(|error| AuthoringError::InvalidSourceManifest(error.to_string()))?;
        manifest.validate()?;
        if manifest.canonical_bytes()? != bytes {
            return Err(AuthoringError::InvalidSourceManifest(
                "source manifest must use canonical JSON".into(),
            ));
        }
        Ok(manifest)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, AuthoringError> {
        self.validate()?;
        canonical_json_bytes(self)
    }

    pub fn validate(&self) -> Result<(), AuthoringError> {
        if self.schema_version != PLUGIN_SOURCE_MANIFEST_VERSION {
            return Err(invalid(format!(
                "schema_version must be {PLUGIN_SOURCE_MANIFEST_VERSION}"
            )));
        }
        if self.build_profile != JavaScriptBuildProfile::PluginPackageV1 {
            return Err(invalid("build_profile must be plugin_package_v1"));
        }
        validate_package_id(self.package_id.as_ref())?;
        let version = Version::parse(self.package_version.as_ref())
            .map_err(|error| invalid(format!("package_version must be exact SemVer: {error}")))?;
        if version.to_string() != self.package_version.as_ref() {
            return Err(invalid("package_version must be canonical exact SemVer"));
        }
        validate_display_text("display.name", &self.display.name, 255)?;
        validate_display_text("display.description", &self.display.description, 4_096)?;
        validate_localized_map("display.localized_names", &self.display.localized_names)?;
        validate_localized_map(
            "display.localized_descriptions",
            &self.display.localized_descriptions,
        )?;
        let expected_extension = match self.language {
            PluginLanguage::JavaScript => [".js", ".mjs"],
            PluginLanguage::TypeScript => [".ts", ".mts"],
        };
        if !expected_extension
            .iter()
            .any(|extension| self.entrypoint.as_str().ends_with(extension))
        {
            return Err(invalid(format!(
                "entrypoint extension does not match {:?}",
                self.language
            )));
        }
        if !self.config_schema.0.is_object() {
            return Err(invalid("config_schema must be a JSON object"));
        }
        if !self.contributions.role_contracts.is_empty()
            || !self.contributions.role_providers.is_empty()
        {
            return Err(invalid(
                "plugin-package-v1 does not allow role contracts or role providers",
            ));
        }
        validate_plugin_schema_registry(&self.contributions, &self.schemas)
            .map_err(|error| invalid(error.to_string()))?;
        let package_ref = (
            self.package_id.as_ref(),
            self.package_version.as_ref(),
        );
        let mut capability_ids = BTreeSet::new();
        for capability in &self.contributions.capabilities {
            if (
                capability.package.id.as_ref(),
                capability.package.version.as_ref(),
            ) != package_ref
            {
                return Err(invalid(format!(
                    "capability {} must reference the source package and version",
                    capability.id.as_ref()
                )));
            }
            if !capability_ids.insert(capability.id.as_ref()) {
                return Err(invalid(format!(
                    "duplicate capability {}",
                    capability.id.as_ref()
                )));
            }
        }
        let mut slots = BTreeSet::new();
        for slot in &self.credential_slots {
            if slot.slot_key.as_ref().is_empty()
                || !slot.slot_key.as_ref().bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-')
                })
            {
                return Err(invalid(format!(
                    "credential slot {} is not a stable machine key",
                    slot.slot_key.as_ref()
                )));
            }
            validate_display_text(
                "credential_slots.display_name",
                &slot.display_name,
                255,
            )?;
            if !slots.insert(slot.slot_key.as_ref()) {
                return Err(invalid(format!(
                    "duplicate credential slot {}",
                    slot.slot_key.as_ref()
                )));
            }
        }
        Ok(())
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub fn build_profile(&self) -> JavaScriptBuildProfile {
        self.build_profile
    }

    pub fn package_id(&self) -> &PackageId {
        &self.package_id
    }

    pub fn package_version(&self) -> &VersionString {
        &self.package_version
    }

    pub fn display(&self) -> &LocalizedMetadata {
        &self.display
    }

    pub fn language(&self) -> PluginLanguage {
        self.language
    }

    pub fn entrypoint(&self) -> &NormalizedSourcePath {
        &self.entrypoint
    }

    pub fn requires_runtime_features(&self) -> &[RuntimeFeatureRef] {
        &self.requires_runtime_features
    }

    pub fn config_schema(&self) -> &StrictJsonValue {
        &self.config_schema
    }

    pub fn contributions(&self) -> &PackageContributions {
        &self.contributions
    }

    pub fn schemas(&self) -> &BTreeMap<CanonicalSchemaRef, StrictJsonValue> {
        &self.schemas
    }

    pub fn credential_slots(&self) -> &[CredentialSlotDeclaration] {
        &self.credential_slots
    }

    pub fn with_contributions(
        mut self,
        contributions: PackageContributions,
    ) -> Result<Self, AuthoringError> {
        self.contributions = contributions;
        self.validate()?;
        Ok(self)
    }

    pub fn with_contributions_and_schemas(
        mut self,
        contributions: PackageContributions,
        schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
    ) -> Result<Self, AuthoringError> {
        self.contributions = contributions;
        self.schemas = schemas;
        self.validate()?;
        Ok(self)
    }

    pub fn with_config_schema(
        mut self,
        config_schema: StrictJsonValue,
    ) -> Result<Self, AuthoringError> {
        self.config_schema = config_schema;
        self.validate()?;
        Ok(self)
    }

    pub fn with_runtime_features(
        mut self,
        requires_runtime_features: Vec<RuntimeFeatureRef>,
    ) -> Result<Self, AuthoringError> {
        self.requires_runtime_features = requires_runtime_features;
        self.validate()?;
        Ok(self)
    }

    pub fn with_credential_slots(
        mut self,
        credential_slots: Vec<CredentialSlotDeclaration>,
    ) -> Result<Self, AuthoringError> {
        self.credential_slots = credential_slots;
        self.validate()?;
        Ok(self)
    }
}

fn validate_package_id(value: &str) -> Result<(), AuthoringError> {
    if value.is_empty()
        || value.len() > 255
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Err(invalid(
            "package_id must use 1-255 ASCII letters, digits, dot, underscore, or hyphen",
        ))
    } else {
        Ok(())
    }
}

fn validate_display_text(
    field: &str,
    value: &str,
    max_len: usize,
) -> Result<(), AuthoringError> {
    if value.trim().is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        Err(invalid(format!(
            "{field} must be non-empty, at most {max_len} bytes, and contain no control characters"
        )))
    } else {
        Ok(())
    }
}

fn validate_localized_map(
    field: &str,
    values: &std::collections::BTreeMap<String, String>,
) -> Result<(), AuthoringError> {
    for (locale, value) in values {
        if locale.is_empty()
            || locale.len() > 64
            || !locale
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(invalid(format!("{field} contains invalid locale {locale:?}")));
        }
        validate_display_text(field, value, 4_096)?;
    }
    Ok(())
}

fn invalid(reason: impl Into<String>) -> AuthoringError {
    AuthoringError::InvalidSourceManifest(reason.into())
}
