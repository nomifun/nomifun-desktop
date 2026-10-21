//! Release gate for the official Agent preset/capability contract.
//!
//! Static seed validation is deliberately insufficient here. This test boots
//! the current Nomi-core product graph, reads the authenticated HTTP catalog,
//! then imports an ordinary application through the production hosting owner
//! before the same Agent contract is checked again.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use nomifun_agent_contracts::{OfficialPresetKey, official_preset_seed_manifest_payload};
use nomifun_api_types::{
    AgentCatalogResponse, AgentPresetEditorResponse, AgentPresetLibraryResponse, AgentPresetRevisionDto, ApiResponse,
    CapabilitySelectionDto, ConversationResponse,
    CapabilityCatalogItemDto,
    AgentChatModelSelectionDto, CapabilityRefDto, CatalogMaterializationStateDto,
    CreateAgentPresetFromTemplateRequest,
    CreateAgentSessionResponseDto, SaveAgentPresetRevisionResponse, SkillCatalogItemDto,
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
    assert!(editor.draft.document.enabled_capabilities.iter()
        .all(|selection| selection.capability.id != "app.hosting"),
        "communicating application development is opt-in, including for the General Agent");
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

    // Independent release expectations describe the complete supported tasks.
    // Reading the seed or production catalog to build this matrix would let a
    // missing default Action silently weaken both sides of the assertion.
    const FILES: &[&str] = &["workspace.files/delete", "workspace.files/patch", "workspace.files/read", "workspace.files/rename", "workspace.files/search", "workspace.files/write"];
    const PROCESS: &[&str] = &["workspace.process/cancel", "workspace.process/close_stdin", "workspace.process/input", "workspace.process/poll", "workspace.process/resize", "workspace.process/start"];
    const ARTIFACTS: &[&str] = &["workspace.artifacts/publish", "workspace.artifacts/read"];
    const VCS: &[&str] = &["workspace.vcs/commit", "workspace.vcs/diff", "workspace.vcs/push", "workspace.vcs/stage", "workspace.vcs/status"];
    const PROJECT_MEMORY: &[&str] = &["project.memory/read", "project.memory/write"];
    const WEB: &[&str] = &["web.research/fetch", "web.research/search"];
    const KNOWLEDGE: &[&str] = &["knowledge/autogen", "knowledge/read", "knowledge/search", "knowledge/write"];
    const SCHEDULE: &[&str] = &["automation.schedule/create", "automation.schedule/delete", "automation.schedule/list", "automation.schedule/update"];
    const BROWSER: &[&str] = &["browser/act", "browser/download", "browser/evaluate", "browser/navigate", "browser/observe", "browser/render_content", "browser/upload"];
    const COMPUTER: &[&str] = nomifun_agent_domain_wave2::COMPUTER_ACTION_IDS;
    const COLLABORATION: &[&str] = &["agent/delegate", "agent/fork", "agent/request_user_decision"];
    const MEDIA: &[&str] = &["creation.media/audio", "creation.media/image", "creation.media/image_edit", "creation.media/music", "creation.media/text", "creation.media/video"];
    const OFFICE: &[&str] = &["office/document.edit", "office/preview", "office/sheet.edit", "office/slides.edit"];
    const ASSETS: &[&str] = &["creative.workshop/asset.read", "creative.workshop/asset.write"];
    const SSH: &[&str] = &["ssh/exec", "ssh/fs.read", "ssh/fs.write", "ssh/sudo"];
    const KNOWLEDGE_MANAGEMENT: &[&str] = &["knowledge.management/create", "knowledge.management/delete", "knowledge.management/inspect", "knowledge.management/list", "knowledge.management/update"];
    const SESSION_MANAGEMENT: &[&str] = &[
        "session.management/cancel_turn", "session.management/create",
        "session.management/execution_adjust", "session.management/execution_cancel",
        "session.management/execution_list", "session.management/execution_retry_step",
        "session.management/execution_status", "session.management/list",
        "session.management/send_input", "session.management/status",
        "session.management/terminal_create", "session.management/terminal_input",
        "session.management/terminal_list", "session.management/terminal_read",
        "session.management/terminal_relaunch", "session.management/terminal_stop",
    ];
    let mut expected = BTreeMap::from([
        (OfficialPresetKey::ChatMinimal, actions(&[
            ("workspace.files", FILES),
            ("workspace.process", PROCESS),
        ])),
        (OfficialPresetKey::AssistantGeneral, actions(&[
            ("model.management", &["model.management/add_model", "model.management/create_provider", "model.management/inspect"]),
            ("knowledge", KNOWLEDGE),
            ("project.memory", PROJECT_MEMORY),
            ("web.research", WEB),
            ("automation.schedule", SCHEDULE),
            ("browser", BROWSER),
            ("computer", COMPUTER),
            ("workspace.artifacts", ARTIFACTS),
            ("workspace.files", FILES),
            ("workspace.process", PROCESS),
            ("workspace.vcs", VCS),
            ("agent.collaboration", COLLABORATION),
            ("requirements", &["requirements/claim", "requirements/read", "requirements/status", "requirements/write"]),
            ("creation.media", MEDIA),
            ("creative.workshop", ASSETS),
            ("office", OFFICE),
            ("ssh", SSH),
            ("knowledge.management", KNOWLEDGE_MANAGEMENT),
            ("session.management", SESSION_MANAGEMENT),
        ])),
        (OfficialPresetKey::CodingCodex, actions(&[
            ("workspace.files", FILES),
            ("workspace.vcs", VCS),
            ("workspace.process", PROCESS),
            ("workspace.artifacts", ARTIFACTS),
            ("project.memory", PROJECT_MEMORY),
            ("web.research", WEB),
            ("agent.collaboration", COLLABORATION),
            ("knowledge", KNOWLEDGE),
            ("ssh", SSH),
            ("knowledge.management", KNOWLEDGE_MANAGEMENT),
            ("session.management", SESSION_MANAGEMENT),
        ])),
        (OfficialPresetKey::CompanionDefault, actions(&[
            ("companion", &["companion/evolve", "companion/learn"]),
            ("companion.memory", &["companion.memory/recall", "companion.memory/write"]),
            ("knowledge", KNOWLEDGE),
            ("channel.messaging", &["channel.messaging/reply", "channel.messaging/send"]),
            ("automation.schedule", SCHEDULE),
            ("workspace.files", FILES),
            ("workspace.process", PROCESS),
            ("workspace.artifacts", ARTIFACTS),
            ("project.memory", PROJECT_MEMORY),
            ("web.research", WEB),
            ("browser", BROWSER),
            ("computer", COMPUTER),
            ("creation.media", MEDIA),
            ("creative.workshop", ASSETS),
            ("office", OFFICE),
            ("ssh", SSH),
            ("agent.collaboration", COLLABORATION),
            ("knowledge.management", KNOWLEDGE_MANAGEMENT),
            ("session.management", SESSION_MANAGEMENT),
            ("robot", &["robot/device", "robot/display", "robot/motion", "robot/vision"]),
        ])),
        (OfficialPresetKey::CustomerServiceDefault, actions(&[
            ("customer.service", &["customer.service/handoff", "customer.service/notes.read", "customer.service/notes.write"]),
            ("knowledge", &["knowledge/read", "knowledge/search"]),
            ("channel.messaging", &["channel.messaging/reply", "channel.messaging/send"]),
            ("workspace.files", FILES),
            ("workspace.process", PROCESS),
            ("workspace.artifacts", ARTIFACTS),
            ("automation.schedule", SCHEDULE),
            ("web.research", WEB),
        ])),
        (OfficialPresetKey::CreativeStudioDefault, actions(&[
            ("creation.media", MEDIA),
            ("creative.workshop", &["creative.workshop/asset.read", "creative.workshop/asset.write", "creative.workshop/canvas.edit", "creative.workshop/canvas.read", "creative.workshop/template.run"]),
            ("office", OFFICE),
            ("workspace.files", FILES),
            ("workspace.artifacts", ARTIFACTS),
            ("workspace.process", PROCESS),
            ("project.memory", PROJECT_MEMORY),
            ("web.research", WEB),
            ("browser", BROWSER),
            ("agent.collaboration", COLLABORATION),
            ("automation.schedule", SCHEDULE),
        ])),
    ]);
    expected.get_mut(&OfficialPresetKey::CompanionDefault).unwrap()
        .get_mut("session.management").unwrap()
        .insert("session.management/execution_answer_decision".to_owned());
    let mut app_expected = expected[&OfficialPresetKey::CodingCodex].clone();
    app_expected.extend(actions(&[("app.hosting", &["app.hosting/guide", "app.hosting/current", "app.hosting/import", "app.hosting/list",
        "app.hosting/update", "app.hosting/preview", "app.hosting/rebuild", "app.hosting/start", "app.hosting/status", "app.hosting/stop"])]));
    expected.insert(OfficialPresetKey::AppDeveloper, app_expected);
    let manifest = official_preset_seed_manifest_payload();
    assert_eq!(manifest.templates.keys().filter(|key|key.as_str().starts_with("companion.")).copied()
        .collect::<BTreeSet<_>>(), BTreeSet::from([OfficialPresetKey::CompanionDefault]),
        "all Companion entries use one official Agent definition");
    assert!(manifest.templates[&OfficialPresetKey::CompanionDefault].required_resource_kinds.iter()
        .any(|kind|kind.as_ref()=="robot"), "device Actions retain their explicit resource requirement");
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
        let skill_ids = manifest.templates[&key].skill_bindings.iter()
            .map(|skill| (skill.id.as_ref().to_owned(), skill.version.as_ref().to_owned()))
            .collect::<BTreeSet<_>>();
        let expected_skills = if key == OfficialPresetKey::CreativeStudioDefault {
            BTreeSet::from([
                ("creative-studio-canvas".to_owned(), "1.0.0".to_owned()),
                ("creative-studio-organize".to_owned(), "1.0.0".to_owned()),
                ("creative-studio-template".to_owned(), "1.0.0".to_owned()),
            ])
        } else {
            BTreeSet::new()
        };
        assert_eq!(skill_ids, expected_skills, "{} default Skill selection changed", key.as_str());
    }

    let minimal = &manifest.templates[&OfficialPresetKey::ChatMinimal];
    assert_eq!(minimal.required_resource_kinds.iter().map(|kind| kind.as_ref()).collect::<BTreeSet<_>>(),
        BTreeSet::from(["process_session", "workspace"]));
    assert!(minimal.required_runtime_features.is_empty());
    let registration=nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
    let process=registration.metadata.manifest.payload.contributions.capabilities.into_iter()
        .find(|capability|capability.id.as_ref()=="workspace.process").unwrap();
    let start=process.contributions.actions.iter().find(|action|action.action_id.as_ref()=="workspace.process/start").unwrap();
    let launch=nomifun_agent_domain_wave2::resolve_action_schema("workspace.process",&start.input_schema).unwrap();
    assert!(launch.0["properties"]["timeout_ms"].get("default").is_none(),"published launch has no implicit process deadline");
    assert!(launch.0["properties"]["timeout_ms"].get("maximum").is_none(),"an explicit long workload deadline is not a human-wait budget");
    assert_eq!(launch.0["properties"]["wait_ms"]["default"],1000);
    let coding = &manifest.templates[&OfficialPresetKey::CodingCodex];
    assert!(coding.enabled_capabilities.iter().all(|selection| {
        !matches!(selection.capability.id.as_ref(), "browser" | "computer")
    }), "Coding's default contract must remain usable on a headless host");
    assert!(coding.required_resource_kinds.iter().all(|kind| {
        !matches!(kind.as_ref(), "browser" | "computer")
    }));
    assert!(coding.required_runtime_features.iter().any(|feature| feature.as_ref() == "code_mode"));
}

