use super::*;
use crate::{
    KernelNomiPluginToolSession, NomiPlatformBuiltinToolAdmission,
    NomiPlatformBuiltinToolSchemaResolver, NomiPluginToolSchemaResolver,
};
use nomifun_agent_contracts::*;
use nomifun_agent_kernel::*;
use std::collections::{BTreeMap, BTreeSet};

struct UnusedSchemas;
#[async_trait]
impl NomiPluginToolSchemaResolver for UnusedSchemas {
    async fn resolve(
        &self,
        _: &ResolvedCapability,
        _: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        panic!("native Tool must not resolve a Plugin schema")
    }
}
#[async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for UnusedSchemas {
    async fn resolve(
        &self,
        _: &ResolvedCapability,
        _: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        panic!("native Tool must not resolve a hosted schema")
    }
}

fn registration(binding: &LocalSearchBinding) -> PluginRegistration {
    let mut registration = nomifun_agent_domain_wave1::registrations()
        .unwrap()
        .into_iter()
        .find(|registration| {
            registration.metadata.manifest.payload.package_id.as_ref()
                == nomifun_agent_domain_wave1::LOCAL_WEBSEARCH_PACKAGE_ID
        })
        .unwrap();
    let mut manifest = registration.metadata.manifest.payload.clone();
    manifest.contributions.capabilities[0]
        .config_schema
        .0
        .as_object_mut()
        .unwrap()
        .insert(
            BINDING_ANNOTATION.into(),
            serde_json::to_value(binding).unwrap(),
        );
    registration.metadata.manifest = ArtifactEnvelope::new(manifest).unwrap();
    registration
}

fn compile(registry: &MaterializedRegistry, owner: &PrincipalRef) -> CompiledSnapshot {
    let capability = registry.capability(&CapabilityId::from(TOOL_NAME)).unwrap();
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: "0190f5fe-7c00-7a00-8000-000000000004".into(),
            revision: 1,
            revision_digest: "".into(),
        },
        payload: AgentPresetRevisionPayload {
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: TOOL_NAME.into(),
                    version: "1.0.0".into(),
                },
                action_allowlist: BTreeSet::from(["nomi_local_websearch.invoke".into()]),
            }],
            skill_bindings: vec![],
            system_role_provider_overrides: BTreeMap::new(),
            persona: String::new(),
            instructions: String::new(),
            starter_prompts: vec![],
        },
        contribution_locks: vec![capability.contribution_lock.clone()],
        created_by: owner.principal_id.clone().into(),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    AgentPresetCompiler::compile(
        registry,
        &CompilerEnvironment {
            resolver_version: "1.0.0".into(),
            required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: "runtime".into(),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: "schema".into(),
            target_contribution_manifest_digest: registry.registry_digest.clone(),
            host_target: "x86_64-pc-windows-msvc".into(),
            host_surface: "desktop".into(),
            availability_evidence_revision: "binding-test".into(),
        },
        CompileRequest {
            plugin_product_capabilities: vec![],
            revision,
            principal: owner.clone(),
            scene: "chat".into(),
            surface: "desktop".into(),
            audience: "owner".into(),
            created_at_ms: 2,
            resolver_run_id: "binding-test".into(),
        },
    )
    .unwrap()
}

async fn session(
    kernel: Arc<KernelRegistry>,
    compiled: CompiledSnapshot,
    owner: PrincipalRef,
) -> Result<crate::NomiPluginToolSession, crate::NomiPluginToolError> {
    let registry = kernel.snapshot().unwrap();
    let admission = Arc::new(
        NomiPlatformBuiltinToolAdmission::from_registry(
            &registry,
            BTreeSet::new(),
            BTreeSet::from([TOOL_NAME.into()]),
            Arc::new(UnusedSchemas),
        )
        .unwrap(),
    );
    KernelNomiPluginToolSession::materialize_with_platform_builtins(
        kernel,
        Arc::new(compiled),
        owner,
        "0190f5fe-7c00-7a00-8000-000000000002".into(),
        "session:0190f5fe-7c00-7a00-8000-000000000002".into(),
        Arc::new(UnusedSchemas),
        admission,
    )
    .await
}

#[tokio::test]
async fn session_uses_frozen_binding_and_rejects_registry_drift() {
    let binding = LocalSearchBinding {
        schema_version: 1,
        runtime_build_digest: "a".repeat(64),
        adapter_digest: "b".repeat(64),
        browser_binary_digest: "c".repeat(64),
        browser_product: "Chrome/fixture".into(),
    };
    let kernel = Arc::new(
        KernelRegistry::new(
            MaterializationPolicy::stable("1.0.0"),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let live = kernel.replace_all(vec![registration(&binding)]).unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
    };
    let frozen = compile(&live, &owner);
    let current = session(kernel.clone(), frozen.clone(), owner.clone())
        .await
        .unwrap();
    assert_eq!(current.local_search_binding(), Some(&binding));
    let mut changed = binding.clone();
    changed.adapter_digest = "d".repeat(64);
    kernel.replace_all(vec![registration(&changed)]).unwrap();
    assert!(
        session(kernel.clone(), frozen, owner.clone())
            .await
            .is_err(),
        "old Snapshot cannot silently adopt a new adapter"
    );
    let updated = compile(&kernel.snapshot().unwrap(), &owner);
    assert_eq!(
        session(kernel, updated, owner)
            .await
            .unwrap()
            .local_search_binding(),
        Some(&changed)
    );
}

#[test]
fn local_runtime_binding_does_not_change_vendor_search_provenance() {
    let binding = LocalSearchBinding {
        schema_version: 1,
        runtime_build_digest: "a".repeat(64),
        adapter_digest: "b".repeat(64),
        browser_binary_digest: "c".repeat(64),
        browser_product: "Chrome/fixture".into(),
    };
    let kernel = KernelRegistry::new(
        MaterializationPolicy::stable("1.0.0"),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap();
    let first = kernel
        .replace_all(nomifun_agent_domain_wave1::registrations().unwrap())
        .unwrap();
    let mut registrations = nomifun_agent_domain_wave1::registrations().unwrap();
    registrations.retain(|registration| {
        registration.metadata.manifest.payload.package_id.as_ref()
            != nomifun_agent_domain_wave1::LOCAL_WEBSEARCH_PACKAGE_ID
    });
    registrations.push(registration(&binding));
    let next = kernel.replace_all(registrations).unwrap();
    let id = CapabilityId::from("web.search");
    assert_eq!(
        first.capability(&id).unwrap().contribution_lock,
        next.capability(&id).unwrap().contribution_lock
    );
    assert_eq!(
        first.capability(&id).unwrap().target_artifact_digest,
        next.capability(&id).unwrap().target_artifact_digest
    );
}
