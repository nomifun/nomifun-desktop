//! Release gate for the official Agent preset/capability contract.
//!
//! Static seed validation is deliberately insufficient here. This test boots
//! the current Nomi-core product graph, reads the authenticated HTTP catalog,
//! then applies a real local Plugin so the production Registry publisher
//! reconciles and refreshes Agent availability before the same contract is
//! checked again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use nomifun_agent_contracts::{
    ActionId, ArtifactEnvelope, ArtifactFileDigest, ArtifactId,
    CapabilityActionDescriptor, CapabilityConsumer, CapabilityContributions,
    CapabilityId, CapabilityKind, CapabilityManifest, CanonicalSchemaRef, DigestHex,
    EffectClass, ExactVersionRef, JAVASCRIPT_HOST_PROTOCOL_VERSION,
    JAVASCRIPT_SDK_CONTRACT_VERSION, JavaScriptBuildProfile,
    JavaScriptEntrypointMetadata, LocalizedMetadata, MINIMUM_NODE_MAJOR,
    PLUGIN_N1_SCHEMA_VERSION, PLUGIN_PACKAGE_PROFILE_VERSION, PackageContributions,
    PackageId, PackageManifest, PlatformConstraint, PluginPackageArtifactV1,
    PluginPackageV1Manifest, RuntimeTarget, StrictJsonValue, ToolPresentationKind,
    VersionString, canonical_json_bytes, capability_surface_declarations,
};
use nomifun_api_types::{
    AgentPresetEditorResponse, AgentPresetLibraryResponse, ApiResponse,
    ApplyPluginCandidateRequest, ApplyPluginTargetDto, CapabilityCatalogItemDto,
    AgentChatModelSelectionDto, CatalogMaterializationStateDto,
    CreateAgentPresetFromTemplateRequest,
    ExactCatalogRefDto, ImportPluginRequest, PluginImportKindDto, PreviewStatusDto,
    ResolveAgentPresetPreviewRequest, ResolveAgentPresetPreviewResponse,
    SkillCatalogItemDto,
};
use nomifun_app::compatibility::{
    AppServices, build_module_states, create_router_with_states,
};
use nomifun_app::{AppConfig, AuthPolicy};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

const LOCAL_TRUST_SECRET: &str = "official-preset-catalog-integrity";

#[tokio::test]
async fn every_published_official_preset_stays_available_after_plugin_catalog_refresh() {
    let root = tempfile::tempdir().expect("allocate isolated product root");
    let database = nomifun_db::init_database_memory()
        .await
        .expect("initialize product database");
    let services = AppServices::from_config(
        database,
        &AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from(LOCAL_TRUST_SECRET)),
            ..AppConfig::default()
        },
    )
    .await
    .expect("compose product services");
    let exact_model = seed_search_and_vision_ready_chat_route(
        &services.database,
        &services.encryption_key,
    )
    .await;
    let owner_user_id = services.authoritative_user_id.to_string();
    let (states, _channel_components) = build_module_states(&services).await;
    let plugin_service = Arc::clone(&states.plugin.service);
    let router = create_router_with_states(&services, states);

    assert_official_preset_catalog_integrity(&router, "startup catalog", &exact_model).await;

    let main = br#"
        export async function activate() {
          return {
            capabilities: {
              "test.release_gate.echo.contribution": {
                async invoke({ input }) { return input; }
              }
            }
          };
        }
    "#;
    let artifact = plugin_artifact(main);
    let source = root.path().join("catalog-refresh-plugin");
    write_plugin_package(&source, &artifact, main);

    let before_import = plugin_service
        .list_library(&owner_user_id)
        .await
        .expect("read initial Plugin library");
    let project = plugin_service
        .import_prebuilt(
            &owner_user_id,
            ImportPluginRequest {
                expected_library_revision: before_import.library_revision,
                import_kind: PluginImportKindDto::PrebuiltArtifact,
                source_path: source.display().to_string(),
                expected_bundle_or_artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
                target_project_id: None,
                expected_project_revision: None,
            },
        )
        .await
        .expect("import refresh-trigger Plugin");
    let candidate = project
        .ready
        .as_ref()
        .expect("prebuilt import must create a ready candidate");
    let before_apply = plugin_service
        .list_library(&owner_user_id)
        .await
        .expect("read Plugin library before apply");
    plugin_service
        .apply_candidate(
            &owner_user_id,
            ApplyPluginCandidateRequest {
                project_id: project.summary.project_id.clone(),
                expected_project_revision: project.summary.project_revision,
                expected_build_generation: project.summary.build_generation,
                candidate_id: candidate.candidate.candidate_id.clone(),
                expected_candidate_digest: candidate.candidate.candidate_digest.clone(),
                target: ApplyPluginTargetDto::InitialInstall {
                    expected_library_revision: before_apply.library_revision,
                },
                allow_breaking: false,
                acknowledge_test_warning: true,
            },
        )
        .await
        .expect("apply Plugin through the production Registry publisher");

    assert_official_preset_catalog_integrity(
        &router,
        "catalog after Plugin Registry reconcile/availability refresh",
        &exact_model,
    )
    .await;

    services
        .shutdown_browser_platform()
        .await
        .expect("shutdown browser platform");
    services.database.close().await;
}

