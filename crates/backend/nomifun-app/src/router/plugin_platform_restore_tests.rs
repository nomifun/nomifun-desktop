use super::tests::{
    assert_installed_context_reaches_nomi_prompt, package_artifact, test_runtime_authority,
    write_package,
};
use super::*;
use nomifun_agent_contracts::{
    ArtifactFileDigest, ArtifactId, CapabilityRef, PluginPackageArtifactV1, PluginSourceKind,
};
use nomifun_agent_control_plane::CatalogProvider;
use nomifun_agent_kernel::{InMemoryPluginStatePersistence, MaterializationPolicy};
use nomifun_api_types::{ApplyPluginTargetDto, PluginImportKindDto};

const ROLE: &str = "test.nomicore.plugin.context-role";
const FACADE: &str = "test.nomicore.plugin.context-facade";

fn package(kind: &str) -> (PluginPackageArtifactV1, Vec<u8>) {
    let package_id = format!("test.nomicore.{kind}");
    let contribution_id = format!("{package_id}.context.contribution");
    let main = if kind == "contract" {
        b"export async function activate() { return { capabilities: {} }; }".to_vec()
    } else {
        format!(
            r#"export async function activate() {{ return {{ capabilities: {{
            "{contribution_id}": {{ async contributeContext() {{
                return {{ instructions: "Context from the installed JS artifact" }};
            }} }}
        }} }}; }}"#
        )
        .into_bytes()
    };
    let original = package_artifact(&main);
    let mut manifest = original.manifest.payload.clone();
    let contributions = &mut manifest.package.contributions;
    if kind == "contract" {
        // Contract is an independent package; it exports no implementation.
        contributions
            .capabilities
            .retain(|c| c.id.as_ref() == FACADE);
        contributions.role_providers.clear();
    } else {
        manifest.package.package_id = package_id.clone().into();
        let mut implementation = contributions
            .capabilities
            .iter()
            .find(|c| c.id.as_ref() == "test.nomicore.plugin.context")
            .unwrap()
            .clone();
        implementation.id = format!("{package_id}.context").into();
        implementation.contribution_id = contribution_id.into();
        implementation.package.id = package_id.into();
        let reference = CapabilityRef {
            id: implementation.id.clone(),
            version: implementation.version.clone(),
        };
        if kind == "cycle" {
            implementation.requires = vec![reference.clone()];
        }
        contributions.capabilities = vec![implementation];
        contributions.role_contracts.clear();
        contributions.role_providers[0]
            .members
            .get_mut(&FACADE.into())
            .unwrap()
            .implementation = Some(reference);
        if kind == "independent" || kind == "cycle" {
            contributions.role_providers.clear();
        } else if kind == "broken" {
            contributions.role_providers[0].role.contract_digest =
                digest_payload(&"incompatible contract").unwrap();
        }
    }
    let referenced_schemas = contributions
        .capabilities
        .iter()
        .flat_map(|capability| capability.contributions.context_schema_refs.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    manifest
        .schemas
        .retain(|reference, _| referenced_schemas.contains(reference));
    let artifact = PluginPackageArtifactV1::new(
        ArtifactId::from(uuid::Uuid::now_v7().to_string()),
        manifest,
        vec![ArtifactFileDigest {
            normalized_relative_path: "main.mjs".into(),
            digest: nomifun_agent_contracts::digest_bytes(&main),
            size_bytes: main.len() as u64,
        }],
    )
    .unwrap();
    (artifact, main)
}

