use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    canonical_ui_tree_digest, digest_bytes, digest_payload, ArtifactId,
    CanonicalDigestError, CanonicalSchemaRef, CredentialSlotDeclaration, DigestHex,
    JavaScriptBuildProfile, LocalizedMetadata, MiniAppM1ContractError,
    MiniAppReleaseArtifactV1, MiniAppReleaseFile, MiniAppReleaseV1Manifest,
    MiniAppResourceContract, MiniAppServiceLifecycle,
    MiniAppServiceReleaseDescriptor, MiniAppUiReleaseDescriptor, PackageContributions,
    PackageRef, StrictJsonValue, VersionString, MINIAPP_M1_SCHEMA_VERSION,
    MINIAPP_RELEASE_PROFILE_VERSION, MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
    MINIAPP_SERVICE_SDK_CONTRACT_VERSION,
};
use serde_json::Value;
use thiserror::Error;

pub const MINIAPP_SURFACE_BRIDGE_BOOTSTRAP_MARKER: &str =
    "nomifun-miniapp-bridge-bootstrap-v1";
pub const MINIAPP_SERVICE_ENTRYPOINT: &str = "service/main.mjs";

const MINIAPP_SURFACE_BRIDGE_BOOTSTRAP: &str =
    r#"<script data-nomifun-miniapp-bridge="nomifun-miniapp-bridge-bootstrap-v1">
(() => {
  const VERSION = '1.0.0';
  const CHALLENGE = 'nomifun-miniapp-bridge-challenge-v1';
  const HANDSHAKE = 'nomifun-miniapp-bridge-handshake-v1';
  const CONNECT = 'nomifun-miniapp-bridge-connect-v1';
  let pendingNonce = null;
  window.addEventListener('message', (event) => {
    if (event.source !== window.parent) return;
    const data = event.data;
    if (!data || data.version !== VERSION) return;
    if (
      data.type === CHALLENGE &&
      typeof data.nonce === 'string' &&
      /^[a-f0-9]{64}$/.test(data.nonce)
    ) {
      pendingNonce = data.nonce;
      window.parent.postMessage(
        { type: HANDSHAKE, version: VERSION, nonce: pendingNonce },
        event.origin
      );
      return;
    }
    if (
      data.type !== CONNECT ||
      data.nonce !== pendingNonce ||
      !event.ports ||
      event.ports.length !== 1
    ) return;
    const port = event.ports[0];
    pendingNonce = null;
    if (!Object.prototype.hasOwnProperty.call(window, '__nomifunMiniAppBridge')) {
      Object.defineProperty(window, '__nomifunMiniAppBridge', {
        value: port,
        configurable: false,
        enumerable: false,
        writable: false,
      });
    }
    port.start();
    window.dispatchEvent(new Event('nomifun-miniapp-bridge-ready'));
  });
})();
</script>
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppStaticBundleFile {
    pub normalized_relative_path: String,
    pub bytes: Vec<u8>,
}