#[tokio::test]
async fn nomifun_app_development_is_optional_and_preserves_personal_grants_and_frozen_sessions() {
    let root = tempfile::tempdir().expect("allocate isolated coding Agent root");
    let database = nomifun_db::init_database_memory().await.expect("initialize product database");
    let services = AppServices::from_config(database, &AppConfig {
        data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(Arc::from(LOCAL_TRUST_SECRET)), ..AppConfig::default()
    }).await.expect("compose product services");
    let model = seed_search_and_vision_ready_chat_route(&services.database, &services.encryption_key).await;
    let (states, _channel_components) = build_module_states(&services).await;
    let router = create_router_with_states(&services, states);
    let create = |name: &str| CreateAgentPresetFromTemplateRequest {
        model: Some(model.clone()), reuse_existing: false, display_name: name.into(),
        description: None, model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new(),
    };
    let personal: AgentPresetEditorResponse = post_data(&router,
        "/api/agent-presets/from-template/coding.codex", &create("Personal coding Agent")).await;
    assert!(personal.draft.document.enabled_capabilities.iter()
        .all(|selection| selection.capability.id != "app.hosting"), "ordinary Coding Agents do not inject application tools by default");
    let catalog: AgentCatalogResponse = get_data(&router, "/api/agent-catalog").await;
    let candidate = catalog.modules.iter().find(|module| module.module.id == "app.hosting")
        .expect("the application development module remains an optional workbench candidate");
    let hosting = CapabilitySelectionDto {
        capability: candidate.module.clone(),
        action_allowlist: candidate.actions.iter().map(|action| action.action_id.clone()).collect(),
    };
    assert_eq!(hosting.action_allowlist, BTreeSet::from([
        "app.hosting/guide", "app.hosting/current", "app.hosting/import", "app.hosting/list", "app.hosting/update", "app.hosting/preview", "app.hosting/rebuild", "app.hosting/start", "app.hosting/status", "app.hosting/stop"
    ].map(str::to_owned)));
    for capability in ["workspace.files", "workspace.process"] {
        assert!(personal.draft.document.enabled_capabilities.iter().any(|selection| selection.capability.id == capability));
    }

    // An ordinary saved personal Agent and Session start without the module.
    let preset_id = &personal.preset.preset_id;
    let open = |preset_id: &str| serde_json::json!({
        "preset_id": preset_id, "title": "Coding application development",
        "resource_selections": official_session_resources("coding.codex", "", ""),
    });
    let frozen: CreateAgentSessionResponseDto = post_data(&router, "/api/agent-sessions", &open(preset_id)).await;
    let frozen_projection: ConversationResponse = get_data(&router,
        &format!("/api/agent-sessions/{}/projection", frozen.agent_session_id)).await;
    let frozen_snapshot = frozen_projection.agent_snapshot.expect("persisted Session snapshot");
    assert!(!frozen_snapshot.enabled_capability_actions.contains_key("app.hosting"));
    let frozen_revision: AgentPresetRevisionDto = get_data(&router, &format!(
        "/api/agent-presets/{}/revisions/{}", frozen.agent_binding.preset_revision_ref.preset_id,
        frozen.agent_binding.preset_revision_ref.revision)).await;

    let still_narrow: AgentPresetEditorResponse = get_data(&router,
        &format!("/api/agent-presets/{preset_id}/editor")).await;
    assert_eq!(still_narrow.revision, personal.revision);
    assert!(!still_narrow.draft.document.enabled_capabilities.iter().any(|selection| selection.capability.id == "app.hosting"));

    // Preserve an explicit older five-Action selection when the candidate adds
    // the optional guide Action. Catalog additions are not new user grants.
    let prior_selection = CapabilitySelectionDto { capability: hosting.capability.clone(), action_allowlist: BTreeSet::from([
        "app.hosting/import", "app.hosting/list", "app.hosting/start", "app.hosting/status", "app.hosting/stop"].map(str::to_owned)) };
    let mut expanded_draft = still_narrow.draft;
    expanded_draft.document.enabled_capabilities.push(prior_selection.clone());
    let expanded: SaveAgentPresetRevisionResponse = post_data(&router, &format!("/api/agent-presets/{preset_id}/revisions"),
        &serde_json::json!({
            "expected_current_revision": still_narrow.revision.unwrap().reference,
            "draft": expanded_draft,
        })).await;
    assert!(expanded.revision.document.enabled_capabilities.contains(&prior_selection));
    let next: CreateAgentSessionResponseDto = post_data(&router, "/api/agent-sessions", &open(preset_id)).await;
    let next_projection: ConversationResponse = get_data(&router,
        &format!("/api/agent-sessions/{}/projection", next.agent_session_id)).await;
    let selected_snapshot = next_projection.agent_snapshot.unwrap();
    assert_eq!(selected_snapshot.enabled_capability_actions.get("app.hosting"), Some(&prior_selection.action_allowlist));
    let expanded_persisted: AgentPresetEditorResponse = get_data(&router,
        &format!("/api/agent-presets/{preset_id}/editor")).await;
    let stored_revision = expanded_persisted.revision.as_ref().unwrap();
    assert_eq!(stored_revision.reference, expanded.revision.reference);
    assert_eq!(stored_revision.document, expanded.revision.document);

    // Publishing defaults without this module cannot revoke a user's explicit
    // saved choice. New official Agents still require the same opt-in.
    let current: AgentPresetEditorResponse = post_data(&router,
        "/api/agent-presets/from-template/coding.codex", &create("Current official coding Agent")).await;
    assert!(current.draft.document.enabled_capabilities.iter().all(|selection| selection.capability.id != "app.hosting"));
    let still_expanded: AgentPresetEditorResponse = get_data(&router,
        &format!("/api/agent-presets/{preset_id}/editor")).await;
    assert_eq!(still_expanded.revision, expanded_persisted.revision,
        "official defaults cannot revoke explicit personal grants or rewrite the stored Revision");
    assert!(still_expanded.draft.document.enabled_capabilities.contains(&prior_selection));

    // The optional guide is granted only by a subsequent explicit user save;
    // that new Revision applies to a new Session while both older ones stay frozen.
    let mut with_guide_draft = still_expanded.draft;
    *with_guide_draft.document.enabled_capabilities.iter_mut()
        .find(|selection| selection.capability.id == "app.hosting").unwrap() = hosting.clone();
    let with_guide: SaveAgentPresetRevisionResponse = post_data(&router, &format!("/api/agent-presets/{preset_id}/revisions"),
        &serde_json::json!({
            "expected_current_revision": still_expanded.revision.unwrap().reference,
            "draft": with_guide_draft,
        })).await;
    assert!(with_guide.revision.document.enabled_capabilities.contains(&hosting));
    let guided: CreateAgentSessionResponseDto = post_data(&router, "/api/agent-sessions", &open(preset_id)).await;
    let guided_projection: ConversationResponse = get_data(&router,
        &format!("/api/agent-sessions/{}/projection", guided.agent_session_id)).await;
    assert_eq!(guided_projection.agent_snapshot.unwrap().enabled_capability_actions.get("app.hosting"), Some(&hosting.action_allowlist));
    let unchanged_selected: ConversationResponse = get_data(&router,
        &format!("/api/agent-sessions/{}/projection", next.agent_session_id)).await;
    assert_eq!(unchanged_selected.agent_snapshot.unwrap(), selected_snapshot,
        "explicitly adding a guide must not rewrite the earlier five-Action Session");
    let unchanged_projection: ConversationResponse = get_data(&router,
        &format!("/api/agent-sessions/{}/projection", frozen.agent_session_id)).await;
    assert_eq!(serde_json::to_value(unchanged_projection.agent_snapshot.unwrap()).unwrap(),
        serde_json::to_value(frozen_snapshot).unwrap(), "saving an Agent must not expand or rewrite the frozen Session");
    let unchanged_revision: AgentPresetRevisionDto = get_data(&router, &format!(
        "/api/agent-presets/{}/revisions/{}", frozen.agent_binding.preset_revision_ref.preset_id,
        frozen.agent_binding.preset_revision_ref.revision)).await;
    assert_eq!(unchanged_revision, frozen_revision, "the exact Session-only immutable Revision is preserved");
    services.database.close().await;
}