async fn seed_search_and_vision_ready_chat_route(
    database: &nomifun_db::Database,
    encryption_key: &[u8; 32],
) -> AgentChatModelSelectionDto {
    use nomifun_db::{
        CreateProviderParams, IProviderRepository, NewProviderModel,
        NewProviderModelCapability, SqliteProviderRepository,
    };

    let provider_id = Uuid::now_v7().to_string();
    let encrypted = nomifun_common::encrypt_string(
        r#"{"api_keys":["official-gate-only"]}"#,
        encryption_key,
    )
    .expect("encrypt isolated gate credential");
    let capabilities = [NewProviderModelCapability {
        task: "chat",
        traits: r#"["vision_input","function_calling","reasoning","web_search"]"#,
        protocol: "openai.responses",
        connection_role: "default",
        endpoint: Some("/responses"),
        provider_params: "{}",
        context_limit: Some(128_000),
        ..Default::default()
    }];
    SqliteProviderRepository::new(database.pool().clone())
        .create(
            CreateProviderParams {
                provider_id: Some(&provider_id),
                platform: "openai",
                name: "Official gate Responses search fixture",
                base_url: "https://api.openai.invalid/v1",
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: Some(-10_000),
            },
            &NewProviderModel {
                model: "official-gate-responses-search",
                enabled: true,
                sort_order: -10_000,
                description: Some("Isolated contract fixture; no external request is sent."),
                capabilities: &capabilities,
            },
            &[],
        )
        .await
        .expect("seed exact Responses search route for the isolated release gate");
    AgentChatModelSelectionDto {
        provider_id,
        model: "official-gate-responses-search".to_owned(),
    }
}

