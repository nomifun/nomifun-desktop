//! Real installed JS Providers, SQLite authoring and the production Nomi
//! Snapshot admission seam. No alternate compiler or Session owner.
use super::restore_tests::{compose, install_artifact};
use super::tests::package_artifact;
use super::*;
use crate::router::nomi_core_control_plane::NomiCoreControlPlaneStore;
use crate::router::nomi_core_session::compile_nomi_plugin_snapshot;
use nomifun_agent_contracts::{
    AgentBindingValue, CapabilityConsumption, CapabilityRef,
    ExactRoleContractRef, PresetRevisionRef, PrincipalRef, RoleContractKey, RoleContractManifest,
    RoleMemberContract, RoleMemberRequirement, RoleProviderContribution,
    RoleProviderMemberContribution, RuntimeProfileKind, StrictJsonValue,
};
use nomifun_agent_control_plane::{
    AgentControlPlane, CatalogProvider, ControlPlaneStore, OfficialTemplateCatalog,
    PresetRevisionCompiler,
};
use nomifun_agent_kernel::{
    CapabilityInvocationRequest, CompiledSnapshot, CompilerEnvironment, KernelError, SessionCapabilityState,
};
use nomifun_api_types::{CreateAgentPresetRequest, SaveAgentPresetRevisionRequest};
use serde_json::json;

const ROLE: &str = "test.graph.contract.role";
const FACADE: &str = "test.graph.contract.tool";
const ACTION: &str = "test.nomicore.plugin.echo.invoke";

fn package(kind: &str) -> (nomifun_agent_contracts::PluginPackageArtifactV1, Vec<u8>) {
    let prefix = format!("test.graph.{kind}");
    let main = if kind == "contract" {
        b"export async function activate() { return { capabilities: {} }; }".to_vec()
    } else {
        format!(
            r#"export async function activate() {{ return {{ capabilities: {{
          "{prefix}.root.contribution": {{ async invoke({{ input, dependencies }}) {{
            return await dependencies.invoke({{ capabilityId: "{prefix}.child",
              actionId: "{ACTION}", callKey: "child", input }});
          }} }},
          "{prefix}.child.contribution": {{ async invoke({{ input }}) {{
            return {{ provider: "{kind}", message: input.message }};
          }} }},
          "{prefix}.context.contribution": {{ async contributeContext() {{
            throw new Error("private Context must not be implicitly consumed");
          }} }}
        }} }}; }}"#
        )
        .into_bytes()
    };
    let original = package_artifact(&main);
    let mut manifest = original.manifest.payload.clone();
    let mut facade = manifest.package.contributions.capabilities[0].clone();
    facade.id = FACADE.into();
    facade.contribution_id = format!("{FACADE}.contribution").into();
    facade.package.id = "test.graph.contract".into();
    let contract = RoleContractManifest {
        key: RoleContractKey {
            role_id: ROLE.into(),
            contract_version: "1.0.0".into(),
        },
        members: vec![RoleMemberContract {
            capability: CapabilityRef {
                id: facade.id.clone(),
                version: facade.version.clone(),
            },
            capability_manifest_digest: digest_payload(&facade).unwrap(),
            requirement: RoleMemberRequirement::Required,
        }],
        serialized_target_resource_kind: None,
    };
    let exact = ExactRoleContractRef {
        key: contract.key.clone(),
        contract_digest: digest_payload(&contract).unwrap(),
    };
    manifest.package.package_id = prefix.clone().into();
    if kind == "contract" {
        manifest.package.contributions.capabilities = vec![facade];
        manifest.package.contributions.role_contracts = vec![contract];
        manifest.package.contributions.role_providers.clear();
    } else {
        let mut child = facade.clone();
        child.id = format!("{prefix}.child").into();
        child.contribution_id = format!("{prefix}.child.contribution").into();
        child.package.id = prefix.clone().into();
        let mut context = manifest.package.contributions.capabilities[1].clone();
        context.id = format!("{prefix}.context").into();
        context.contribution_id = format!("{prefix}.context.contribution").into();
        context.package.id = prefix.clone().into();
        context.conflicts.clear();
        let mut root = child.clone();
        root.id = format!("{prefix}.root").into();
        root.contribution_id = format!("{prefix}.root.contribution").into();
        root.requires = [&child, &context]
            .into_iter()
            .map(|c| CapabilityRef {
                id: c.id.clone(),
                version: c.version.clone(),
            })
            .collect();
        manifest.package.contributions.role_contracts.clear();
        manifest.package.contributions.role_providers = vec![RoleProviderContribution {
            role: exact,
            display: facade.display.clone(),
            members: BTreeMap::from([(
                FACADE.into(),
                RoleProviderMemberContribution {
                    implementation: Some(CapabilityRef {
                        id: root.id.clone(),
                        version: root.version.clone(),
                    }),
                    supported_platforms: root.supported_platforms.clone(),
                    required_resource_kinds: BTreeSet::new(),
                },
            )]),
        }];
        manifest.package.contributions.capabilities = vec![root, child, context];
    }
    let artifact = nomifun_agent_contracts::PluginPackageArtifactV1::new(
        original.artifact_id,
        manifest,
        original.files,
    )
    .unwrap();
    (artifact, main)
}