#[tokio::test]
async fn current_template_and_explicit_saved_official_create_distinct_canonical_bindings() {
    let root = tempfile::tempdir().expect("allocate isolated template selection root");
    std::fs::create_dir_all(root.path().join("work")).unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(database, &AppConfig {
        data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(Arc::from(LOCAL_TRUST_SECRET)), ..AppConfig::default()
    }).await.unwrap();
    let model = seed_search_and_vision_ready_chat_route(&services.database, &services.encryption_key).await;
    let (states, _channel_components) = build_module_states(&services).await;
    let router = create_router_with_states(&services, states);
    let library: AgentPresetLibraryResponse = get_data(&router, "/api/agent-preset-templates").await;
    let official = &library.official_presets[&nomifun_api_types::OfficialPresetKeyDto::ChatMinimal];
    let author: AgentPresetEditorResponse = get_data(&router,
        &format!("/api/agent-presets/{}/editor", official.preset_id)).await;
    let template: AgentPresetEditorResponse = post_data(&router,
        "/api/agent-presets/from-template/chat.minimal", &serde_json::json!({
            "model": model, "reuse_existing": true, "display_name": "Current Minimal"
        })).await;
    assert!(official.current_stable_revision.is_none());
    let untouched_selection = serde_json::json!({"kind":"preset", "preset_id":official.preset_id});
    let first_product: serde_json::Value = write_data(&router, "PUT",
        "/api/product-agent-bindings/customer/template-selection-fixture", &serde_json::json!({
            "selection": untouched_selection, "model": model,
        })).await;
    let first_cron: serde_json::Value = post_data(&router, "/api/cron/jobs", &serde_json::json!({
        "name":"Untouched official Preset first use", "message":"Do not execute this fixture",
        "schedule":{"kind":"at", "at_ms":4_102_444_800_000_i64},
        "created_by":"user", "execution_mode":"new_conversation",
        "agent_selection":untouched_selection, "model":model,
    })).await;
    for binding in [&first_product["agent_binding"], &first_cron["metadata"]["agent_binding"]] {
        assert!(!binding["typed_resource_bindings"].as_array().unwrap().is_empty());
        let reference = &binding["preset_revision_ref"];
        let revision: AgentPresetRevisionDto = get_data(&router, &format!(
            "/api/agent-presets/{}/revisions/{}", reference["preset_id"].as_str().unwrap(),
            reference["revision"].as_u64().unwrap())).await;
        assert_eq!(revision.document.instructions, template.draft.document.instructions);
        for selection in &template.draft.document.enabled_capabilities {
            assert!(revision.document.enabled_capabilities.contains(selection),
                "a valid untouched official Preset preserves each published baseline grant on first use");
        }
    }
    let after_first_use: AgentPresetLibraryResponse = get_data(&router, "/api/agent-preset-templates").await;
    assert!(after_first_use.official_presets[&nomifun_api_types::OfficialPresetKeyDto::ChatMinimal]
        .current_stable_revision.is_none(), "first use cannot author the stable official configuration");
    let mut draft = author.draft;
    draft.document = template.draft.document.clone();
    draft.document.instructions = "EXPLICIT_SAVED_OFFICIAL_INSTRUCTIONS".into();
    let process = draft.document.enabled_capabilities.iter_mut()
        .find(|selection| selection.capability.id == "workspace.process").unwrap();
    assert!(process.action_allowlist.remove("workspace.process/resize"));
    let saved: SaveAgentPresetRevisionResponse = post_data(&router,
        &format!("/api/agent-presets/{}/revisions", official.preset_id), &serde_json::json!({
            "expected_current_revision": draft.current_revision, "draft": draft,
            "reason": "Explicit current-generation official customization"
        })).await;
    let options: serde_json::Value = get_data(&router,
        &format!("/api/product-agent-bindings/customer/template-selection-fixture?provider_id={}&model={}",
            model.provider_id, model.model)).await;
    assert!(options["options"].as_array().unwrap().iter().any(|option|
        option["selection"] == serde_json::json!({"kind":"template", "template_key":"chat.minimal"})),
        "the current Template remains a distinct product choice: {options}");
    assert!(options["options"].as_array().unwrap().iter().any(|option|
        option["selection"] == serde_json::json!({"kind":"preset", "preset_id":official.preset_id})),
        "the saved official customization remains explicitly selectable: {options}");
    for (selection, instructions) in [
        (serde_json::json!({"kind":"template", "template_key":"chat.minimal"}), template.draft.document.instructions.as_str()),
        (serde_json::json!({"kind":"preset", "preset_id":official.preset_id}), "EXPLICIT_SAVED_OFFICIAL_INSTRUCTIONS"),
    ] {
        let product: serde_json::Value = write_data(&router, "PUT",
            "/api/product-agent-bindings/customer/template-selection-fixture", &serde_json::json!({
                "selection": selection, "model": model,
            })).await;
        let cron: serde_json::Value = post_data(&router, "/api/cron/jobs", &serde_json::json!({
            "name":"Template selection admission", "message":"Do not execute this fixture",
            "schedule":{"kind":"at", "at_ms":4_102_444_800_000_i64},
            "created_by":"user", "execution_mode":"new_conversation",
            "agent_selection":selection, "model":model,
        })).await;
        for binding in [&product["agent_binding"], &cron["metadata"]["agent_binding"]] {
            assert!(!binding["typed_resource_bindings"].as_array().unwrap().is_empty());
            let reference = &binding["preset_revision_ref"];
            let revision: AgentPresetRevisionDto = get_data(&router, &format!(
                "/api/agent-presets/{}/revisions/{}", reference["preset_id"].as_str().unwrap(),
                reference["revision"].as_u64().unwrap())).await;
            assert_eq!(revision.document.instructions, instructions,
                "product and Cron admission must honor the same explicit selection");
        }
    }

    // Both choices traverse the production HTTP resource preparation and
    // canonical opening owners. Template selection never reads the saved draft.
    let open = |preset_id: &str| serde_json::json!({
        "preset_id": preset_id, "model": model,
        "resource_selections": official_session_resources("chat.minimal", "", ""),
    });
    let explicit: CreateAgentSessionResponseDto = post_data(&router, "/api/agent-sessions", &open(&official.preset_id)).await;
    let explicit_projection: ConversationResponse = get_data(&router,
        &format!("/api/agent-sessions/{}/projection", explicit.agent_session_id)).await;
    let explicit_snapshot = explicit_projection.agent_snapshot.unwrap();
    assert_eq!(explicit_snapshot.instructions, "EXPLICIT_SAVED_OFFICIAL_INSTRUCTIONS");
    assert!(!explicit_snapshot.enabled_capability_actions["workspace.process"].contains("workspace.process/resize"));
    assert!(!explicit.agent_binding.typed_resource_bindings.is_empty());

    let current: AgentPresetEditorResponse = post_data(&router,
        "/api/agent-presets/from-template/chat.minimal", &serde_json::json!({
            "model": model, "reuse_existing": true, "display_name": "Current Minimal"
        })).await;
    assert_eq!(current.draft.document, template.draft.document);
    assert_ne!(current.preset.preset_id, official.preset_id);
    let opened: CreateAgentSessionResponseDto = post_data(&router, "/api/agent-sessions", &open(&current.preset.preset_id)).await;
    let current_projection: ConversationResponse = get_data(&router,
        &format!("/api/agent-sessions/{}/projection", opened.agent_session_id)).await;
    let current_snapshot = current_projection.agent_snapshot.unwrap();
    assert_eq!(current_snapshot.instructions, template.draft.document.instructions);
    assert!(current_snapshot.enabled_capability_actions["workspace.process"].contains("workspace.process/resize"));
    assert!(!opened.agent_binding.typed_resource_bindings.is_empty());
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    for response in [&explicit, &opened] {
        let canonical = store.get_live_session(&nomifun_agent_contracts::AgentSessionId::from(response.agent_session_id.clone())).await.unwrap();
        assert_eq!(serde_json::to_value(canonical.agent_binding).unwrap(), serde_json::to_value(&response.agent_binding).unwrap());
    }
    let unchanged: AgentPresetEditorResponse = get_data(&router,
        &format!("/api/agent-presets/{}/editor", official.preset_id)).await;
    assert_eq!(unchanged.revision.unwrap().reference, saved.revision.reference);
    assert_eq!(unchanged.draft.document.instructions, "EXPLICIT_SAVED_OFFICIAL_INSTRUCTIONS");
    let unchanged: ConversationResponse = get_data(&router,
        &format!("/api/agent-sessions/{}/projection", explicit.agent_session_id)).await;
    assert_eq!(unchanged.agent_snapshot.unwrap(), explicit_snapshot);
    services.cleanup_after_startup_failure(anyhow::anyhow!("template selection fixture ended")).await;
}