async fn assert_official_preset_catalog_integrity(
    router: &axum::Router,
    phase: &str,
    model: &AgentChatModelSelectionDto,
) {
    let library: AgentPresetLibraryResponse = get_data(
        router,
        "/api/agent-preset-templates?source=official",
    )
    .await;
    let capabilities: Vec<CapabilityCatalogItemDto> =
        get_data(router, "/api/capabilities").await;
    let skills: Vec<SkillCatalogItemDto> =
        get_data(router, "/api/agent-catalog/skills").await;

    assert!(
        !library.official_templates.is_empty(),
        "{phase}: the released product must publish at least one official preset"
    );
    let capability_by_ref = capabilities
        .iter()
        .map(|item| {
            (
                (
                    item.capability.id.clone(),
                    item.capability.version.clone(),
                ),
                item,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let capabilities_by_id = capabilities.iter().fold(
        BTreeMap::<&str, Vec<&CapabilityCatalogItemDto>>::new(),
        |mut index, item| {
            index.entry(&item.capability.id).or_default().push(item);
            index
        },
    );
    let skill_by_ref = skills
        .iter()
        .map(|item| {
            (
                (item.skill.id.clone(), item.skill.version.clone()),
                item,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut missing = Vec::new();
    let mut unavailable = Vec::new();
    for template in &library.official_templates {
        let template_key = format!("{:?}", template.template_key);
        let direct_capabilities = template
            .seed
            .enabled_capabilities
            .iter();
        for reference in direct_capabilities {
            require_available_exact_capability(
                phase,
                &template_key,
                reference,
                &capability_by_ref,
                &mut missing,
                &mut unavailable,
            );
        }

        for capability_id in &template.role_coverage.required_capability_ids {
            match capabilities_by_id.get(capability_id.as_str()) {
                None => missing.push(format!(
                    "{template_key}: role coverage requires missing capability {capability_id}"
                )),
                Some(items) if !items.iter().any(|item| capability_is_available(item)) => {
                    unavailable.push(format!(
                        "{template_key}: role coverage capability {capability_id} has no available published version ({})",
                        items
                            .iter()
                            .map(|item| capability_state(item))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                Some(_) => {}
            }
        }

        for skill_ref in &template.seed.skill_bindings {
            let Some(skill) = skill_by_ref.get(&(
                skill_ref.id.clone(),
                skill_ref.version.clone(),
            )) else {
                missing.push(format!(
                    "{template_key}: references missing skill {}@{}",
                    skill_ref.id, skill_ref.version
                ));
                continue;
            };
            for reference in &skill.required_capabilities {
                require_available_exact_capability(
                    phase,
                    &format!("{template_key} via skill {}@{}", skill_ref.id, skill_ref.version),
                    reference,
                    &capability_by_ref,
                    &mut missing,
                    &mut unavailable,
                );
            }
        }
    }

    assert!(
        missing.is_empty() && unavailable.is_empty(),
        "{phase}: every capability referenced by a published official preset must exist and be available; missing={missing:#?}; unavailable={unavailable:#?}"
    );

    for template in &library.official_templates {
        let template_key = serde_json::to_value(template.template_key)
            .expect("serialize official template key")
            .as_str()
            .expect("official template key is a string")
            .to_owned();
        let editor: AgentPresetEditorResponse = post_data(
            router,
            &format!("/api/agent-presets/from-template/{template_key}"),
            &CreateAgentPresetFromTemplateRequest {
                model: Some(model.clone()),
                reuse_existing: false,
                display_name: format!("Catalog gate {phase} {template_key}"),
                description: None,
                model_route_refs: BTreeMap::new(),
                chat_route_records: BTreeMap::new(),
            },
        )
        .await;
        let revision = editor
            .revision
            .as_ref()
            .unwrap_or_else(|| panic!("{phase}: {template_key} creation did not persist revision 1"));
        let preview: ResolveAgentPresetPreviewResponse = post_data(
            router,
            &format!(
                "/api/agent-presets/{}/resolve-preview",
                editor.preset.preset_id
            ),
            &ResolveAgentPresetPreviewRequest {
                expected_current_revision: Some(revision.reference.clone()),
                draft: editor.draft,
                scene: "agent_settings".to_owned(),
                surface: "desktop".to_owned(),
                audience: "owner".to_owned(),
            },
        )
        .await;
        assert_eq!(
            preview.status,
            PreviewStatusDto::Ready,
            "{phase}: {template_key} must preview as ready; diagnostics={:#?}",
            preview.diagnostics
        );
        assert!(
            preview.can_save_revision && preview.can_create_session,
            "{phase}: {template_key} must allow save/session creation; can_save_revision={}, can_create_session={}, diagnostics={:#?}",
            preview.can_save_revision,
            preview.can_create_session,
            preview.diagnostics
        );
    }
}

fn require_available_exact_capability(
    phase: &str,
    template: &str,
    reference: &ExactCatalogRefDto,
    capability_by_ref: &BTreeMap<(String, String), &CapabilityCatalogItemDto>,
    missing: &mut Vec<String>,
    unavailable: &mut Vec<String>,
) {
    match capability_by_ref.get(&(reference.id.clone(), reference.version.clone())) {
        None => missing.push(format!(
            "{phase}: {template} references missing capability {}@{}",
            reference.id, reference.version
        )),
        Some(item) if !capability_is_available(item) => unavailable.push(format!(
            "{phase}: {template} references unavailable capability {}@{} ({})",
            reference.id,
            reference.version,
            capability_state(item)
        )),
        Some(_) => {}
    }
}

fn capability_is_available(item: &CapabilityCatalogItemDto) -> bool {
    item.materialization_state == CatalogMaterializationStateDto::Materialized
        && item.unavailable_code.is_none()
}

fn capability_state(item: &CapabilityCatalogItemDto) -> String {
    format!(
        "{}@{} state={:?} unavailable_code={:?}",
        item.capability.id,
        item.capability.version,
        item.materialization_state,
        item.unavailable_code
    )
}

async fn get_data<T: serde::de::DeserializeOwned>(
    router: &axum::Router,
    path: &str,
) -> T {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("x-nomi-local-trust", LOCAL_TRUST_SECRET)
                .body(Body::empty())
                .expect("build catalog request"),
        )
        .await
        .expect("dispatch catalog request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read catalog response")
        .to_bytes();
    assert!(
        status.is_success(),
        "GET {path} returned {status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    let response: ApiResponse<T> =
        serde_json::from_slice(&bytes).expect("decode typed catalog response");
    assert!(response.success, "GET {path} returned an error envelope");
    response.data.expect("successful catalog response data")
}

async fn post_data<T, B>(router: &axum::Router, path: &str, body: &B) -> T
where
    T: serde::de::DeserializeOwned,
    B: serde::Serialize,
{
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("x-nomi-local-trust", LOCAL_TRUST_SECRET)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(body).expect("serialize request body"),
                ))
                .expect("build POST request"),
        )
        .await
        .expect("dispatch POST request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read POST response")
        .to_bytes();
    assert!(
        status.is_success(),
        "POST {path} returned {status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    let response: ApiResponse<T> =
        serde_json::from_slice(&bytes).expect("decode typed POST response");
    assert!(response.success, "POST {path} returned an error envelope");
    response.data.expect("successful POST response data")
}

fn plugin_artifact(main: &[u8]) -> PluginPackageArtifactV1 {
    let package = ExactVersionRef {
        id: PackageId::from("test.release-gate.plugin"),
        version: VersionString::from("1.0.0"),
    };
    let input_schema = StrictJsonValue(serde_json::json!({
        "additionalProperties": false,
        "properties": {"message": {"type": "string"}},
        "required": ["message"],
        "type": "object"
    }));
    let output_schema = StrictJsonValue(serde_json::json!({
        "additionalProperties": true,
        "type": "object"
    }));
    let input_ref = schema_ref("echo-input", &input_schema);
    let output_ref = schema_ref("echo-output", &output_schema);
    let capability = CapabilityManifest {
        id: CapabilityId::from("test.release-gate.plugin.echo"),
        contribution_id: "test.release_gate.echo.contribution".into(),
        version: VersionString::from("1.0.0"),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: display("Release Gate Echo", "Triggers a real Plugin Catalog refresh."),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop", "headless"],
            [CapabilityConsumer::Agent],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(serde_json::json!({
            "type": "object",
            "additionalProperties": false
        })),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from("test.release-gate.plugin.echo.invoke"),
                input_schema: input_ref.clone(),
                output_schema: output_ref.clone(),
                effect_class: EffectClass::Pure,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            ..Default::default()
        },
    };
    PluginPackageArtifactV1::new(
        ArtifactId::from(Uuid::now_v7().to_string()),
        PluginPackageV1Manifest {
            schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
            build_profile: JavaScriptBuildProfile::PluginPackageV1,
            build_profile_version: PLUGIN_PACKAGE_PROFILE_VERSION.into(),
            package: PackageManifest {
                schema_version: PLUGIN_N1_SCHEMA_VERSION.into(),
                host_contract_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                package_id: package.id,
                package_version: package.version,
                display: display(
                    "Release Gate Plugin",
                    "Product Catalog refresh fixture.",
                ),
                package_dependencies: Vec::new(),
                requires_runtime_features: Vec::new(),
                config_schema: StrictJsonValue(serde_json::json!({
                    "type": "object",
                    "additionalProperties": false
                })),
                provides_services: Vec::new(),
                requires_services: Vec::new(),
                entrypoint: JavaScriptEntrypointMetadata {
                    normalized_relative_path: "main.mjs".into(),
                    module_digest: sha256(main),
                    host_protocol_version: JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                    sdk_contract_version: JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
                }
                .into(),
                contributions: PackageContributions {
                    capabilities: vec![capability],
                    ..Default::default()
                },
            },
            schemas: BTreeMap::from([
                (input_ref, input_schema),
                (output_ref, output_schema),
            ]),
            supported_targets: BTreeSet::from([native_plugin_target()]),
            minimum_node_major: MINIMUM_NODE_MAJOR,
            dependency_lock_digest: DigestHex::from("d".repeat(64)),
            credential_slots: Vec::new(),
        },
        vec![ArtifactFileDigest {
            normalized_relative_path: "main.mjs".into(),
            digest: sha256(main),
            size_bytes: u64::try_from(main.len()).expect("Plugin fixture size"),
        }],
    )
    .expect("build valid Plugin package")
}

fn schema_ref(name: &str, schema: &StrictJsonValue) -> CanonicalSchemaRef {
    CanonicalSchemaRef::from(format!(
        "schema://test.release-gate.plugin/{name}@1#{}",
        nomifun_agent_contracts::digest_payload(&schema.0)
            .expect("digest schema")
            .as_ref()
    ))
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn write_plugin_package(root: &Path, artifact: &PluginPackageArtifactV1, main: &[u8]) {
    std::fs::create_dir_all(root).expect("create Plugin fixture directory");
    std::fs::write(
        root.join("manifest.json"),
        canonical_json_bytes(
            &ArtifactEnvelope::new(artifact.manifest.payload.clone())
                .expect("build Plugin manifest envelope"),
        )
        .expect("serialize Plugin manifest"),
    )
    .expect("write Plugin manifest");
    std::fs::write(root.join("main.mjs"), main).expect("write Plugin entrypoint");
}

fn sha256(bytes: &[u8]) -> DigestHex {
    DigestHex::from(format!("{:x}", Sha256::digest(bytes)))
}

fn native_plugin_target() -> RuntimeTarget {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        (os, arch) => panic!("unsupported Plugin test host {os}/{arch}"),
    };
    RuntimeTarget::from(target)
}
