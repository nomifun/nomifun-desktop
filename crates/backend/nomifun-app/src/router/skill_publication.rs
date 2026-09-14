//! Skill Library -> ordinary immutable Plugin candidate. No engine mounting,
//! implicit activation, mutable directory consumption or script execution.
use axum::{Extension, Json, Router, extract::State, routing::post};
use nomifun_agent_contracts::*;
use nomifun_api_types::{ApiResponse, ImportPluginRequest, PluginImportKindDto};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_plugin_platform::application::PluginApplicationService;
use nomifun_skill_library::{
    SkillPaths,
    frozen::{self, FrozenLibrarySkill, LibrarySkillSelection},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const ENTRYPOINT: &[u8] = b"// Frozen Skill resources only; never execute bundled scripts.\nexport async function activate() { return { capabilities: {} }; }\n";
static CAPTURE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
// Serialize this product publication lane, including target discovery and CAS.
// This is not a replacement for repository transactions or cross-process locks.
static PUBLICATION_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Clone)]
struct PublicationState {
    paths: SkillPaths,
    plugins: Arc<PluginApplicationService>,
}

pub(super) fn routes(paths: SkillPaths, plugins: Arc<PluginApplicationService>) -> Router {
    Router::new()
        .route("/api/skills/frozen/preview", post(preview))
        .route("/api/skills/frozen/publish", post(publish))
        .with_state(PublicationState { paths, plugins })
}

#[derive(Serialize)]
struct FrozenFile {
    path: String,
    bytes: usize,
    sha256: String,
}
#[derive(Serialize)]
struct Preview {
    selection: LibrarySkillSelection,
    source_digest: String,
    artifact_digest: String,
    package_id: String,
    package_version: String,
    skill_id: String,
    files: Vec<FrozenFile>,
    expected_library_revision: u64,
    projects: Vec<ProjectChoice>,
}
#[derive(Serialize)]
struct ProjectChoice {
    project_id: String,
    project_revision: u64,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishRequest {
    selection: LibrarySkillSelection,
    expected_source_digest: String,
    expected_artifact_digest: String,
    expected_library_revision: u64,
    target_project_id: Option<String>,
    expected_project_revision: Option<u64>,
}
#[derive(Serialize)]
struct Published {
    project_id: String,
}

struct Prepared {
    preview: Preview,
    files: BTreeMap<String, Vec<u8>>,
    _capture_permit: Option<tokio::sync::SemaphorePermit<'static>>,
}
fn fail(error: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!("Skill publication: {error}"))
}
fn sha(bytes: &[u8]) -> DigestHex {
    format!("{:x}", Sha256::digest(bytes)).into()
}

async fn prepare(
    state: &PublicationState,
    user: &str,
    selection: LibrarySkillSelection,
) -> Result<Prepared, AppError> {
    let permit = CAPTURE_SLOTS.try_acquire().map_err(|_| {
        AppError::Conflict(
            "Skill capture is busy; try again after the current previews/publications finish"
                .into(),
        )
    })?;
    let paths = state.paths.clone();
    let user = user.to_owned();
    // The task retains its permit even if an HTTP caller disconnects.
    tokio::task::spawn_blocking(move || {
        let mut prepared = build(&user, frozen::capture(&paths, selection)?)?;
        prepared._capture_permit = Some(permit);
        Ok(prepared)
    })
    .await
    .map_err(fail)?
}

async fn preview(
    State(state): State<PublicationState>,
    Extension(user): Extension<CurrentUser>,
    Json(selection): Json<LibrarySkillSelection>,
) -> Result<Json<ApiResponse<Preview>>, AppError> {
    let mut prepared = prepare(&state, user.id.as_str(), selection).await?;
    let (revision, projects) = state
        .plugins
        .captured_import_targets(user.id.as_str(), &prepared.preview.package_id)
        .await
        .map_err(fail)?;
    prepared.preview.expected_library_revision = revision;
    prepared.preview.projects = projects
        .into_iter()
        .map(|project| ProjectChoice {
            project_id: project.project_id,
            project_revision: project.project_revision,
            display_name: project.display_name,
        })
        .collect();
    Ok(Json(ApiResponse::ok(prepared.preview)))
}

async fn publish(
    State(state): State<PublicationState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<PublishRequest>,
) -> Result<Json<ApiResponse<Published>>, AppError> {
    let _publication = PUBLICATION_SLOT.try_acquire().map_err(|_| {
        AppError::Conflict(
            "Another Skill publication is in progress; inspect its outcome before retrying".into(),
        )
    })?;
    if request.target_project_id.is_some() != request.expected_project_revision.is_some() {
        return Err(fail(
            "target Project and expected revision must be supplied together",
        ));
    }
    let prepared = prepare(&state, user.id.as_str(), request.selection).await?;
    if prepared.preview.source_digest != request.expected_source_digest
        || prepared.preview.artifact_digest != request.expected_artifact_digest
    {
        return Err(AppError::Conflict(
            "Skill changed since preview; review the new bytes before publishing".into(),
        ));
    }
    let (_, existing) = state
        .plugins
        .captured_import_targets(user.id.as_str(), &prepared.preview.package_id)
        .await
        .map_err(fail)?;
    if request.target_project_id.is_none() && !existing.is_empty() {
        return Err(AppError::Conflict("This Skill already has a Project; preview again and explicitly select its update target".into()));
    }
    let project = state
        .plugins
        .import_captured(
            user.id.as_str(),
            ImportPluginRequest {
                expected_library_revision: request.expected_library_revision,
                import_kind: PluginImportKindDto::PrebuiltArtifact,
                source_path: "[captured Skill Library bytes]".into(),
                expected_bundle_or_artifact_digest: request.expected_artifact_digest,
                target_project_id: request.target_project_id,
                expected_project_revision: request.expected_project_revision,
            },
            prepared.files,
        )
        .await
        .map_err(fail)?;
    Ok(Json(ApiResponse::ok(Published {
        project_id: project.summary.project_id,
    })))
}