#[tokio::test]
async fn every_published_official_preset_stays_available_after_ordinary_app_import() {
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
    let corpus_version = nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
    nomifun_skill_library::materialize_if_needed(
        &services.data_dir, nomifun_skill_library::builtin_skills_corpus(), &corpus_version,
    ).await.expect("materialize the production builtin skill corpus for the isolated release gate");
    let (states, _channel_components) = build_module_states(&services).await;
    let skill_paths = states.skill.skill_paths.clone();
    let router = create_router_with_states(&services, states);

    assert_official_preset_catalog_integrity(&router, &services, &skill_paths, "startup catalog", &exact_model).await;

    let source = root.path().join("catalog-hosted-app");
    std::fs::create_dir_all(&source).expect("create ordinary application fixture");
    std::fs::write(source.join("index.html"), "<!doctype html><title>Catalog application</title>")
        .expect("write ordinary application fixture");
    let imported: serde_json::Value = post_data(
        &router,
        "/api/apps/import",
        &serde_json::json!({"source": source, "import_kind": "artifact", "name": "Catalog application"}),
    )
    .await;
    let app_id = imported["id"].as_str().expect("ordinary application registration");
    assert_eq!(services.app_hosting.get(app_id).await.unwrap().state, nomifun_app_hosting::AppState::Stopped);

    assert_official_preset_catalog_integrity(
        &router,
        &services,
        &skill_paths,
        "catalog after ordinary application import",
        &exact_model,
    )
    .await;

    services.app_hosting.delete(app_id).await.expect("remove disposable application");
    assert!(source.join("index.html").exists(), "application import and delete retain the source");

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
    services: &AppServices,
    skill_paths: &nomifun_skill_library::SkillPaths,
    phase: &str,
    model: &AgentChatModelSelectionDto,
) {
    let has_library_skills = !nomifun_skill_library::frozen::capture_inventory(skill_paths)
        .await.expect("capture the host's actual library inventory").skills.is_empty();
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

    let companion: serde_json::Value = post_data(
        router,
        "/api/companion/companions",
        &serde_json::json!({"name": "Catalog companion", "character": "mochi"}),
    )
    .await;
    let companion_id = companion["companion_id"].as_str().expect("fixture companion id");
    let customer: serde_json::Value = post_data(
        router,
        "/api/customer-service/agents",
        &serde_json::json!({"name": "Catalog customer", "provider_id": model.provider_id, "model": model.model}),
    )
    .await;
    let customer_id = customer["cs_agent_id"].as_str().expect("fixture customer-service Agent id");

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

        let session: CreateAgentSessionResponseDto = post_data(
            router,
            "/api/agent-sessions",
            &serde_json::json!({
                "preset_id": editor.preset.preset_id,
                "title": format!("Catalog gate {phase} {template_key} session"),
                "resource_selections": official_session_resources(&template_key, companion_id, customer_id),
            }),
        )
        .await;
        assert_eq!(session.state, "ready", "{phase}: {template_key} did not open a ready Session");
        let binding = &session.agent_binding;
        assert_eq!(binding.binding_version, 1);
        let bound_revision: AgentPresetRevisionDto = get_data(router, &format!(
            "/api/agent-presets/{}/revisions/{}", binding.preset_revision_ref.preset_id,
            binding.preset_revision_ref.revision,
        )).await;
        assert_eq!(bound_revision.reference, binding.preset_revision_ref);
        let mut expected_document = revision.document.clone();
        if !expected_document.enabled_capabilities.is_empty()
            || !expected_document.skill_bindings.is_empty() || has_library_skills {
            expected_document.enabled_capabilities.push(CapabilitySelectionDto {
                capability: CapabilityRefDto { id: "agent.tool-discovery".into() },
                action_allowlist: BTreeSet::from(["tool.discovery.rank".into()]),
            });
        }
        expected_document.enabled_capabilities.sort_by(|left, right| left.capability.id.cmp(&right.capability.id));
        assert_eq!(bound_revision.document, expected_document,
            "{phase}: {template_key} Session compilation must freeze the complete authored contract plus exact runtime discovery");
        let projection: ConversationResponse = get_data(router,
            &format!("/api/agent-sessions/{}/projection", session.agent_session_id)).await;
        let snapshot = projection.agent_snapshot.expect("Session projection retains its frozen snapshot");
        assert_eq!(snapshot.canonical_binding.as_ref(), Some(binding));
        assert_eq!(snapshot.preset_id, binding.preset_revision_ref.preset_id);
        assert_eq!(snapshot.preset_revision, i64::try_from(binding.preset_revision_ref.revision).unwrap());
        assert_eq!(snapshot.resolved_model, Some(nomifun_api_types::ExecutionModelRef {
            provider_id: model.provider_id.clone(), model: model.model.clone(),
        }));
        assert_eq!(snapshot.enabled_capability_actions, expected_document.enabled_capabilities.iter()
            .map(|selection| (selection.capability.id.clone(), selection.action_allowlist.clone())).collect());
        let expected_resources = official_session_resources(&template_key, companion_id, customer_id);
        let expected_resources = expected_resources.as_array().unwrap().iter()
            .map(|resource| (resource["resource_kind"].as_str().unwrap(), resource["resource_id"].as_str().unwrap())).collect::<BTreeSet<_>>();
        let actual_resources = binding.typed_resource_bindings.iter()
            .map(|resource| (resource.resource_kind.as_str(), resource.resource_id.as_str())).collect::<BTreeSet<_>>();
        assert_eq!(actual_resources, expected_resources);
        assert_eq!(binding.typed_resource_bindings.len(), expected_resources.len());
        for resource in &binding.typed_resource_bindings {
            assert_eq!(resource.owner_id, services.authoritative_user_id.as_ref());
            assert!(resource.connection_config_ref.is_none());
            let operations = match resource.resource_kind.as_str() {
                "process_session" => ["execute"].as_slice(),
                "browser" => ["act", "download", "evaluate", "navigate", "observe", "render_content", "upload"].as_slice(),
                "computer" => ["observe", "input", "launch"].as_slice(),
                "scheduler" => ["read", "write", "delete"].as_slice(),
                "workspace" | "project_memory" | "companion" | "companion_memory" | "asset_library" | "customer" => ["read", "write"].as_slice(),
                kind => panic!("unaccounted resource kind {kind}"),
            };
            assert_eq!(resource.operations, operations.iter().map(|operation| (*operation).to_owned()).collect());
            let parameters = match resource.resource_kind.as_str() {
                "workspace" | "process_session" => BTreeMap::from([("workspace_location".to_owned(), "managed-session".to_owned())]),
                "browser" => BTreeMap::from([("provider_kind".to_owned(), "managed".to_owned())]),
                "computer" => BTreeMap::from([("scope".to_owned(), "local_desktop".to_owned())]),
                _ => BTreeMap::new(),
            };
            assert_eq!(resource.typed_parameters, parameters);
            let canonical: nomifun_agent_contracts::TypedResourceBinding = serde_json::from_value(serde_json::to_value(resource).unwrap()).unwrap();
            assert_eq!(resource.binding_id, nomifun_agent_contracts::resource_definition_id(&canonical).unwrap().as_ref());
        }
    }
}

