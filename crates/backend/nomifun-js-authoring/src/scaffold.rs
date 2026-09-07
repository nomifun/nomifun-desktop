use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::canonical::canonical_json_bytes;
use crate::error::AuthoringError;
use crate::path::NormalizedSourcePath;

pub const PLUGIN_SOURCE_MANIFEST_VERSION: &str = "1.0.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginLanguage {
    JavaScript,
    TypeScript,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginScaffoldRequest {
    pub package_id: String,
    pub package_version: String,
    pub display_name: String,
    pub description: String,
    pub language: PluginLanguage,
}

impl PluginScaffoldRequest {
    pub(crate) fn validate(&self) -> Result<(), AuthoringError> {
        if self.package_id.is_empty()
            || self.package_id.len() > 255
            || !self
                .package_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(AuthoringError::InvalidField {
                field: "package_id",
                reason: "must use 1-255 ASCII letters, digits, dot, underscore, or hyphen".into(),
            });
        }
        let version = Version::parse(&self.package_version).map_err(|error| {
            AuthoringError::InvalidField {
                field: "package_version",
                reason: format!("must be exact SemVer: {error}"),
            }
        })?;
        if version.to_string() != self.package_version {
            return Err(AuthoringError::InvalidField {
                field: "package_version",
                reason: "must be canonical exact SemVer".into(),
            });
        }
        validate_display_text("display_name", &self.display_name, 255)?;
        validate_display_text("description", &self.description, 4_096)
    }
}

pub(crate) fn render_plugin_scaffold(
    request: &PluginScaffoldRequest,
) -> Result<Vec<(NormalizedSourcePath, Vec<u8>)>, AuthoringError> {
    request.validate()?;
    let entrypoint = match request.language {
        PluginLanguage::JavaScript => "src/main.js",
        PluginLanguage::TypeScript => "src/main.ts",
    };
    let manifest = PluginSourceManifest {
        schema_version: PLUGIN_SOURCE_MANIFEST_VERSION,
        build_profile: "plugin-package-v1",
        package_id: &request.package_id,
        package_version: &request.package_version,
        display: PluginSourceDisplay {
            name: &request.display_name,
            description: &request.description,
        },
        entrypoint,
        capabilities: Vec::new(),
    };
    let package_json = FixedPackageJson {
        private: true,
        module_type: "module",
        dependencies: BTreeMap::new(),
    };
    let source = match request.language {
        PluginLanguage::JavaScript => JAVASCRIPT_ENTRYPOINT,
        PluginLanguage::TypeScript => TYPESCRIPT_ENTRYPOINT,
    };

    Ok(vec![
        (
            NormalizedSourcePath::parse("nomifun.plugin.json")?,
            canonical_json_bytes(&manifest)?,
        ),
        (
            NormalizedSourcePath::parse("package.json")?,
            canonical_json_bytes(&package_json)?,
        ),
        (
            NormalizedSourcePath::parse(entrypoint)?,
            source.as_bytes().to_vec(),
        ),
    ])
}

#[derive(Serialize)]
struct PluginSourceManifest<'a> {
    schema_version: &'static str,
    build_profile: &'static str,
    package_id: &'a str,
    package_version: &'a str,
    display: PluginSourceDisplay<'a>,
    entrypoint: &'a str,
    capabilities: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct PluginSourceDisplay<'a> {
    name: &'a str,
    description: &'a str,
}

#[derive(Serialize)]
struct FixedPackageJson {
    private: bool,
    #[serde(rename = "type")]
    module_type: &'static str,
    dependencies: BTreeMap<String, String>,
}

const JAVASCRIPT_ENTRYPOINT: &str = r#"export async function activate({ mount, sdk }) {
  void mount;
  void sdk;
  return { capabilities: {} };
}
"#;

const TYPESCRIPT_ENTRYPOINT: &str = r#"type ActivationContext = Readonly<{
  mount: unknown;
  sdk: unknown;
}>;

export async function activate({ mount, sdk }: ActivationContext) {
  void mount;
  void sdk;
  return { capabilities: {} };
}
"#;

fn validate_display_text(
    field: &'static str,
    value: &str,
    max_len: usize,
) -> Result<(), AuthoringError> {
    if value.trim().is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        Err(AuthoringError::InvalidField {
            field,
            reason: format!(
                "must be non-empty, at most {max_len} bytes, and contain no control characters"
            ),
        })
    } else {
        Ok(())
    }
}
