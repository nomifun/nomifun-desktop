//! Exercise cold discovery through the actual product composition and HTTP route.
use super::*;
use nomifun_agent_contracts::{CapabilityRef, LogicalArtifactRef, SkillDefinition};
use nomifun_api_types::{
    AgentPresetDocumentDto, CapabilitySelectionDto, CreateAgentPresetRequest,
    CreateAgentSessionRequestDto, CreateAgentSessionResponseDto, SetPluginEnabledRequest,
    SlashCommandItem,
};

const SKILL: &str = "test.release-gate.plugin.guide";
const CONTEXT: &str = "test.release-gate.plugin.context";

#[tokio::test]
async fn cold_skill_commands_use_saved_binding_without_starting_runtime_or_context() {
    let root = tempfile::tempdir().unwrap();
    let services = AppServices::from_config(
        nomifun_db::init_database_memory().await.unwrap(),
        &AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from(LOCAL_TRUST_SECRET)),
            ..AppConfig::default()
        },
    )
    .await
    .unwrap();
    let model =
        seed_search_and_vision_ready_chat_route(&services.database, &services.encryption_key).await;
    let owner = services.authoritative_user_id.to_string();
    let (states, _) = build_module_states(&services).await;
    let plugins = states.plugin.service.clone();
    let router = create_router_with_states(&services, states);

    let marker = root.path().join("unexpected-activation");
    let main = format!(
        "import {{ writeFileSync }} from 'node:fs'; export async function activate() {{ writeFileSync({}, 'unexpected'); throw new Error('discovery must not activate'); }}",
        serde_json::to_string(&marker.to_string_lossy()).unwrap(),
    );
    let original = plugin_artifact(main.as_bytes());
    let mut manifest = original.manifest.payload.clone();
    let tool = manifest.package.contributions.capabilities[0].clone();
    let mut context = tool.clone();
    context.id = CONTEXT.into();
    context.contribution_id = "test.release_gate.context.contribution".into();
    context.kind = CapabilityKind::ContextContributor;
    context.contributions = CapabilityContributions {
        context_schema_refs: vec![tool.contributions.actions[0].output_schema.clone()],
        ..Default::default()
    };
    manifest.package.contributions.capabilities.push(context);
    let body =
        "---\ndescription: Exact package guide\n---\nRead this guidance before the first turn.";
    manifest.package.contributions.skills.push(SkillDefinition {
        id: SKILL.into(),
        version: "1.0.0".into(),
        package: tool.package.clone(),
        display: display("Package guide", "Exact package guide"),
        body_ref: LogicalArtifactRef {
            artifact_id: "guide".into(),
            normalized_relative_path: "resources/guide.md".into(),
            digest: sha256(body.as_bytes()),
        },
        resources: Vec::new(),
        requires_capabilities: vec![CapabilityRef {
            id: tool.id.clone(),
            version: tool.version.clone(),
        }],
        supported_surfaces: capability_surface_declarations(
            ["desktop", "headless"],
            [CapabilityConsumer::Agent],
        ),
    });
    let artifact = PluginPackageArtifactV1::new(
        Uuid::now_v7().to_string().into(),
        manifest,
        vec![
            ArtifactFileDigest {
                normalized_relative_path: "main.mjs".into(),
                digest: sha256(main.as_bytes()),
                size_bytes: main.len() as u64,
            },
            ArtifactFileDigest {
                normalized_relative_path: "resources/guide.md".into(),
                digest: sha256(body.as_bytes()),
                size_bytes: body.len() as u64,
            },
        ],
    )
    .unwrap();
    let source = root.path().join("package");
    write_plugin_package(&source, &artifact, main.as_bytes());
    std::fs::create_dir_all(source.join("resources")).unwrap();
    std::fs::write(source.join("resources/guide.md"), body).unwrap();
    let library = plugins.list_library(&owner).await.unwrap();
    let project = plugins
        .import_prebuilt(
            &owner,
            ImportPluginRequest {
                expected_library_revision: library.library_revision,
                import_kind: PluginImportKindDto::PrebuiltArtifact,
                source_path: source.display().to_string(),
                expected_bundle_or_artifact_digest: artifact.artifact_digest.as_ref().to_owned(),
                target_project_id: None,
                expected_project_revision: None,
            },
        )
        .await
        .unwrap();
    let candidate = &project.ready.as_ref().unwrap().candidate;
    let library = plugins.list_library(&owner).await.unwrap();
    let installed = plugins
        .apply_candidate(
            &owner,
            ApplyPluginCandidateRequest {
                project_id: project.summary.project_id.clone(),
                expected_project_revision: project.summary.project_revision,
                expected_build_generation: project.summary.build_generation,
                candidate_id: candidate.candidate_id.clone(),
                expected_candidate_digest: candidate.candidate_digest.clone(),
                target: ApplyPluginTargetDto::InitialInstall {
                    expected_library_revision: library.library_revision,
                },
                allow_breaking: false,
                acknowledge_test_warning: true,
            },
        )
        .await
        .unwrap();
    assert!(!marker.exists());

    let preset: AgentPresetEditorResponse = post_data(
        &router,
        "/api/agent-presets",
        &CreateAgentPresetRequest {
            display_name: "Cold package commands".into(),
            description: None,
            fork_from_revision: None,
            document: Some(AgentPresetDocumentDto {
                schema_version: "1.0.0".into(),
                model_route_refs: BTreeMap::new(),
                chat_route_records: BTreeMap::new(),
                enabled_capabilities: [tool.id.as_ref(), CONTEXT]
                    .into_iter()
                    .map(|id| CapabilitySelectionDto {
                        capability: ExactCatalogRefDto {
                            id: id.into(),
                            version: "1.0.0".into(),
                        },
                        action_allowlist: (id == tool.id.as_ref())
                            .then(|| {
                                BTreeSet::from([
                                    "test.release-gate.plugin.echo.invoke".into(),
                                ])
                            })
                            .unwrap_or_default(),
                    })
                    .collect(),
                skill_bindings: vec![ExactCatalogRefDto {
                    id: SKILL.into(),
                    version: "1.0.0".into(),
                }],
                system_role_provider_overrides: BTreeMap::new(),
                context_order: Vec::new(),
                middleware_order: Vec::new(),
                persona: String::new(),
                instructions: String::new(),
                starter_prompts: Vec::new(),
            }),
        },
    )
    .await;
    let session: CreateAgentSessionResponseDto = post_data(
        &router,
        "/api/agent-sessions",
        &CreateAgentSessionRequestDto {
            preset_id: preset.preset.preset_id,
            model: Some(model),
            title: Some("Not started".into()),
            resource_selections: Vec::new(),
            capability_selection: None,
        },
    )
    .await;
    let session_id = session.agent_session_id.as_str();
    assert_eq!(services.agent_runtime_sessions.active_runtime_count(), 0);
    assert!(services
        .conversation_repo
        .get(session_id)
        .await
        .unwrap()
        .is_none(), "canonical command discovery must not create a legacy Conversation row");
    let path = format!("/api/agent-sessions/{session_id}/slash-commands");
    for _ in 0..2 {
        let commands: Vec<SlashCommandItem> = get_data(&router, &path).await;
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command, format!("skill:{SKILL}"));
        assert_eq!(commands[0].description, "Exact package guide");
    }
    assert_eq!(services.agent_runtime_sessions.active_runtime_count(), 0);
    assert!(
        !marker.exists(),
        "neither the engine nor Plugin Context should activate during discovery"
    );
    assert!(services
        .conversation_repo
        .get(session_id)
        .await
        .unwrap()
        .is_none());

    plugins
        .set_enabled(
            &owner,
            SetPluginEnabledRequest {
                mount_id: installed.summary.mount_id.clone(),
                expected_mount_revision: installed.summary.mount_revision,
                expected_current_target_digest: installed
                    .summary
                    .current
                    .as_ref()
                    .unwrap()
                    .artifact_digest
                    .clone(),
                enabled: false,
            },
        )
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header("x-nomi-local-trust", LOCAL_TRUST_SECRET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "withdrawal must not return cached commands or a directory fallback"
    );
    assert_eq!(services.agent_runtime_sessions.active_runtime_count(), 0);
    assert!(!marker.exists());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
