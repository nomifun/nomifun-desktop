//! Release gate for the official Agent preset/capability contract.
//!
//! Static seed validation is deliberately insufficient here. This test boots
//! the current Nomi-core product graph, reads the authenticated HTTP catalog,
//! then installs a real Unified Plugin so the production Action registry is
//! exercised before the same Agent contract is checked again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use nomifun_agent_contracts::{OfficialPresetKey, official_preset_seed_manifest_payload};
use nomifun_api_types::{
    AgentCatalogResponse, AgentPresetEditorResponse, AgentPresetLibraryResponse, ApiResponse,
    CapabilityCatalogItemDto,
    AgentChatModelSelectionDto, CapabilityRefDto, CatalogMaterializationStateDto,
    CreateAgentPresetFromTemplateRequest, InspectPluginImportRequest,
    InstallPluginImportRequest, InstallPluginImportResponseDto, PluginImportInspectionDto,
    PluginImportKindDto, PluginInstallOutcomeDto, SkillCatalogItemDto,
};
use nomifun_app::compatibility::{
    AppServices, build_module_states, create_router_with_states,
};
use nomifun_app::{AppConfig, AuthPolicy};
use nomifun_browser_platform::runtime::{
    BrowserRuntime, BrowserRuntimeFactory, CreateBrowserRuntime, WorkspaceError,
};
use nomifun_browser_platform::workspace::BrowserResourceService;
use tower::ServiceExt;
use uuid::Uuid;

const LOCAL_TRUST_SECRET: &str = "official-preset-catalog-integrity";

struct CatalogGateBrowserFactory;

#[async_trait]
impl BrowserRuntimeFactory for CatalogGateBrowserFactory {
    async fn create(
        &self,
        _request: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        Err(WorkspaceError::NativeUnavailable)
    }
}