async fn invoke(
    kernel: &KernelRegistry,
    snapshot: &CompiledSnapshot,
    owner: &PrincipalRef,
    capability: &str,
) -> Result<StrictJsonValue, KernelError> {
    let active = SessionCapabilityState::new(snapshot).snapshot().unwrap();
    kernel
        .invoke(
            snapshot,
            &active,
            CapabilityInvocationRequest {
                principal: owner.clone(),
                session_owner: owner.clone(),
                agent_session_id: "graph-session".into(),
                operation_id: "graph-operation".into(),
                idempotency_key: "graph-key".into(),
                correlation_id: "graph-correlation".into(),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: capability.into(),
                action_id: ACTION.into(),
                resource_binding_ids: BTreeSet::new(),
                state_scope_key: "session:graph-session".into(),
                input: StrictJsonValue(json!({"message":"hello"})),
            },
        )
        .await
}

#[tokio::test]
async fn installed_heterogeneous_providers_freeze_private_graphs_across_save_open_and_withdrawal() {
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let (kernel, catalog, composition) = compose(database.pool(), root.path(), &owner).await;
    let mut installs = Vec::new();
    for kind in ["contract", "a", "b"] {
        let (artifact, main) = package(kind);
        installs.push(
            install_artifact(
                &composition.router,
                root.path(),
                &owner,
                kind,
                artifact,
                main,
            )
            .await
            .unwrap(),
        );
    }
    let candidates = catalog.snapshot().unwrap().as_api().unwrap();
    let role = candidates
        .roles
        .iter()
        .find(|r| r.role.key.role_id == ROLE)
        .unwrap();
    let choices = ["test.graph.a", "test.graph.b"].map(|id| {
        role.providers
            .iter()
            .find(|provider| provider.source_package.id == id)
            .unwrap()
            .selection
            .clone()
    });
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
        availability_evidence_revision: "graph-flow".into(),
    };
    let store = Arc::new(NomiCoreControlPlaneStore::new(database.pool().clone()));
    let templates = OfficialTemplateCatalog::load().unwrap();
    let control = AgentControlPlane::new(
        store.clone(),
        catalog,
        templates.clone(),
        PresetRevisionCompiler::new()
            .with_canonical_registry(kernel.clone(), environment.clone()),
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
                display_name: "Heterogeneous providers".into(),
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
            id: FACADE.into(),
            version: "1.0.0".into(),
        },
        action_allowlist: BTreeSet::new(),
    }];
    let preset = draft.preset_id.clone();
    let mut saved_plans = Vec::new();
    for (kind, choice) in ["a", "b"].into_iter().zip(choices) {
        draft
            .document
            .system_role_provider_overrides
            .insert(ROLE.into(), choice);
        let saved = control
            .save_revision(
                &actor,
                &preset,
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
            serde_json::from_value(serde_json::to_value(&saved.revision.reference).unwrap())
                .unwrap();
        let frozen = store.get_snapshot(&reference).await.unwrap().unwrap();
        frozen.validate().unwrap();
        assert_eq!(frozen.content.enabled_capabilities.len(), 3);
        assert_eq!(
            frozen
                .content
                .contributions()
                .map(|c| c.capability.id.as_ref())
                .collect::<Vec<_>>(),
            vec![FACADE]
        );
        for dependency in frozen
            .content
            .enabled_capabilities
            .iter()
            .filter(|c| c.capability.id.as_ref() != FACADE)
        {
            assert_eq!(dependency.consumption, CapabilityConsumption::Dependency);
            assert!(
                dependency
                    .capability
                    .id
                    .as_ref()
                    .starts_with(&format!("test.graph.{kind}."))
            );
        }
        let revision = store.get_revision(&reference).await.unwrap().unwrap();
        let binding = AgentBindingValue {
            preset_revision_ref: reference.clone(),
            resolved_snapshot_ref: frozen.snapshot_ref.clone(),
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        };
        let compiled = compile_nomi_plugin_snapshot(
            &kernel,
            &environment,
            binding.clone(),
            revision.clone(),
            frozen.clone(),
            &principal,
        )
        .unwrap();
        let session = nomifun_ai_agent::KernelNomiPluginToolSession::materialize(
            kernel.clone(),
            Arc::new(compiled.clone()),
            principal.clone(),
            "graph-session".into(),
            "session:graph-session".into(),
            composition.schema_resolver.clone(),
        )
        .await
        .unwrap();
        assert_eq!(session.actions().len(), 1);
        assert_eq!(session.actions()[0].capability_id().as_ref(), FACADE);
        assert!(session.initial_context_contributions().is_empty());
        assert_eq!(
            invoke(&kernel, &compiled, &principal, FACADE)
                .await
                .unwrap()
                .0,
            json!({"provider":kind,"message":"hello"})
        );
        assert!(matches!(
            invoke(
                &kernel,
                &compiled,
                &principal,
                &format!("test.graph.{kind}.child")
            )
            .await,
            Err(KernelError::CapabilityNotInPreset { .. })
        ));
        let clean = control
            .save_revision(
                &actor,
                &preset,
                SaveAgentPresetRevisionRequest {
                    expected_current_revision: draft.current_revision.clone(),
                    draft: draft.clone(),
                    reason: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(clean.revision.reference, saved.revision.reference);
        saved_plans.push((reference, revision, frozen, binding, compiled));
    }
    let (_, old_revision, old_snapshot, old_binding, old_compiled) = &saved_plans[0];
    let reopened = compile_nomi_plugin_snapshot(
        &kernel,
        &environment,
        old_binding.clone(),
        old_revision.clone(),
        old_snapshot.clone(),
        &principal,
    )
    .unwrap();
    assert_eq!(
        invoke(&kernel, &reopened, &principal, FACADE)
            .await
            .unwrap()
            .0["provider"],
        "a"
    );
    for (reference, _, frozen, _, _) in &saved_plans {
        assert_eq!(
            &store.get_snapshot(reference).await.unwrap().unwrap(),
            frozen
        );
    }
    // A valid legacy-shaped envelope is not silently reinterpreted as a new
    // graph: the current open seam rejects its semantic drift without writes.
    let mut legacy = old_snapshot.clone();
    for record in &mut legacy.content.enabled_capabilities {
        record.consumption = CapabilityConsumption::Contribution;
        record.dependency_refs.clear();
    }
    legacy.snapshot_ref.snapshot_digest = digest_payload(&legacy.content).unwrap();
    legacy.validate().unwrap();
    let mut legacy_binding = old_binding.clone();
    legacy_binding.resolved_snapshot_ref = legacy.snapshot_ref.clone();
    let error = compile_nomi_plugin_snapshot(
        &kernel,
        &environment,
        legacy_binding,
        old_revision.clone(),
        legacy,
        &principal,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("differs from the persisted"),
        "{error}"
    );
    let a = &installs[1];
    composition
        .router
        .service
        .set_enabled(
            &owner,
            SetPluginEnabledRequest {
                mount_id: a.summary.mount_id.clone(),
                expected_mount_revision: a.summary.mount_revision,
                expected_current_target_digest: a
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
    assert!(
        compile_nomi_plugin_snapshot(
            &kernel,
            &environment,
            old_binding.clone(),
            old_revision.clone(),
            old_snapshot.clone(),
            &principal
        )
        .is_err()
    );
    assert!(
        invoke(&kernel, old_compiled, &principal, FACADE)
            .await
            .is_err()
    );
    assert_eq!(
        invoke(&kernel, &saved_plans[1].4, &principal, FACADE)
            .await
            .unwrap()
            .0["provider"],
        "b"
    );
    assert_eq!(
        store
            .get_snapshot(&saved_plans[0].0)
            .await
            .unwrap()
            .unwrap(),
        *old_snapshot
    );
}
