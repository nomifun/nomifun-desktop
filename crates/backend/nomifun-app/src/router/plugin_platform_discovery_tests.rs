//! Installed JS discovery through the product Catalog, saved revisions and
//! production Nomi snapshot adapter. No substitute compiler or state owner.
use super::restore_tests::{compose_with_base, install_artifact};
use super::*;
use crate::router::nomi_core_control_plane::NomiCoreControlPlaneStore;
use crate::router::nomi_core_session::compile_nomi_plugin_snapshot;
use nomifun_agent_contracts::{
    AgentBindingValue, CapabilityRef, PackageContributions, PluginPackageArtifactV1,
    PresetRevisionRef, PrincipalRef, RoleProviderMemberContribution, RuntimeProfileKind,
    StrictJsonValue,
};
use nomifun_agent_control_plane::{
    AgentControlPlane, CatalogProvider, ControlPlaneStore, OfficialTemplateCatalog,
    PresetRevisionCompiler,
};
use nomifun_agent_kernel::{
    CapabilityInvocationRequest, CompilerEnvironment, SessionCapabilityState,
};
use nomifun_ai_agent::tool_discovery::{self as discovery, CAPABILITY_ID, ROLE_ID};
use nomifun_api_types::{CreateAgentPresetRequest, SaveAgentPresetRevisionRequest};
use serde_json::json;