#[tokio::test]
async fn general_agent_compiles_when_browser_is_attachable_but_not_yet_connected() {
    let root = tempfile::tempdir().expect("allocate isolated product root");
    let database = nomifun_db::init_database_memory()
        .await
        .expect("initialize product database");
    let mut services = AppServices::from_config(
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
    assert!(services.browser_resources.is_none());
    services.attached_chrome = Some(nomifun_app::AttachedChromeProviderService::new());
    let model = seed_search_and_vision_ready_chat_route(
        &services.database,
        &services.encryption_key,
    )
    .await;
    let (states, _channel_components) = build_module_states(&services).await;
    let router = create_router_with_states(&services, states);

    let editor: AgentPresetEditorResponse = post_data(
        &router,
        "/api/agent-presets/from-template/assistant.general",
        &CreateAgentPresetFromTemplateRequest {
            model: Some(model),
            reuse_existing: false,
            display_name: "Attachable Browser general Agent".into(),
            description: None,
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
        },
    )
    .await;
    assert_eq!(
        editor.revision.expect("general Agent revision").reference.revision,
        1
    );
}

#[test]
fn official_preset_action_safety_matrix_is_exact() {
    fn actions(
        entries: &[(&'static str, &[&'static str])],
    ) -> BTreeMap<String, BTreeSet<String>> {
        entries.iter().map(|(module, actions)| (
            (*module).to_owned(),
            actions.iter().map(|action| (*action).to_owned()).collect(),
        )).collect()
    }

    let expected = BTreeMap::from([
        (OfficialPresetKey::ChatMinimal, actions(&[])),
        (OfficialPresetKey::AssistantGeneral, actions(&[
            ("knowledge", &["knowledge/autogen", "knowledge/read", "knowledge/search", "knowledge/write"]),
            ("project.memory", &["project.memory/read", "project.memory/write"]),
            ("web.research", &["web.research/fetch", "web.research/search"]),
            ("automation.schedule", &["automation.schedule/list"]),
            ("browser", &["browser/navigate", "browser/observe", "browser/render_content"]),
            ("computer", &["computer/a11y.observe", "computer/observe"]),
            ("workspace.artifacts", &["workspace.artifacts/read"]),
            ("workspace.files", &["workspace.files/patch", "workspace.files/read", "workspace.files/search", "workspace.files/write"]),
            ("workspace.process", &["workspace.process/cancel", "workspace.process/close_stdin", "workspace.process/exec", "workspace.process/input", "workspace.process/poll", "workspace.process/resize", "workspace.process/start"]),
            ("workspace.vcs", &["workspace.vcs/commit", "workspace.vcs/diff", "workspace.vcs/stage", "workspace.vcs/status"]),
            ("agent.collaboration", &["agent/delegate", "agent/fork", "agent/request_user_decision"]),
            ("requirements", &["requirements/claim", "requirements/read", "requirements/status", "requirements/write"]),
            ("creation.media", &["creation.media/audio", "creation.media/image", "creation.media/image_edit", "creation.media/music", "creation.media/video"]),
            ("agent.tool-discovery", &["tool.discovery.rank"]),
        ])),
        (OfficialPresetKey::CodingCodex, actions(&[
            ("workspace.files", &["workspace.files/patch", "workspace.files/read", "workspace.files/search", "workspace.files/write"]),
            ("workspace.vcs", &["workspace.vcs/commit", "workspace.vcs/diff", "workspace.vcs/stage", "workspace.vcs/status"]),
            ("workspace.process", &["workspace.process/cancel", "workspace.process/close_stdin", "workspace.process/exec", "workspace.process/input", "workspace.process/poll", "workspace.process/resize", "workspace.process/start"]),
            ("workspace.artifacts", &["workspace.artifacts/publish", "workspace.artifacts/read"]),
            ("project.memory", &["project.memory/read", "project.memory/write"]),
            ("web.research", &["web.research/fetch", "web.research/search"]),
            ("agent.collaboration", &["agent/delegate", "agent/fork", "agent/request_user_decision"]),
            ("agent.tool-discovery", &["tool.discovery.rank"]),
        ])),
        (OfficialPresetKey::CompanionDefault, actions(&[
            ("companion", &["companion/evolve", "companion/learn"]),
            ("companion.memory", &["companion.memory/recall", "companion.memory/write"]),
            ("knowledge", &["knowledge/read", "knowledge/search"]),
            ("channel.messaging", &["channel.messaging/reply"]),
            ("robot", &["robot/vision"]),
            ("automation.schedule", &["automation.schedule/create", "automation.schedule/delete", "automation.schedule/list", "automation.schedule/update"]),
            ("agent.tool-discovery", &["tool.discovery.rank"]),
        ])),
        (OfficialPresetKey::CustomerServiceDefault, actions(&[
            ("customer.service", &["customer.service/handoff", "customer.service/notes.read"]),
            ("knowledge", &["knowledge/read", "knowledge/search"]),
            ("channel.messaging", &["channel.messaging/reply"]),
        ])),
        (OfficialPresetKey::CreativeStudioDefault, actions(&[
            ("creation.media", &["creation.media/audio", "creation.media/image", "creation.media/image_edit", "creation.media/music", "creation.media/text", "creation.media/video"]),
            ("creative.workshop", &["creative.workshop/asset.read", "creative.workshop/asset.write", "creative.workshop/canvas.edit", "creative.workshop/canvas.read", "creative.workshop/template.run"]),
            ("office", &["office/document.edit", "office/preview", "office/sheet.edit", "office/slides.edit"]),
            ("workspace.files", &["workspace.files/patch", "workspace.files/read", "workspace.files/search", "workspace.files/write"]),
            ("workspace.artifacts", &["workspace.artifacts/publish", "workspace.artifacts/read"]),
            ("workspace.process", &["workspace.process/cancel", "workspace.process/close_stdin", "workspace.process/exec", "workspace.process/input", "workspace.process/poll", "workspace.process/resize", "workspace.process/start"]),
            ("project.memory", &["project.memory/read", "project.memory/write"]),
            ("web.research", &["web.research/fetch", "web.research/search"]),
            ("agent.tool-discovery", &["tool.discovery.rank"]),
        ])),
    ]);
    let manifest = official_preset_seed_manifest_payload();
    for key in OfficialPresetKey::ALL {
        let actual = manifest.templates[&key].enabled_capabilities.iter()
            .map(|selection| (
                selection.capability.id.as_ref().to_owned(),
                selection.action_allowlist.iter()
                    .map(|action| action.as_ref().to_owned())
                    .collect::<BTreeSet<_>>(),
            ))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual, expected[&key], "{} default Action matrix changed", key.as_str());
    }
}