fn build(owner: &str, captured: FrozenLibrarySkill) -> Result<Prepared, AppError> {
    let identity = serde_json::to_vec(&(owner, &captured.selection)).map_err(fail)?;
    let package_id = format!("library.skill.{}", sha(&identity).as_ref());
    let package_version = format!("0.0.0+{}", captured.source_digest);
    let skill_id = format!("{package_id}.skill");
    let package = PackageRef {
        id: package_id.clone().into(),
        version: package_version.clone().into(),
    };
    let display = LocalizedMetadata {
        name: captured.selection.name.clone(),
        description: format!(
            "Frozen library Skill: {}. Explicit publication; no automatic capabilities or script execution.",
            captured.selection.name
        ),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    };
    let mut files = BTreeMap::from([("main.mjs".to_owned(), ENTRYPOINT.to_vec())]);
    let mut inventory = Vec::new();
    let mut resources = Vec::new();
    let mut body_ref = None;
    let mut encoded_images = 0usize;
    for (path, bytes) in captured.files {
        if matches!(
            std::path::Path::new(&path)
                .extension()
                .and_then(|v| v.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("png" | "jpg" | "jpeg" | "webp")
        ) {
            let image = nomifun_ai_agent::model_attachments::prepare_image_resource(&bytes, &path)
                .map_err(fail)?;
            if let nomifun_chat_model_broker::ChatToolResultPart::Image { data_base64, .. } = image
            {
                encoded_images += data_base64.len();
                if encoded_images > 8 * 1024 * 1024 || data_base64.len() > 2 * 1024 * 1024 {
                    return Err(fail("prepared images exceed the Skill resource envelope"));
                }
            }
        }
        let digest = sha(&bytes);
        inventory.push(FrozenFile {
            path: path.clone(),
            bytes: bytes.len(),
            sha256: digest.as_ref().into(),
        });
        let resource_identity = serde_json::to_vec(&(&path, &digest)).map_err(fail)?;
        let artifact = LogicalArtifactRef {
            artifact_id: format!("skill-resource:{}", sha(&resource_identity).as_ref()).into(),
            normalized_relative_path: format!("resources/skill/{path}"),
            digest,
        };
        files.insert(artifact.normalized_relative_path.clone(), bytes);
        if path == "SKILL.md" {
            body_ref = Some(artifact);
        } else {
            let kind = if path.starts_with("scripts/") {
                SkillResourceKind::Script
            } else if path.starts_with("templates/") {
                SkillResourceKind::Template
            } else if path.starts_with("examples/") {
                SkillResourceKind::Example
            } else {
                SkillResourceKind::Reference
            };
            resources.push(SkillResourceRef { kind, artifact });
        }
    }
    let skill = SkillDefinition {
        id: skill_id.clone().into(),
        version: package.version.clone(),
        package: package.clone(),
        display: display.clone(),
        body_ref: body_ref.ok_or_else(|| fail("missing Skill body"))?,
        resources,
        requires_capabilities: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop", "headless"],
            [CapabilityConsumer::Agent],
        ),
    };
    let manifest = PluginPackageV1Manifest {
        schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
        build_profile: JavaScriptBuildProfile::PluginPackageV1,
        build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
        package: PackageManifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            package_id: package.id,
            package_version: package.version,
            display,
            package_dependencies: Vec::new(),
            requires_runtime_features: Vec::new(),
            config_schema: StrictJsonValue(
                serde_json::json!({"type":"object","additionalProperties":false}),
            ),
            provides_services: Vec::new(),
            requires_services: Vec::new(),
            entrypoint: JavaScriptEntrypointMetadata {
                normalized_relative_path: "main.mjs".into(),
                module_digest: sha(ENTRYPOINT),
                host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            }
            .into(),
            contributions: PackageContributions {
                skills: vec![skill],
                ..Default::default()
            },
        },
        schemas: BTreeMap::new(),
        supported_targets: BTreeSet::from([
            "x86_64-pc-windows-msvc".into(),
            "aarch64-apple-darwin".into(),
            "x86_64-apple-darwin".into(),
            "x86_64-unknown-linux-gnu".into(),
            "aarch64-unknown-linux-gnu".into(),
        ]),
        minimum_node_major: MINIMUM_NODE_MAJOR,
        dependency_lock_digest: sha(b"nomifun-frozen-skill-no-dependencies-v1"),
        credential_slots: Vec::new(),
    };
    let artifact = PluginPackageArtifactV1::new(
        "skill-publication-preview".into(),
        manifest,
        files
            .iter()
            .map(|(path, bytes)| ArtifactFileDigest {
                normalized_relative_path: path.clone(),
                digest: sha(bytes),
                size_bytes: bytes.len() as u64,
            })
            .collect(),
    )
    .map_err(fail)?;
    files.insert(
        "manifest.json".into(),
        canonical_json_bytes(&artifact.manifest).map_err(fail)?,
    );
    Ok(Prepared {
        _capture_permit: None,
        preview: Preview {
            selection: captured.selection,
            source_digest: captured.source_digest,
            artifact_digest: artifact.artifact_digest.as_ref().into(),
            package_id,
            package_version,
            skill_id,
            files: inventory,
            expected_library_revision: 0,
            projects: Vec::new(),
        },
        files,
    })
}