#[tokio::test]
async fn installed_discovery_is_selectable_and_saved_provider_is_used_without_fallback() {
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let builtin = crate::router::nomi_core_tool_discovery::registration().unwrap();
    let builtin_manifest = builtin.metadata.manifest.payload.clone();
    let builtin_mount = builtin.metadata.mount_id.clone();
    let main =
        include_bytes!("../../../nomifun-ai-agent/tests/fixtures/discovery/main.mjs").to_vec();
    let original = super::tests::package_artifact(&main);
    let mut manifest = original.manifest.payload.clone();
    let mut implementation = builtin_manifest.contributions.capabilities[0].clone();
    implementation.id = "example.discovery".into();
    implementation.contribution_id = "capability:example.discovery".into();
    implementation.package.id = manifest.package.package_id.clone();
    let mut provider = builtin_manifest.contributions.role_providers[0].clone();
    provider.display.name = "User reverse discovery".into();
    provider.members.insert(
        CAPABILITY_ID.into(),
        RoleProviderMemberContribution {
            implementation: Some(CapabilityRef {
                id: implementation.id.clone(),
                version: implementation.version.clone(),
            }),
            supported_platforms: implementation.supported_platforms.clone(),
            required_resource_kinds: BTreeSet::new(),
        },
    );
    manifest.package.contributions = PackageContributions {
        capabilities: vec![implementation],
        role_providers: vec![provider],
        ..Default::default()
    };
    manifest.schemas = discovery::schemas();
    let artifact =
        PluginPackageArtifactV1::new(original.artifact_id, manifest, original.files).unwrap();
    let (kernel, catalog, composition) =
        compose_with_base(database.pool(), root.path(), &owner, vec![builtin]).await;
    let installed = install_artifact(
        &composition.router,
        root.path(),
        &owner,
        "discovery",
        artifact,
        main,
    )
    .await
    .unwrap();
    let candidates = catalog.snapshot().unwrap().as_api().unwrap();
    let role = candidates
        .roles
        .iter()
        .find(|role| role.role.key.role_id == ROLE_ID)
        .unwrap();
    assert_eq!(role.providers.len(), 2);
    let choices = [builtin_mount.as_ref(), installed.summary.mount_id.as_str()].map(|mount| {
        role.providers
            .iter()
            .find(|provider| provider.selection.provider_mount_id == mount)
            .unwrap()
            .selection
            .clone()
    });
    assert!(
        candidates
            .capabilities
            .iter()
            .any(|c| c.capability.id == "example.discovery" && c.unavailable_code.is_none())
    );
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
        availability_evidence_revision: "discovery-flow".into(),
    };
    let store = Arc::new(NomiCoreControlPlaneStore::new(database.pool().clone()));
    let templates = OfficialTemplateCatalog::load().unwrap();
    let control = AgentControlPlane::new(
        store.clone(),
        catalog,
        templates.clone(),
        PresetRevisionCompiler::new()
            .with_canonical_registry(kernel.clone(), environment.clone())
            .with_consumer_validator(crate::router::nomi_core_tool_discovery::validate_snapshot),
    );
    let actor = owner.clone().into();
    let principal = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: owner.clone(),
    };
    let mut draft = control
        .create_preset(
            &actor,
            CreateAgentPresetRequest {
                display_name: "Selectable discovery".into(),
                description: None,
                fork_from_revision: None,
                document: None,
            },
        )
        .await
        .unwrap()
        .draft;
    draft.document.enabled_capabilities = vec![nomifun_api_types::CapabilitySelectionDto {
        capability: nomifun_api_types::ExactCatalogRefDto {
            id: CAPABILITY_ID.into(),
            version: "1.0.0".into(),
        },
        action_allowlist: BTreeSet::from([discovery::ACTION_ID.to_owned()]),
    }];
    let mut frozen_plans = Vec::new();
    for choice in choices {
        draft
            .document
            .system_role_provider_overrides
            .insert(ROLE_ID.into(), choice);
        let saved = control
            .save_revision(
                &actor,
                &draft.preset_id.clone(),
                SaveAgentPresetRevisionRequest {
                    expected_current_revision: draft.current_revision.clone(),
                    draft: draft.clone(),
                    reason: None,
                },
            )
            .await
            .unwrap();
        draft.current_revision = Some(saved.revision.reference.clone());
        let reference: PresetRevisionRef =
            serde_json::from_value(serde_json::to_value(saved.revision.reference).unwrap())
                .unwrap();
        let frozen = store.get_snapshot(&reference).await.unwrap().unwrap();
        let revision = store.get_revision(&reference).await.unwrap().unwrap();
        let binding = AgentBindingValue {
            preset_revision_ref: reference,
            resolved_snapshot_ref: frozen.snapshot_ref.clone(),
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        };
        frozen_plans.push((binding, revision, frozen));
    }
    let mut conflicting = draft.clone();
    conflicting
        .document
        .enabled_capabilities
        .push(nomifun_api_types::CapabilitySelectionDto {
            capability: nomifun_api_types::ExactCatalogRefDto {
                id: "example.discovery".into(),
                version: "1.0.0".into(),
            },
            action_allowlist: BTreeSet::from([discovery::ACTION_ID.to_owned()]),
        });
    let error = control
        .save_revision(
            &actor,
            &draft.preset_id,
            SaveAgentPresetRevisionRequest {
                expected_current_revision: draft.current_revision.clone(),
                draft: conflicting,
                reason: None,
            },
        )
        .await
        .unwrap_err();
    assert!(
        error
            .details()
            .unwrap()
            .to_string()
            .contains("multiple discovery policies")
    );
    let unchanged = control
        .save_revision(
            &actor,
            &draft.preset_id,
            SaveAgentPresetRevisionRequest {
                expected_current_revision: draft.current_revision.clone(),
                draft: draft.clone(),
                reason: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(Some(unchanged.revision.reference), draft.current_revision);
    // Reopen both after the second save: the first must remain on the builtin.
    for ((binding, revision, frozen), expected) in frozen_plans.into_iter().zip(["Alpha", "Beta"]) {
        let compiled = Arc::new(
            compile_nomi_plugin_snapshot(
                &kernel,
                &environment,
                binding,
                revision,
                frozen,
                &principal,
            )
            .unwrap(),
        );
        let session = nomifun_ai_agent::KernelNomiPluginToolSession::materialize(
            kernel.clone(),
            compiled.clone(),
            principal.clone(),
            "discovery-session".into(),
            "session:discovery-session".into(),
            composition.schema_resolver.clone(),
        )
        .await
        .unwrap();
        assert!(session.actions().is_empty());
        assert_eq!(
            session.provider_names_for(CAPABILITY_ID, false),
            vec!["ToolSearch"]
        );
        let active = SessionCapabilityState::new(&compiled).snapshot().unwrap();
        let key = uuid::Uuid::now_v7().to_string();
        let result = kernel.invoke_shared(compiled.clone(), &active, CapabilityInvocationRequest {
            principal: principal.clone(), session_owner: principal.clone(), agent_session_id: "discovery-session".into(),
            operation_id: key.clone().into(), idempotency_key: key.clone().into(), correlation_id: key.into(),
            resolved_snapshot_ref: compiled.snapshot_ref().clone(), active_set_generation: active.generation,
            capability_id: CAPABILITY_ID.into(), action_id: discovery::ACTION_ID.into(), resource_binding_ids: BTreeSet::new(), state_scope_key: "session:discovery-session".into(),
            input: StrictJsonValue(json!({"query":"shared","limit":5,"candidates":[
                {"name":"Alpha","description":"shared","aliases":[]},{"name":"Beta","description":"shared","aliases":[]}
            ]})),
        }).await.unwrap();
        assert_eq!(result.0["names"][0], expected);
    }
}