#[tokio::test]
async fn every_published_official_preset_stays_available_after_unified_plugin_install() {
    let root = tempfile::tempdir().expect("allocate isolated product root");
    let database = nomifun_db::init_database_memory()
        .await
        .expect("initialize product database");
    let mut services = AppServices::from_config(
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
    // This gate validates the desktop release graph, not the headless server
    // graph. The runtime itself is never opened; presence of the host-owned
    // Browser Resource Service is the exact boot fact that selects the native
    // Browser execution-role provider during compilation.
    services.browser_resources = Some(Arc::new(BrowserResourceService::new(Arc::new(
        CatalogGateBrowserFactory,
    ))));
    let exact_model = seed_search_and_vision_ready_chat_route(
        &services.database,
        &services.encryption_key,
    )
    .await;
    let (states, _channel_components) = build_module_states(&services).await;
    let router = create_router_with_states(&services, states);

    assert_official_preset_catalog_integrity(&router, "startup catalog", &exact_model).await;

    let source = root.path().join("catalog-refresh-plugin");
    write_unified_plugin_package(&source);
    let inspection: PluginImportInspectionDto = post_data(
        &router,
        "/api/plugins/import/inspect",
        &InspectPluginImportRequest {
            source_path: source.display().to_string(),
            kind: PluginImportKindDto::Directory,
        },
    )
    .await;
    let confirmation = inspection
        .permission_expansion
        .as_ref()
        .map(|value| value.confirmation_id.clone());
    let installed: InstallPluginImportResponseDto = post_data(
        &router,
        "/api/plugins/import",
        &InstallPluginImportRequest {
            source_path: source.display().to_string(),
            kind: PluginImportKindDto::Directory,
            expected_plugin_revision: None,
            create_copy: false,
            permission_confirmation_id: confirmation,
            config: None,
            credential_bindings: None,
        },
    )
    .await;
    assert!(matches!(installed.result, PluginInstallOutcomeDto::Installed { .. }));

    assert_official_preset_catalog_integrity(
        &router,
        "catalog after Unified Plugin Action registry install",
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
    let catalog: AgentCatalogResponse = get_data(router, "/api/agent-catalog").await;
    let skills: Vec<SkillCatalogItemDto> =
        get_data(router, "/api/agent-catalog/skills").await;

    assert!(
        !library.official_templates.is_empty(),
        "{phase}: the released product must publish at least one official preset"
    );
    let capability_by_ref = capabilities
        .iter()
        .map(|item| (item.capability.id.clone(), item))
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
    let module_by_ref = catalog
        .modules
        .iter()
        .map(|module| (module.module.id.clone(), module))
        .collect::<BTreeMap<_, _>>();

    let mut missing = Vec::new();
    let mut unavailable = Vec::new();
    for template in &library.official_templates {
        let template_key = format!("{:?}", template.template_key);
        let direct_capabilities = template
            .seed
            .enabled_capabilities
            .iter();
        for selection in direct_capabilities {
            require_available_capability(
                phase,
                &template_key,
                &selection.capability,
                &capability_by_ref,
                &mut missing,
                &mut unavailable,
            );
            match module_by_ref.get(&selection.capability.id) {
                None => missing.push(format!(
                    "{template_key}: Module catalog entry is missing for {}",
                    selection.capability.id,
                )),
                Some(module) => {
                    if module.display_name.trim().is_empty() || module.description.trim().is_empty() {
                        unavailable.push(format!(
                            "{template_key}: {} has no user-visible Module introduction",
                            selection.capability.id,
                        ));
                    }
                    let declared = module
                        .actions
                        .iter()
                        .map(|action| action.action_id.as_str())
                        .collect::<BTreeSet<_>>();
                    if selection.action_allowlist.is_empty()
                        || !selection
                            .action_allowlist
                            .iter()
                            .all(|action| declared.contains(action.as_str()))
                    {
                        unavailable.push(format!(
                            "{template_key}: {} contains an empty or undeclared exact Action grant",
                            selection.capability.id,
                        ));
                    }
                }
            }
        }

        for capability_id in &template.role_coverage.required_capability_ids {
            match capabilities_by_id.get(capability_id.as_str()) {
                None => missing.push(format!(
                    "{template_key}: role coverage requires missing capability {capability_id}"
                )),
                Some(items) if !items.iter().any(|item| capability_is_available(item)) => {
                    unavailable.push(format!(
                        "{template_key}: role coverage capability {capability_id} has no available publication ({})",
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
                require_available_capability(
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
        assert_eq!(revision.reference.revision, 1);
    }
}

fn require_available_capability(
    phase: &str,
    template: &str,
    reference: &CapabilityRefDto,
    capability_by_ref: &BTreeMap<String, &CapabilityCatalogItemDto>,
    missing: &mut Vec<String>,
    unavailable: &mut Vec<String>,
) {
    match capability_by_ref.get(&reference.id) {
        None => missing.push(format!(
            "{phase}: {template} references missing capability {}",
            reference.id
        )),
        Some(item) if !capability_is_available(item) => unavailable.push(format!(
            "{phase}: {template} references unavailable capability {} ({})",
            reference.id,
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
        "{} state={:?} unavailable_code={:?}",
        item.capability.id,
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

fn write_unified_plugin_package(root: &Path) {
    std::fs::create_dir_all(root).expect("create Plugin fixture directory");
    std::fs::write(
        root.join("nomifun.plugin.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "nomifun.plugin/v1",
            "id": "test.release-gate.plugin",
            "version": "1.0.0",
            "name": "Release Gate Plugin",
            "description": "Exercises the Unified Action registry",
            "hostApi": "1.0.0",
            "entrypoints": {"service": "service/main.mjs", "serviceMode": "onDemand"},
            "actions": {
                "echo": {
                    "name": "Echo",
                    "description": "Return the input",
                    "input": {"type": "object"},
                    "output": {"type": "object"},
                    "effect": "read"
                }
            },
            "bindings": [],
            "dataVersion": 0,
            "migrations": [],
            "configSchema": {"type": "object"},
            "secrets": [],
            "permissions": []
        }))
        .expect("serialize Unified Plugin manifest"),
    )
    .expect("write Plugin manifest");
    std::fs::create_dir_all(root.join("service")).expect("create Service directory");
    std::fs::write(
        root.join("service/main.mjs"),
        "export async function activate() { return { async invoke(_action, input) { return input; } }; }",
    )
    .expect("write Plugin Service");
}