impl MiniAppStaticBundleFile {
    pub fn new(
        normalized_relative_path: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            normalized_relative_path: normalized_relative_path.into(),
            bytes: bytes.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppStaticServiceInput {
    pub main_mjs: Vec<u8>,
    pub lifecycle: MiniAppServiceLifecycle,
    pub uses_files: bool,
    pub uses_private_database: bool,
    pub service_contract_digest: DigestHex,
    pub runtime_requirements_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppStaticServiceMaterialization {
    pub descriptor: MiniAppServiceReleaseDescriptor,
    pub file: MiniAppStaticBundleFile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniAppStaticBundleInput {
    pub artifact_id: ArtifactId,
    pub display: LocalizedMetadata,
    pub ui_index_html: Vec<u8>,
    pub ui_assets: Vec<MiniAppStaticBundleFile>,
    pub service: Option<MiniAppStaticServiceInput>,
    pub package_json: Option<Vec<u8>>,
    pub dependency_lock_digest: DigestHex,
    pub dependency_graph_digest: DigestHex,
    pub config_schema: StrictJsonValue,
    pub credential_slots: Vec<CredentialSlotDeclaration>,
    pub resource_contract: MiniAppResourceContract,
    pub schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
    pub bridge_contract_digest: DigestHex,
    pub contribution_package: PackageRef,
    pub contributions: PackageContributions,
    pub migrations: Vec<nomifun_agent_contracts::MiniAppMigration>,
}

#[derive(Debug, Error)]
pub enum MiniAppStaticBundleBuildError {
    #[error("invalid static bundle path {path}: {reason}")]
    InvalidPath { path: String, reason: String },
    #[error("static bundle file is empty: {path}")]
    EmptyFile { path: String },
    #[error("static bundle path collides under Windows semantics: {path}")]
    PathCollision { path: String },
    #[error("static bundle file size cannot be represented: {path}")]
    FileSizeOverflow { path: String },
    #[error("package.json is invalid: {0}")]
    InvalidPackageJson(String),
    #[error("custom JavaScript build/install scripts are forbidden")]
    CustomScriptsForbidden,
    #[error("UI-only MiniApp cannot include Service source")]
    UiOnlyServiceSource,
    #[error("MiniApp Service source must be valid UTF-8")]
    InvalidServiceSourceEncoding,
    #[error("MiniApp Service source contains a NUL byte")]
    InvalidServiceSourceContent,
    #[error("UI entrypoint must be valid UTF-8 for the Host Bridge bootstrap")]
    InvalidUiEntrypointEncoding,
    #[error(transparent)]
    CanonicalDigest(#[from] CanonicalDigestError),
    #[error(transparent)]
    Contract(#[from] MiniAppM1ContractError),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MiniAppStaticBundleBuilder;

impl MiniAppStaticBundleBuilder {
    pub const fn new() -> Self {
        Self
    }

    pub fn build(
        &self,
        input: MiniAppStaticBundleInput,
    ) -> Result<MiniAppReleaseArtifactV1, MiniAppStaticBundleBuildError> {
        build_miniapp_static_bundle(input)
    }

    pub fn build_ui_only(
        &self,
        input: MiniAppStaticBundleInput,
    ) -> Result<MiniAppReleaseArtifactV1, MiniAppStaticBundleBuildError> {
        if input.service.is_some() {
            return Err(MiniAppStaticBundleBuildError::UiOnlyServiceSource);
        }
        build_miniapp_static_bundle(input)
    }

    pub const fn ui_only_requires_node() -> bool {
        false
    }

    pub const fn build_profile() -> JavaScriptBuildProfile {
        JavaScriptBuildProfile::MiniAppReleaseV1
    }
}

pub fn build_miniapp_static_bundle(
    input: MiniAppStaticBundleInput,
) -> Result<MiniAppReleaseArtifactV1, MiniAppStaticBundleBuildError> {
    validate_no_custom_scripts(input.package_json.as_deref())?;

    let MiniAppStaticBundleInput {
        artifact_id,
        display,
        ui_index_html,
        ui_assets,
        service,
        package_json: _,
        dependency_lock_digest,
        dependency_graph_digest,
        config_schema,
        mut credential_slots,
        resource_contract,
        schemas,
        bridge_contract_digest,
        contribution_package,
        contributions,
        migrations,
    } = input;

    let mut collision_keys = BTreeSet::new();
    let mut files = Vec::with_capacity(
        1 + ui_assets.len() + usize::from(service.is_some()),
    );
    let ui_index_html = materialize_surface_entrypoint(&ui_index_html)?;
    push_file(
        &mut files,
        &mut collision_keys,
        "ui/index.html".to_owned(),
        ui_index_html,
    )?;

    for asset in ui_assets {
        validate_ui_asset_path(&asset.normalized_relative_path)?;
        push_file(
            &mut files,
            &mut collision_keys,
            asset.normalized_relative_path,
            asset.bytes,
        )?;
    }

    let service = service
        .map(materialize_service_release)
        .transpose()?;
    let service_descriptor = service
        .as_ref()
        .map(|materialized| materialized.descriptor.clone());

    if let Some(service) = service {
        push_file(
            &mut files,
            &mut collision_keys,
            service.file.normalized_relative_path,
            service.file.bytes,
        )?;
    }

    credential_slots.sort_by(|left, right| {
        left.slot_key.as_ref().cmp(right.slot_key.as_ref())
    });

    let entrypoint_digest = files
        .iter()
        .find(|file| file.normalized_relative_path == "ui/index.html")
        .map(|file| file.digest.clone())
        .ok_or_else(|| MiniAppStaticBundleBuildError::EmptyFile {
            path: "ui/index.html".to_owned(),
        })?;
    let ui_tree_digest = canonical_ui_tree_digest(&files)?;
    let config_schema_digest = digest_payload(&config_schema.0)?;
    let credential_slots_digest = digest_payload(&credential_slots)?;
    let resource_contract_digest = digest_payload(&resource_contract)?;

    let manifest = MiniAppReleaseV1Manifest {
        schema_version: VersionString::from(MINIAPP_M1_SCHEMA_VERSION),
        build_profile: JavaScriptBuildProfile::MiniAppReleaseV1,
        build_profile_version: VersionString::from(
            MINIAPP_RELEASE_PROFILE_VERSION,
        ),
        display,
        ui: MiniAppUiReleaseDescriptor {
            entrypoint: "ui/index.html".to_owned(),
            entrypoint_digest,
            ui_tree_digest,
        },
        service: service_descriptor,
        dependency_lock_digest,
        dependency_graph_digest,
        config_schema,
        config_schema_digest,
        credential_slots,
        credential_slots_digest,
        resource_contract,
        resource_contract_digest,
        schemas,
        bridge_contract_digest,
        contribution_package,
        contributions,
        migrations,
    };

    Ok(MiniAppReleaseArtifactV1::new(
        artifact_id,
        manifest,
        files,
    )?)
}

/// Add the Host-owned Surface Bridge bootstrap to every immutable Release
/// entrypoint. Source Store bytes stay untouched; the deterministic Build
/// transform ensures custom authored HTML has the same nonce handshake as the
/// default scaffold.
pub fn materialize_surface_entrypoint(
    source: &[u8],
) -> Result<Vec<u8>, MiniAppStaticBundleBuildError> {
    if source.is_empty() {
        return Ok(Vec::new());
    }
    let source = std::str::from_utf8(source)
        .map_err(|_| MiniAppStaticBundleBuildError::InvalidUiEntrypointEncoding)?;
    if source.contains(MINIAPP_SURFACE_BRIDGE_BOOTSTRAP_MARKER) {
        return Ok(source.as_bytes().to_vec());
    }
    let mut materialized =
        Vec::with_capacity(MINIAPP_SURFACE_BRIDGE_BOOTSTRAP.len() + source.len());
    materialized.extend_from_slice(MINIAPP_SURFACE_BRIDGE_BOOTSTRAP.as_bytes());
    materialized.extend_from_slice(b"<script data-nomifun-product-sdk=\"1\">");
    materialized.extend_from_slice(include_bytes!("../assets/product-sdk.js"));
    materialized.extend_from_slice(b"</script>");
    materialized.extend_from_slice(source.as_bytes());
    Ok(materialized)
}

/// Validate Service source framing without executing or statically interpreting
/// JavaScript. The Service Host validates the required `start(context)` export
/// during its startup handshake.
pub fn validate_service_source(
    source: &[u8],
) -> Result<(), MiniAppStaticBundleBuildError> {
    if source.is_empty() {
        return Err(MiniAppStaticBundleBuildError::EmptyFile {
            path: MINIAPP_SERVICE_ENTRYPOINT.to_owned(),
        });
    }
    std::str::from_utf8(source)
        .map_err(|_| MiniAppStaticBundleBuildError::InvalidServiceSourceEncoding)?;
    if source.contains(&0) {
        return Err(MiniAppStaticBundleBuildError::InvalidServiceSourceContent);
    }
    Ok(())
}

/// Materialize the one Service file and its manifest descriptor from the same
/// input bytes so the Release cannot carry a digest/file mismatch.
pub fn materialize_service_release(
    input: MiniAppStaticServiceInput,
) -> Result<MiniAppStaticServiceMaterialization, MiniAppStaticBundleBuildError> {
    validate_service_source(&input.main_mjs)?;
    let MiniAppStaticServiceInput {
        main_mjs,
        lifecycle,
        uses_files,
        uses_private_database,
        service_contract_digest,
        runtime_requirements_digest,
    } = input;
    let module_digest = digest_bytes(&main_mjs);
    Ok(MiniAppStaticServiceMaterialization {
        descriptor: MiniAppServiceReleaseDescriptor {
            entrypoint: MINIAPP_SERVICE_ENTRYPOINT.to_owned(),
            module_digest,
            lifecycle,
            uses_files,
            uses_private_database,
            service_contract_digest,
            host_protocol_version: VersionString::from(
                MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
            ),
            sdk_contract_version: VersionString::from(
                MINIAPP_SERVICE_SDK_CONTRACT_VERSION,
            ),
            runtime_requirements_digest,
        },
        file: MiniAppStaticBundleFile::new(
            MINIAPP_SERVICE_ENTRYPOINT,
            main_mjs,
        ),
    })
}

pub fn validate_static_bundle_path(
    path: &str,
) -> Result<(), MiniAppStaticBundleBuildError> {
    validate_normalized_relative_path(path)?;
    if path == "ui/index.html"
        || path == "service/main.mjs"
        || path.starts_with("ui/")
    {
        Ok(())
    } else {
        Err(MiniAppStaticBundleBuildError::InvalidPath {
            path: path.to_owned(),
            reason: "miniapp-release-v1 permits ui/** and service/main.mjs only"
                .to_owned(),
        })
    }
}

pub fn validate_no_custom_scripts(
    package_json: Option<&[u8]>,
) -> Result<(), MiniAppStaticBundleBuildError> {
    let Some(package_json) = package_json else {
        return Ok(());
    };
    let value: Value = serde_json::from_slice(package_json)
        .map_err(|error| MiniAppStaticBundleBuildError::InvalidPackageJson(
            error.to_string(),
        ))?;
    let object = value.as_object().ok_or_else(|| {
        MiniAppStaticBundleBuildError::InvalidPackageJson(
            "package.json must contain a JSON object".to_owned(),
        )
    })?;
    let Some(scripts) = object.get("scripts") else {
        return Ok(());
    };
    match scripts {
        Value::Object(entries) if entries.is_empty() => Ok(()),
        Value::Object(_) => Err(
            MiniAppStaticBundleBuildError::CustomScriptsForbidden,
        ),
        _ => Err(MiniAppStaticBundleBuildError::InvalidPackageJson(
            "package.json scripts must be an object".to_owned(),
        )),
    }
}

fn validate_ui_asset_path(
    path: &str,
) -> Result<(), MiniAppStaticBundleBuildError> {
    validate_static_bundle_path(path)?;
    if path == "ui/index.html" {
        return Err(MiniAppStaticBundleBuildError::InvalidPath {
            path: path.to_owned(),
            reason: "ui/index.html is the reserved UI entrypoint".to_owned(),
        });
    }
    Ok(())
}

fn validate_normalized_relative_path(
    path: &str,
) -> Result<(), MiniAppStaticBundleBuildError> {
    if path.is_empty()
        || path.trim() != path
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains(':')
        || path.split('/').any(|segment| {
            segment.is_empty() || segment == "." || segment == ".."
        })
    {
        return Err(MiniAppStaticBundleBuildError::InvalidPath {
            path: path.to_owned(),
            reason: "path must be normalized, relative, slash-separated, and traversal-free"
                .to_owned(),
        });
    }
    for component in path.split('/') {
        if component.ends_with('.')
            || component.ends_with(' ')
            || is_windows_reserved_name(component)
        {
            return Err(MiniAppStaticBundleBuildError::InvalidPath {
                path: path.to_owned(),
                reason: "path is not stable under Windows filename semantics"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

fn push_file(
    files: &mut Vec<MiniAppReleaseFile>,
    collision_keys: &mut BTreeSet<String>,
    path: String,
    bytes: Vec<u8>,
) -> Result<(), MiniAppStaticBundleBuildError> {
    validate_static_bundle_path(&path)?;
    let collision_key = windows_collision_key(&path);
    if !collision_keys.insert(collision_key) {
        return Err(MiniAppStaticBundleBuildError::PathCollision { path });
    }
    if bytes.is_empty() {
        return Err(MiniAppStaticBundleBuildError::EmptyFile { path });
    }
    let size_bytes = u64::try_from(bytes.len()).map_err(|_| {
        MiniAppStaticBundleBuildError::FileSizeOverflow {
            path: path.clone(),
        }
    })?;
    files.push(MiniAppReleaseFile {
        normalized_relative_path: path,
        digest: digest_bytes(&bytes),
        size_bytes,
    });
    Ok(())
}

fn windows_collision_key(path: &str) -> String {
    path.split('/')
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("/")
}

fn is_windows_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}