fn official_session_resources(
    template_key: &str,
    companion_id: &str,
    customer_id: &str,
) -> serde_json::Value {
    let resource = |kind: &str, id: &str| serde_json::json!({"resource_kind": kind, "resource_id": id});
    match template_key {
        "chat.minimal" => serde_json::json!([
            resource("process_session", "managed-process-session"),
            resource("workspace", "default-workspace"),
        ]),
        "assistant.general" => serde_json::json!([
            resource("asset_library", "creative-studio-assets"),
            resource("browser", "managed-browser"),
            resource("computer", "local-desktop"),
            resource("process_session", "managed-process-session"),
            resource("project_memory", "default-project-memory"),
            resource("scheduler", "installation-scheduler"),
            resource("workspace", "default-workspace"),
        ]),
        "coding.codex" | "app.developer" => serde_json::json!([
            resource("process_session", "managed-process-session"),
            resource("project_memory", "default-project-memory"),
            resource("workspace", "default-workspace"),
        ]),
        "companion.default" => serde_json::json!([
            resource("asset_library", "creative-studio-assets"),
            resource("browser", "managed-browser"),
            resource("companion", companion_id),
            resource("companion_memory", companion_id),
            resource("computer", "local-desktop"),
            resource("process_session", "managed-process-session"),
            resource("project_memory", "default-project-memory"),
            resource("scheduler", "installation-scheduler"),
            resource("workspace", "default-workspace"),
        ]),
        "customer-service.default" => serde_json::json!([
            resource("customer", customer_id),
            resource("process_session", "managed-process-session"),
            resource("scheduler", "installation-scheduler"),
            resource("workspace", "default-workspace"),
        ]),
        "creative-studio.default" => serde_json::json!([
            resource("asset_library", "creative-studio-assets"),
            resource("browser", "managed-browser"),
            resource("process_session", "managed-process-session"),
            resource("project_memory", "default-project-memory"),
            resource("scheduler", "installation-scheduler"),
            resource("workspace", "default-workspace"),
        ]),
        other => panic!("published official template {other} needs a session resource fixture"),
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
    write_data(router, "POST", path, body).await
}

async fn write_data<T, B>(router: &axum::Router, method: &str, path: &str, body: &B) -> T
where
    T: serde::de::DeserializeOwned,
    B: serde::Serialize,
{
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", LOCAL_TRUST_SECRET)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(body).expect("serialize request body"),
                ))
                .expect("build write request"),
        )
        .await
        .expect("dispatch write request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read write response")
        .to_bytes();
    assert!(
        status.is_success(),
        "{method} {path} returned {status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    let response: ApiResponse<T> =
        serde_json::from_slice(&bytes).expect("decode typed write response");
    assert!(response.success, "{method} {path} returned an error envelope");
    response.data.expect("successful write response data")
}