#[tokio::test]
async fn unscoped_kernel_failure_preserves_generation_until_bad_installation_is_disabled() {
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let (kernel, _, composition) = compose(database.pool(), root.path(), &owner).await;
    let independent = install(&composition.router, root.path(), &owner, "independent")
        .await
        .unwrap();
    let before = kernel.snapshot().unwrap();
    let publisher = &composition.runtime_participant.publisher;
    let published_before = publisher.dynamic_registrations.read().await.keys().cloned().collect::<BTreeSet<_>>();
    assert_eq!(published_before, BTreeSet::from([PluginMountId::from(independent.summary.mount_id.clone())]));
    let error = install(&composition.router, root.path(), &owner, "cycle")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cycle"), "{error}");
    let after = kernel.snapshot().unwrap();
    assert_eq!(publisher.dynamic_registrations.read().await.keys().cloned().collect::<BTreeSet<_>>(), published_before,
        "failed Plugin recovery must not change the source set reused by MCP refresh");
    assert!(
        Arc::ptr_eq(&before, &after),
        "failed recovery must not publish an empty or partial generation"
    );
    assert!(
        after
            .capability(&"test.nomicore.independent.context".into())
            .is_some()
    );

    let inventory = composition
        .runtime_participant
        .publisher
        .repository
        .inventory(&owner)
        .await
        .unwrap();
    let row = inventory
        .mounts
        .iter()
        .find(|mount| mount.mount_id == independent.summary.mount_id)
        .unwrap();
    let registration = composition
        .runtime_participant
        .publisher
        .registration_for(row)
        .await
        .unwrap();
    let mount: PluginMountId = row.mount_id.clone().into();
    let candidates = BTreeMap::from([(mount.clone(), registration)]);
    use nomifun_agent_kernel::KernelError;
    for error in [
        KernelError::CapabilityDependencyCycle,
        KernelError::RegistryPoisoned,
        KernelError::InvalidRegistration {
            mount_id: "not-a-candidate".into(),
            reason: "base failure".into(),
        },
    ] {
        assert!(registry_recovery::rejected_mounts(&error, &candidates).is_empty());
    }
    for error in [
        KernelError::InvalidRoleProvider {
            role_id: ROLE.into(),
            mount_id: mount.clone(),
            reason: "fixture".into(),
        },
        KernelError::MissingCapabilityDependency {
            capability_id: "test.nomicore.independent.context".into(),
            dependency_id: "missing.context".into(),
            dependency_version: "1.0.0".into(),
        },
        KernelError::DuplicatePackage {
            package_id: "test.nomicore.independent".into(),
        },
    ] {
        assert_eq!(
            registry_recovery::rejected_mounts(&error, &candidates),
            vec![mount.clone()]
        );
    }

    let library = composition
        .router
        .service
        .list_library(&owner)
        .await
        .unwrap();
    let cycle = library
        .plugins
        .iter()
        .find(|plugin| plugin.mount_id != independent.summary.mount_id)
        .unwrap();
    composition
        .router
        .service
        .set_enabled(
            &owner,
            SetPluginEnabledRequest {
                mount_id: cycle.mount_id.clone(),
                expected_mount_revision: cycle.mount_revision,
                expected_current_target_digest: cycle
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
    let recovered = kernel.snapshot().unwrap();
    assert_eq!(recovered.generation, before.generation + 1);
    assert!(
        recovered
            .capability(&"test.nomicore.independent.context".into())
            .is_some()
    );
    assert!(
        recovered
            .capability(&"test.nomicore.cycle.context".into())
            .is_none()
    );
}

#[tokio::test]
async fn persisted_defaults_refresh_saves_but_do_not_reselect_frozen_nomi_sessions() {
    use crate::router::nomi_core_control_plane::NomiCoreControlPlaneStore;
    use crate::router::nomi_core_role_defaults::NomiCoreRoleBindingStore;
    use nomifun_agent_contracts::{
        AgentBindingValue, PresetRevisionRef, PrincipalRef, RuntimeProfileKind,
    };
    use nomifun_agent_control_plane::{
        AgentControlPlane, ControlPlaneStore, OfficialTemplateCatalog, PresetRevisionCompiler,
    };
    use nomifun_agent_kernel::CompilerEnvironment;
    use nomifun_api_types::{
        CreateAgentPresetRequest, PutAgentRoleDefaultRequest, SaveAgentPresetRevisionRequest,
    };
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let (kernel, catalog, composition) = compose(database.pool(), root.path(), &owner).await;
    install(&composition.router, root.path(), &owner, "contract")
        .await
        .unwrap();
    let provider = install(&composition.router, root.path(), &owner, "provider")
        .await
        .unwrap();
    install(&composition.router, root.path(), &owner, "alternate")
        .await
        .unwrap();
    let candidates = catalog.snapshot().unwrap().as_api().unwrap();
    let role = candidates
        .roles
        .iter()
        .find(|r| r.role.key.role_id == ROLE)
        .unwrap();
    let a = role
        .providers
        .iter()
        .find(|p| p.source_package.id == "test.nomicore.provider")
        .unwrap()
        .selection
        .clone();
    let b = role
        .providers
        .iter()
        .find(|p| p.source_package.id == "test.nomicore.alternate")
        .unwrap()
        .selection
        .clone();
    let environment = CompilerEnvironment {
        resolver_version: "1.0.0".into(),
        required_runtime_protocol_version: "1.0.0".into(),
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: "1".repeat(64).into(),
        available_runtime_features: BTreeSet::new(),
        installation_role_bindings: BTreeMap::new(),
        canonical_schema_manifest_digest: "2".repeat(64).into(),
        target_contribution_manifest_digest: kernel.snapshot().unwrap().registry_digest.clone(),
        host_target: "x86_64-pc-windows-msvc".into(),
        host_surface: "desktop".into(),
        availability_evidence_revision: "default-flow".into(),
    };
    let templates = OfficialTemplateCatalog::load().unwrap();
    let store = Arc::new(NomiCoreControlPlaneStore::new(database.pool().clone()));
    let control = Arc::new(
        AgentControlPlane::new(
            store.clone(),
            catalog,
            templates.clone(),
            PresetRevisionCompiler::new()
                .with_canonical_registry(kernel.clone(), environment.clone()),
        )
        .with_installation_role_binding_store(Arc::new(NomiCoreRoleBindingStore::new(
            database.pool().clone(),
        ))),
    );
    let actor: nomifun_agent_contracts::UserId = owner.clone().into();
    assert_eq!(
        control
            .role_defaults(&"not-the-owner".into())
            .await
            .unwrap_err()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert!(control.role_defaults(&actor).await.unwrap().is_empty());
    let mut invalid = a.clone();
    invalid.provider_mount_id = "missing".into();
    assert!(
        control
            .put_role_default(
                &actor,
                ROLE,
                PutAgentRoleDefaultRequest {
                    selection: invalid,
                    expected_binding_version: 0,
                }
            )
            .await
            .is_err()
    );
    assert!(control.role_defaults(&actor).await.unwrap().is_empty());

    // Exercise the public PUT route with the same owner extension as production.
    use tower::ServiceExt;
    let router =
        nomifun_agent_control_plane::control_plane_router(control.clone()).layer(axum::Extension(
            nomifun_agent_control_plane::AuthenticatedOwner(actor.clone()),
        ));
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri(format!("/api/agent-role-defaults/{ROLE}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_vec(&PutAgentRoleDefaultRequest {
                        selection: a.clone(),
                        expected_binding_version: 0,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let editor = control
        .create_preset(
            &actor,
            CreateAgentPresetRequest {
                display_name: "Inherited Context".into(),
                description: None,
                fork_from_revision: None,
                document: None,
            },
        )
        .await
        .unwrap();
    let mut draft = editor.draft;
    draft.document.enabled_capabilities = vec![nomifun_api_types::CapabilitySelectionDto {
        capability: nomifun_api_types::ExactCatalogRefDto {
            id: FACADE.into(),
            version: "1.0.0".into(),
        },
        action_allowlist: BTreeSet::new(),
    }];
    let preset_id = draft.preset_id.clone();
    let first = control
        .save_revision(
            &actor,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                draft: draft.clone(),
                expected_current_revision: None,
                reason: None,
            },
        )
        .await
        .unwrap();
    let reference: PresetRevisionRef =
        serde_json::from_value(serde_json::to_value(&first.revision.reference).unwrap()).unwrap();
    let frozen = store.get_snapshot(&reference).await.unwrap().unwrap();
    let revision = store.get_revision(&reference).await.unwrap().unwrap();
    assert_eq!(
        frozen.content.resolved_role_providers[&ROLE.into()]
            .provider
            .mount_id
            .as_ref(),
        a.provider_mount_id
    );
    let updated_default = control
        .put_role_default(
            &actor,
            ROLE,
            PutAgentRoleDefaultRequest {
                selection: b.clone(),
                expected_binding_version: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated_default.binding_version, 2);
    assert_eq!(
        control
            .put_role_default(
                &actor,
                ROLE,
                PutAgentRoleDefaultRequest {
                    selection: a.clone(),
                    expected_binding_version: 1,
                }
            )
            .await
            .unwrap_err()
            .status(),
        StatusCode::CONFLICT
    );
    draft.current_revision = Some(first.revision.reference.clone());
    let second = control
        .save_revision(
            &actor,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                draft: draft.clone(),
                expected_current_revision: draft.current_revision.clone(),
                reason: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        second.revision.reference.revision,
        first.revision.reference.revision + 1
    );
    let second_ref: PresetRevisionRef =
        serde_json::from_value(serde_json::to_value(&second.revision.reference).unwrap()).unwrap();
    let second_snapshot = store.get_snapshot(&second_ref).await.unwrap().unwrap();
    assert_eq!(
        second_snapshot.content.resolved_role_providers[&ROLE.into()]
            .provider
            .mount_id
            .as_ref(),
        b.provider_mount_id
    );
    assert_eq!(
        store.get_snapshot(&reference).await.unwrap().unwrap(),
        frozen
    );

    // Runtime receives a different current default, but must revalidate the old lock.
    let mut runtime_environment = environment.clone();
    runtime_environment.installation_role_bindings =
        nomifun_db::load_installation_role_bindings(database.pool())
            .await
            .unwrap();
    let binding = AgentBindingValue {
        preset_revision_ref: reference,
        resolved_snapshot_ref: frozen.snapshot_ref.clone(),
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    };
    let principal = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: owner.clone(),
    };
    let compiled = crate::router::nomi_core_session::compile_nomi_plugin_snapshot(
        &kernel,
        &runtime_environment,
        binding.clone(),
        revision.clone(),
        frozen.clone(),
        &principal,
    )
    .unwrap();
    let session = nomifun_ai_agent::KernelNomiPluginToolSession::materialize(
        kernel.clone(),
        Arc::new(compiled),
        principal.clone(),
        uuid::Uuid::now_v7().to_string().into(),
        "session:default-freeze".into(),
        composition.schema_resolver.clone(),
    )
    .await
    .unwrap();
    assert!(
        session
            .system_prompt_with_initial_context(None)
            .unwrap()
            .unwrap()
            .contains("Context from the installed JS artifact")
    );

    draft.current_revision = Some(second.revision.reference.clone());
    draft
        .document
        .system_role_provider_overrides
        .insert(ROLE.into(), a.clone());
    let explicit = control
        .save_revision(
            &actor,
            &preset_id,
            SaveAgentPresetRevisionRequest {
                draft: draft.clone(),
                expected_current_revision: draft.current_revision.clone(),
                reason: None,
            },
        )
        .await
        .unwrap();
    let explicit_ref: PresetRevisionRef =
        serde_json::from_value(serde_json::to_value(&explicit.revision.reference).unwrap())
            .unwrap();
    assert_eq!(
        store
            .get_snapshot(&explicit_ref)
            .await
            .unwrap()
            .unwrap()
            .content
            .resolved_role_providers[&ROLE.into()]
            .provider
            .mount_id
            .as_ref(),
        a.provider_mount_id
    );
    composition
        .router
        .service
        .set_enabled(
            &owner,
            SetPluginEnabledRequest {
                mount_id: provider.summary.mount_id,
                expected_mount_revision: provider.summary.mount_revision,
                expected_current_target_digest: provider.summary.current.unwrap().artifact_digest,
                enabled: false,
            },
        )
        .await
        .unwrap();
    assert!(
        crate::router::nomi_core_session::compile_nomi_plugin_snapshot(
            &kernel,
            &runtime_environment,
            binding,
            revision,
            frozen,
            &principal,
        )
        .is_err(),
        "withdrawal must not silently execute the new default"
    );
    assert_eq!(
        control.role_defaults(&actor).await.unwrap()[0],
        updated_default
    );
}

pub(super) async fn compose(
    pool: &SqlitePool,
    root: &Path,
    owner: &str,
) -> (
    Arc<KernelRegistry>,
    Arc<KernelCatalogProvider>,
    NomiCorePluginComposition,
) {
    compose_with_base(pool, root, owner, Vec::new()).await
}

pub(super) async fn compose_with_base(
    pool: &SqlitePool, root: &Path, owner: &str, base: Vec<nomifun_agent_kernel::PluginRegistration>,
) -> (Arc<KernelRegistry>, Arc<KernelCatalogProvider>, NomiCorePluginComposition) {
    let mut policy = MaterializationPolicy::stable("1.0.0");
    policy
        .allowed_sources
        .insert(PluginSourceKind::ManagedLocal);
    let kernel = Arc::new(
        KernelRegistry::new(policy, Arc::new(InMemoryPluginStatePersistence::new())).unwrap(),
    );
    let catalog = Arc::new(KernelCatalogProvider::new(kernel.clone()));
    let composition = build_nomi_core_plugin_state(
        pool.clone(),
        root.to_path_buf(),
        owner,
        kernel.clone(),
        catalog.clone(),
        base,
        test_runtime_authority().await,
        BTreeSet::new(),
        None,
    )
    .await
    .unwrap();
    (kernel, catalog, composition)
}

async fn install(
    state: &PluginRouterState,
    root: &Path,
    owner: &str,
    kind: &str,
) -> Result<PluginDetailDto, PluginServiceError> {
    let (artifact, main) = package(kind);
    install_artifact(state, root, owner, kind, artifact, main).await
}

pub(super) async fn install_artifact(
    state: &PluginRouterState,
    root: &Path,
    owner: &str,
    kind: &str,
    artifact: PluginPackageArtifactV1,
    main: Vec<u8>,
) -> Result<PluginDetailDto, PluginServiceError> {
    let source = root.join(format!("incoming-{kind}"));
    write_package(&source, &artifact, &main);
    let project = state
        .service
        .import_prebuilt(
            owner,
            ImportPluginRequest {
                expected_library_revision: state
                    .service
                    .list_library(owner)
                    .await
                    .unwrap()
                    .library_revision,
                import_kind: PluginImportKindDto::PrebuiltArtifact,
                source_path: source.display().to_string(),
                expected_bundle_or_artifact_digest: artifact.artifact_digest.as_ref().into(),
                target_project_id: None,
                expected_project_revision: None,
            },
        )
        .await
        .unwrap();
    let candidate = project.ready.as_ref().unwrap().candidate.clone();
    state
        .service
        .test_candidate(
            owner,
            TestPluginCandidateRequest {
                project_id: project.summary.project_id.clone(),
                expected_project_revision: project.summary.project_revision,
                expected_build_generation: project.summary.build_generation,
                candidate_id: candidate.candidate_id.clone(),
                expected_candidate_digest: candidate.candidate_digest.clone(),
                expected_config_revision: 0,
                expected_credential_bindings_revision: 0,
                resolved_test_input_digest: "9".repeat(64),
            },
        )
        .await
        .unwrap();
    state
        .service
        .apply_candidate(
            owner,
            ApplyPluginCandidateRequest {
                project_id: project.summary.project_id,
                expected_project_revision: project.summary.project_revision,
                expected_build_generation: project.summary.build_generation,
                candidate_id: candidate.candidate_id,
                expected_candidate_digest: candidate.candidate_digest,
                target: ApplyPluginTargetDto::InitialInstall {
                    expected_library_revision: state
                        .service
                        .list_library(owner)
                        .await
                        .unwrap()
                        .library_revision,
                },
                allow_breaking: false,
                acknowledge_test_warning: true,
            },
        )
        .await
}

#[tokio::test]
async fn restore_batches_cross_package_provider_before_contract_and_isolates_bad_provider() {
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let (kernel, _, first) = compose(database.pool(), root.path(), &owner).await;
    // A committed-but-unreconciled install allocates the earlier Mount. No fake
    // UUID, hand-edited DB identity or ordering assumption is used in recovery.
    assert!(
        install(&first.router, root.path(), &owner, "provider")
            .await
            .is_err()
    );
    let library = first.router.service.list_library(&owner).await.unwrap();
    assert_eq!(library.plugins.len(), 1);
    let provider = library.plugins[0].clone();
    let contract = install(&first.router, root.path(), &owner, "contract")
        .await
        .unwrap();
    assert!(
        kernel
            .snapshot()
            .unwrap()
            .role_provider(&ROLE.into(), &provider.mount_id.clone().into())
            .is_some(),
        "installing the prerequisite must recover its Provider without a manual retry"
    );
    assert!(
        provider.mount_id < contract.summary.mount_id,
        "regression must exercise Provider-before-contract Mount order"
    );
    install(&first.router, root.path(), &owner, "independent")
        .await
        .unwrap();
    assert!(
        install(&first.router, root.path(), &owner, "broken")
            .await
            .is_err()
    );
    let before = kernel.snapshot().unwrap();
    let exact = before
        .role_provider(&ROLE.into(), &provider.mount_id.clone().into())
        .unwrap()
        .provider
        .clone();
    drop(first);

    let (restored, catalog, composition) = compose(database.pool(), root.path(), &owner).await;
    let snapshot = restored.snapshot().unwrap();
    assert_eq!(
        snapshot.generation, 1,
        "only the complete recovered batch may be published"
    );
    assert_eq!(
        snapshot
            .role_provider(&ROLE.into(), &provider.mount_id.clone().into())
            .unwrap()
            .provider,
        exact
    );
    assert!(
        snapshot
            .capability(&"test.nomicore.independent.context".into())
            .is_some()
    );
    assert!(
        snapshot
            .capability(&"test.nomicore.broken.context".into())
            .is_none()
    );
    assert_eq!(snapshot.role_providers.len(), 1);
    let api = catalog.snapshot().unwrap().as_api().unwrap();
    let role = api
        .roles
        .iter()
        .find(|r| r.role.key.role_id == ROLE)
        .unwrap();
    assert_eq!(role.providers.len(), 1);
    assert_eq!(
        role.providers[0].selection.provider_mount_id,
        provider.mount_id
    );
    assert_installed_context_reaches_nomi_prompt(
        restored.clone(),
        composition.schema_resolver.clone(),
        &owner,
        FACADE,
        Some(role.providers[0].selection.clone()),
        catalog.clone(),
        None,
        None,
    )
    .await;

    let disabled = composition
        .router
        .service
        .set_enabled(
            &owner,
            SetPluginEnabledRequest {
                mount_id: contract.summary.mount_id.clone(),
                expected_mount_revision: contract.summary.mount_revision,
                expected_current_target_digest: contract
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
    let withdrawn = restored.snapshot().unwrap();
    assert_eq!(withdrawn.generation, snapshot.generation + 1);
    assert!(withdrawn.capability(&FACADE.into()).is_none());
    assert!(
        withdrawn
            .capability(&"test.nomicore.provider.context".into())
            .is_none()
    );
    assert!(withdrawn.role_providers.is_empty());
    assert!(
        withdrawn
            .capability(&"test.nomicore.independent.context".into())
            .is_some()
    );
    assert!(
        catalog
            .snapshot()
            .unwrap()
            .as_api()
            .unwrap()
            .roles
            .is_empty()
    );
    let retained = composition
        .router
        .service
        .get_mount(&owner, &provider.mount_id)
        .await
        .unwrap();
    assert!(
        retained.summary.lifecycle == nomifun_api_types::PluginLifecycleDto::Enabled,
        "dependency withdrawal must not disable the installed Provider"
    );
    assert_eq!(retained.summary.mount_revision, provider.mount_revision);
    assert_eq!(retained.summary.current, provider.current);

    composition
        .router
        .service
        .set_enabled(
            &owner,
            SetPluginEnabledRequest {
                mount_id: disabled.summary.mount_id,
                expected_mount_revision: disabled.summary.mount_revision,
                expected_current_target_digest: disabled.summary.current.unwrap().artifact_digest,
                enabled: true,
            },
        )
        .await
        .unwrap();
    let enabled = restored.snapshot().unwrap();
    assert_eq!(enabled.generation, withdrawn.generation + 1);
    assert_eq!(
        enabled
            .role_provider(&ROLE.into(), &provider.mount_id.clone().into())
            .unwrap()
            .provider,
        exact
    );
    assert!(
        enabled
            .capability(&"test.nomicore.broken.context".into())
            .is_none()
    );
    let api = catalog.snapshot().unwrap().as_api().unwrap();
    let selection = api
        .roles
        .iter()
        .find(|r| r.role.key.role_id == ROLE)
        .unwrap()
        .providers[0]
        .selection
        .clone();
    assert_installed_context_reaches_nomi_prompt(
        restored,
        composition.schema_resolver.clone(),
        &owner,
        FACADE,
        Some(selection),
        catalog,
        None,
        None,
    )
    .await;
}
