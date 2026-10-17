//! Projection of selected, exact Snapshot contributions into Nomi tools.
//! No global catalog discovery and no executable/runtime loading occurs here.
use nomifun_agent_contracts::{
    ActionId, CapabilityId, CapabilityManifest, ContributionSourceKind, ToolPresentationKind,
};
use nomifun_agent_kernel::{ActiveCapabilitySetSnapshot, CompiledSnapshot, MaterializedRegistry};
use nomifun_ai_agent::{
    NomiPlatformBuiltinToolSchemaResolver, NomiPluginToolSchemaResolver,
};
use nomifun_chat_model_broker::ChatToolDefinition;
use nomifun_agent_runtime::{
    AgentToolExposure, AgentToolPlan, compile_agent_tool_plan,
};
use nomifun_common::AppError;
use nomifun_agent_contracts::tool_presentation::{
    namespaced_tool_name, platform_tool_name, relative_action,
};
use nomifun_agent_contracts::tool_model_guide::{ToolModelGuide, tool_model_guide};

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Nomi tool surface: {value}"))
}

pub(super) async fn compile(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
    plugin_schemas: &dyn NomiPluginToolSchemaResolver,
    platform_builtin_schemas: &dyn NomiPlatformBuiltinToolSchemaResolver,
    host_dynamic_capability_ids: &std::collections::BTreeSet<CapabilityId>,
) -> Result<AgentToolPlan, AppError> {
    if snapshot.registry_generation != registry.generation
        || snapshot.registry_digest != registry.registry_digest
    {
        return Err(error("registry changed after Snapshot compilation"));
    }
    let allowed_platform_actions = snapshot
        .content()
        .enabled_capabilities
        .iter()
        .filter(|selected| {
            selected.contribution_lock.source_kind == ContributionSourceKind::PlatformBuiltin
        })
        .filter_map(|selected| {
            snapshot.policy(&selected.capability.id).map(|policy| {
                (
                    selected.capability.id.clone(),
                    policy.allowed_actions.clone(),
                )
            })
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut exposures = Vec::new();
    for selected in snapshot.content().enabled_capabilities.iter().filter(|selected| {
        active.active.contains(&selected.capability.id)
            && selected.contribution_lock.source_kind == ContributionSourceKind::PlatformBuiltin
            && nomifun_agent_domain_wave2::WORKSPACE_EXECUTION_CAPABILITY_IDS.contains(&selected.capability.id.as_ref())
    }) {
        let capability_id = &selected.capability.id;
        let capability = registry.capability(capability_id).ok_or_else(|| error("tool is not materialized"))?;
        let allowed = allowed_platform_actions.get(capability_id)
            .ok_or_else(|| error("selected PlatformBuiltin has no compiled Action policy"))?;
        for action in capability.manifest.contributions.actions.iter().filter(|action| {
            action.presentation == ToolPresentationKind::FunctionTool && allowed.contains(&action.action_id)
        }) {
            if !action_resources_bound(snapshot, capability_id, &action.action_id)? { continue; }
            let schema = nomifun_agent_domain_wave2::resolve_action_schema(capability_id.as_ref(), &action.input_schema)
                .map_err(error)?;
            exposures.push(standard_tool_exposure(capability_id, &action.action_id, &schema,
                std::env::consts::OS, tool_model_guide(capability_id.as_ref(), action.action_id.as_ref()))?);
        }
    }
    let mut exact_actions = exposures
        .iter()
        .map(|exposure| (exposure.capability_id.clone(), exposure.action_id.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    for selected in snapshot
        .content()
        .enabled_capabilities
        .iter()
        .filter(|selected| {
            active.active.contains(&selected.capability.id)
                && selected.contribution_lock.source_kind
                    == ContributionSourceKind::PlatformBuiltin
        })
    {
        // Host-dynamic Modules (currently Robot) are compiled from their exact
        // bound device later by the Engine session host. They have no static
        // schema owner and must never block a chat-only Session when unbound.
        if host_dynamic_capability_ids.contains(&selected.capability.id) {
            continue;
        }
        let capability = registry
            .capability(&selected.capability.id)
            .ok_or_else(|| error("selected PlatformBuiltin capability is unavailable"))?;
        let allowed = allowed_platform_actions
            .get(&selected.capability.id)
            .ok_or_else(|| error("selected PlatformBuiltin has no compiled Action policy"))?;
        for action in capability
            .manifest
            .contributions
            .actions
            .iter()
            .filter(|action| {
                action.presentation == ToolPresentationKind::FunctionTool
                    && allowed.contains(&action.action_id)
            })
        {
            if !action_resources_bound(snapshot, &selected.capability.id, &action.action_id)? {
                continue;
            }
            if !exact_actions.insert((
                selected.capability.id.clone(),
                action.action_id.clone(),
            )) {
                continue;
            }
            let schema = platform_builtin_schemas
                .resolve(selected, &action.input_schema)
                .await
                .map_err(error)?;
            let schema = super::engine_creation_tools::conversation_creation_schema(
                selected.capability.id.as_ref(),
                action.action_id.as_ref(),
                schema,
            );
            if !concrete_object_schema(&schema.0, true) {
                return Err(error(format!(
                    "{} action {} has no strict canonical tool schema",
                    selected.capability.id.as_ref(),
                    action.action_id.as_ref(),
                )));
            }
            exposures.push(AgentToolExposure {
                definition: ChatToolDefinition {
                    name: platform_tool_name(
                        selected.capability.id.as_ref(),
                        action.action_id.as_ref(),
                    ),
                    description: format!(
                        "{}: {} Action: {}.",
                        capability.manifest.display.name,
                        capability.manifest.display.description,
                        action.action_id.as_ref(),
                    ),
                    input_schema: schema,
                    deferred: false,
                },
                capability_id: selected.capability.id.clone(),
                action_id: action.action_id.clone(),
            });
        }
    }
    for selected in snapshot
        .content()
        .enabled_capabilities
        .iter()

    {
        if active.active.contains(&selected.capability.id)
            && selected.contribution_lock.source_kind == ContributionSourceKind::McpBinding
        {
            let mut resource_ready = false;
            for action in &selected.action_allowlist {
                resource_ready |= action_resources_bound(snapshot, &selected.capability.id, action)?;
            }
            if !resource_ready {
                continue;
            }
            let tool = super::nomi_core_mcp_catalog::frozen_tool(registry, selected)?;
            if !snapshot.content().mcp_tool_locks.contains(&tool.lock) {
                return Err(error(
                    "MCP tool descriptor differs from its Snapshot mapping",
                ));
            }
            let action = tool.action();
            let origin = tool.display_name.strip_suffix(&format!(" / {}", tool.remote_tool_name))
                .unwrap_or(&tool.display_name);
            exposures.push(AgentToolExposure {
                definition: ChatToolDefinition {
                    name: namespaced_tool_name("mcp", origin, &tool.remote_tool_name,
                        selected.capability.id.as_ref().as_bytes()),
                    description: format!(
                        "{}: {}. Remote MCP tool; executes only through the frozen platform grant.",
                        tool.display_name, tool.description
                    ),
                    input_schema: nomifun_agent_contracts::StrictJsonValue(tool.input_schema),
                    deferred: false,
                },
                capability_id: selected.capability.id.clone(),
                action_id: action,
            });
            continue;
        }
        if !active.active.contains(&selected.capability.id)
            || selected.contribution_lock.source_kind != ContributionSourceKind::AgentModule
        {
            continue;
        }
        let capability = registry
            .capability(&selected.capability.id)
            .ok_or_else(|| error("selected Plugin capability is unavailable"))?;
        let policy = snapshot
            .policy(&selected.capability.id)
            .ok_or_else(|| error("selected Plugin capability has no authority policy"))?;
        if capability.manifest.contributions.actions.is_empty() {
            return Err(error(format!(
                "{} has no function-tool action; Nomi has not admitted its lifecycle",
                selected.capability.id.as_ref()
            )));
        }
        if capability.manifest.display.name.len() > 256
            || capability.manifest.display.description.len() > 4096
        {
            return Err(error("Plugin tool description exceeds context bounds"));
        }
        let actions = admitted_module_actions(&capability.manifest, &policy.allowed_actions).collect::<Vec<_>>();
        if actions.is_empty() {
            return Err(error("selected Plugin has no allowed function-tool actions"));
        }
        for action in actions {
            if !action_resources_bound(snapshot, &selected.capability.id, &action.action_id)? {
                continue;
            }
            let schema = plugin_schemas
                .resolve(selected, &action.input_schema)
                .await
                .map_err(error)?;
            // Readable origin/action alias; authorization remains bound to the
            // full canonical capability/action identity.
            let identity = format!(
                "{}\0{}",
                selected.capability.id.as_ref(),
                action.action_id.as_ref()
            );
            let name = namespaced_tool_name("plugin", &capability.manifest.display.name,
                relative_action(selected.capability.id.as_ref(), action.action_id.as_ref()),
                identity.as_bytes());
            exposures.push(AgentToolExposure {
                definition: ChatToolDefinition {
                    name,
                    description: format!("{}: {}\nCapability {}, action {}. Executes through the Agent's frozen platform authority.", capability.manifest.display.name, capability.manifest.display.description, selected.capability.id.as_ref(), action.action_id.as_ref()),
                    input_schema: schema,
                    deferred: false,
                },
                capability_id: selected.capability.id.clone(), action_id: action.action_id.clone(),
            });
        }
    }
    let exposures = resource_ready_exposures(snapshot, exposures)?;
    compile_agent_tool_plan(snapshot, active, registry, exposures).map_err(error)
}

fn resource_ready_exposures(
    snapshot: &CompiledSnapshot,
    exposures: Vec<AgentToolExposure>,
) -> Result<Vec<AgentToolExposure>, AppError> {
    let mut resource_ready = Vec::with_capacity(exposures.len());
    for exposure in exposures {
        if action_resources_bound(snapshot, &exposure.capability_id, &exposure.action_id)? {
            resource_ready.push(exposure);
        }
    }
    Ok(resource_ready)
}

fn standard_tool_exposure(
    capability_id: &CapabilityId, action_id: &ActionId,
    canonical: &nomifun_agent_contracts::StrictJsonValue, host_os: &str,
    guide: Option<&ToolModelGuide>,
) -> Result<AgentToolExposure, AppError> {
    let guide = guide.ok_or_else(|| error(format!("{} has no current model guide",action_id.as_ref())))?;
    if guide.capability_id != capability_id.as_ref() || guide.action_id != action_id.as_ref() {
        return Err(error("model guide belongs to another canonical Action"));
    }
    if !concrete_object_schema(&canonical.0, action_id.as_ref() == "workspace.vcs/status") {
        return Err(error(format!("{} has no strict canonical tool schema",capability_id.as_ref())));
    }
    let rendered = guide.render(canonical, host_os).map_err(error)?;
    Ok(AgentToolExposure {
        definition: ChatToolDefinition {
            name: platform_tool_name(capability_id.as_ref(), action_id.as_ref()),
            description: rendered.description, input_schema: rendered.input_schema, deferred: false,
        }, capability_id:capability_id.clone(), action_id:action_id.clone(),
    })
}

/// Model-visible Actions and execution admission share the exact resource
/// operations frozen in their Snapshot descriptors.
fn action_resources_bound(
    snapshot: &CompiledSnapshot,
    capability_id: &CapabilityId,
    action_id: &ActionId,
) -> Result<bool, AppError> {
    snapshot.action_resources_bound(capability_id, action_id).map_err(error)
}

fn admitted_module_actions<'a>(
    manifest: &'a CapabilityManifest,
    allowed_actions: &'a std::collections::BTreeSet<ActionId>,
) -> impl Iterator<Item = &'a nomifun_agent_contracts::CapabilityActionDescriptor> + 'a {
    manifest.contributions.actions.iter().filter(|action| {
        allowed_actions.contains(&action.action_id)
            && !matches!(
                action.presentation,
                ToolPresentationKind::Hidden | ToolPresentationKind::CodeMode
            )
    })
}

fn concrete_object_schema(schema: &serde_json::Value, allow_empty: bool) -> bool {
    let strict = |schema: &serde_json::Value| {
        schema.get("type").and_then(|value| value.as_str()) == Some("object")
            && schema
                .get("additionalProperties")
                .and_then(|value| value.as_bool())
                == Some(false)
            && schema
                .get("properties")
                .and_then(|value| value.as_object())
                .is_some_and(|properties| allow_empty || !properties.is_empty())
    };
    strict(schema)
        || (schema.get("type").and_then(|value| value.as_str()) == Some("object")
            && schema
                .get("oneOf")
                .and_then(|value| value.as_array())
                .is_some_and(|variants| {
                    !variants.is_empty() && variants.len() <= 16 && variants.iter().all(strict)
                }))
}

pub(super) fn validate_session_mcp(
    snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    resources: &[nomifun_agent_contracts::TypedResourceBinding],
) -> Result<(), AppError> {
    super::nomi_core_mcp_catalog::validate_session_selection(snapshot, resources)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        CapabilityActionDescriptor, CapabilityConsumer, CapabilityContributions, CapabilityKind,
        EffectClass, LocalizedMetadata, PackageRef, PlatformConstraint, StrictJsonValue,
        capability_surface_declarations,
    };
    use serde_json::json;

    use super::*;
    use nomifun_agent_contracts::tool_model_guide::tool_model_guides;

    fn canonical_process_schema(action_id: &str) -> StrictJsonValue {
        let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
        let capability = registration.metadata.manifest.payload.contributions.capabilities.into_iter()
            .find(|capability| capability.id.as_ref() == "workspace.process").unwrap();
        let reference = &capability.contributions.actions.iter()
            .find(|action| action.action_id.as_ref() == action_id).unwrap().input_schema;
        nomifun_agent_domain_wave2::resolve_action_schema("workspace.process", reference).unwrap()
    }

    fn platform_snapshot(
        registrations: Vec<nomifun_agent_kernel::PluginRegistration>, selected: &[(&str, &[&str])],
        resource_kinds: &[&str],
    ) -> (CompiledSnapshot, std::sync::Arc<MaterializedRegistry>, std::sync::Arc<nomifun_agent_kernel::KernelRegistry>) {
        use nomifun_agent_contracts::*;
        use nomifun_agent_kernel::*;
        let registry = std::sync::Arc::new(KernelRegistry::new(MaterializationPolicy::stable("1.0.0"),
            std::sync::Arc::new(InMemoryPluginStatePersistence::new())).unwrap());
        let materialized = registry.replace_all(registrations).unwrap();
        let owner = PrincipalRef { principal_kind: "user".into(), principal_id: "creative-owner".into() };
        let mut revision = AgentPresetRevision {
            reference: PresetRevisionRef { preset_id: "creative-resource-fixture".into(), revision: 1, revision_digest: "".into() },
            payload: AgentPresetRevisionPayload {
                context_order: vec![], middleware_order: vec![], schema_version: "1.0.0".into(),
                model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new(),
                enabled_capabilities: selected.iter().map(|(id, actions)| CapabilitySelection {
                    capability: CapabilityRef { id: (*id).into() },
                    action_allowlist: actions.iter().map(|id| ActionId::from(*id)).collect(),
                }).collect(), skill_bindings: vec![], system_role_provider_overrides: BTreeMap::new(),
                persona: String::new(), instructions: String::new(), starter_prompts: vec![], runtime_policy: Default::default(),
            },
            contribution_locks: selected.iter().map(|(id, _)| materialized.capability(&CapabilityId::from(*id))
                .unwrap().contribution_lock.clone()).collect(),
            created_by: "creative-owner".into(), created_at_ms: 1, reason: None,
        };
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        let snapshot = AgentPresetCompiler::compile(&materialized, &CompilerEnvironment {
            resolver_version: "1.0.0".into(), required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: "a".repeat(64).into(), available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(), canonical_schema_manifest_digest: "a".repeat(64).into(),
            target_contribution_manifest_digest: materialized.registry_digest.clone(), host_target: "windows-desktop-x64".into(),
            host_surface: "desktop".into(), availability_evidence_revision: "creative-resource-test".into(),
        }, CompileRequest { revision, principal: owner.clone(), scene: "creative".into(), surface: "desktop".into(),
            audience: "user".into(), created_at_ms: 1, resolver_run_id: "creative-resource-resolve".into(),
        }).unwrap().with_target_resource_bindings(&owner, resource_kinds.iter().map(|kind|TypedResourceBinding {
            binding_id:format!("fixture-{kind}-binding").into(),resource_kind:(*kind).into(),
            resource_id:if *kind=="asset_library" {nomifun_agent_domain_wave3::CREATIVE_ASSET_LIBRARY_RESOURCE_ID.into()}
                else {format!("fixture-{kind}").into()},owner_id:owner.principal_id.clone(),
            operations:BTreeSet::from(["read".into(),"write".into(),"execute".into()]),
            connection_config_ref:None,typed_parameters:BTreeMap::new(),
        }).collect()).unwrap();
        (snapshot, materialized, registry)
    }

    fn creative_snapshot(selected: &[(&str, &[&str])]) -> (CompiledSnapshot, std::sync::Arc<MaterializedRegistry>) {
        let (snapshot,materialized,_)=platform_snapshot(nomifun_agent_domain_wave3::registrations().unwrap(),selected,&["asset_library"]);
        (snapshot,materialized)
    }

    #[tokio::test]
    async fn asset_and_office_tools_are_materialized_without_canvas_and_keep_exact_grants() {
        struct NoPluginSchemas;
        #[async_trait::async_trait]
        impl NomiPluginToolSchemaResolver for NoPluginSchemas {
            async fn resolve(&self, _: &nomifun_agent_contracts::ResolvedCapability,
                _: &nomifun_agent_contracts::CanonicalSchemaRef) -> Result<StrictJsonValue, String> {
                Err("no Plugin selected".into())
            }
        }
        struct LibrarySchemas;
        #[async_trait::async_trait]
        impl NomiPlatformBuiltinToolSchemaResolver for LibrarySchemas {
            async fn resolve(&self, capability: &nomifun_agent_contracts::ResolvedCapability,
                reference: &nomifun_agent_contracts::CanonicalSchemaRef) -> Result<StrictJsonValue, String> {
                if reference.as_ref().contains("/canvas.") {
                    return Err("Canvas schema owner is unavailable".into());
                }
                super::super::agent_wave3_host::resolve_schema(capability, reference)
            }
        }
        for canvas_selected in [false, true] {
            let mut selected: Vec<(&str, &[&str])> = vec![
                ("creative.workshop", &["creative.workshop/asset.read", "creative.workshop/asset.write"]),
                ("office", &["office/preview", "office/document.edit"]),
            ];
            if canvas_selected {
                selected[0].1 = &["creative.workshop/asset.read", "creative.workshop/asset.write", "creative.workshop/canvas.read"];
            }
            let (snapshot, materialized) = creative_snapshot(&selected);
            let active = nomifun_agent_kernel::SessionCapabilityState::new(&snapshot).snapshot().unwrap();
            let plan = compile(&snapshot, &active, &materialized, &NoPluginSchemas,
                &LibrarySchemas, &BTreeSet::new()).await.unwrap();
            let actions = plan.model_definitions().iter().map(|definition| plan.binding(&definition.name).unwrap()
                .action_id.as_ref().to_owned()).collect::<BTreeSet<_>>();
            assert_eq!(actions, BTreeSet::from(["creative.workshop/asset.read".into(), "creative.workshop/asset.write".into(),
                "office/preview".into(), "office/document.edit".into()]));
            super::super::nomi_core_resource_bindings::validate_frozen_action_resource_contracts(snapshot.content()).unwrap();
            let mut incomplete = snapshot.content().clone();
            incomplete.enabled_capabilities[0].actions.iter_mut()
                .for_each(|action| action.required_resource_operations = None);
            assert_eq!(super::super::nomi_core_resource_bindings::validate_frozen_action_resource_contracts(&incomplete)
                .unwrap_err().code(), "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH");
            assert_eq!(snapshot.content().required_resource_kinds,
                if canvas_selected { BTreeSet::from(["asset_library".into(), "canvas".into()]) }
                else { BTreeSet::from(["asset_library".into()]) });
            assert!(!snapshot.action_resources_bound(&"office".into(), &"office/sheet.edit".into()).unwrap());
        }
        let (snapshot, materialized) = creative_snapshot(&[("creative.workshop", &["creative.workshop/canvas.read"])]);
        let active = nomifun_agent_kernel::SessionCapabilityState::new(&snapshot).snapshot().unwrap();
        assert!(compile(&snapshot, &active, &materialized, &NoPluginSchemas, &LibrarySchemas, &BTreeSet::new())
            .await.unwrap().model_definitions().is_empty(), "an unbound Module can have an empty tool table");
        let principal = snapshot.envelope.actor.clone();
        let available = snapshot.with_target_resource_bindings(&principal, vec![nomifun_agent_contracts::TypedResourceBinding {
            binding_id: "available-canvas".into(), resource_kind: "canvas".into(), resource_id: "owned-canvas".into(),
            owner_id: principal.principal_id.clone(), operations: BTreeSet::from(["read".into()]),
            connection_config_ref: None, typed_parameters: BTreeMap::new(),
        }]).unwrap();
        let active = nomifun_agent_kernel::SessionCapabilityState::new(&available).snapshot().unwrap();
        assert!(compile(&available, &active, &materialized, &NoPluginSchemas, &LibrarySchemas, &BTreeSet::new()).await.is_err(),
            "an available resource cannot bypass a missing schema owner");
    }

    fn canonical_workspace_schema(capability_id: &str, action_id: &str) -> StrictJsonValue {
        let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
        let capability = registration.metadata.manifest.payload.contributions.capabilities.into_iter()
            .find(|capability| capability.id.as_ref() == capability_id).unwrap();
        let action = capability.contributions.actions.iter().find(|action| action.action_id.as_ref() == action_id).unwrap();
        nomifun_agent_domain_wave2::resolve_action_schema(capability_id, &action.input_schema).unwrap()
    }

    fn guided_workspace_exposures(host_os: &str) -> Vec<AgentToolExposure> {
        tool_model_guides().iter().map(|guide| {
            let schema = canonical_workspace_schema(&guide.capability_id, &guide.action_id);
            standard_tool_exposure(&guide.capability_id.as_str().into(), &guide.action_id.as_str().into(),
                &schema, host_os, Some(guide)).unwrap()
        }).collect()
    }

    fn restore_guide_annotations(
        rendered: &StrictJsonValue, canonical: &StrictJsonValue, guide: &ToolModelGuide,
    ) -> StrictJsonValue {
        let mut restored = rendered.clone();
        for parameter in &guide.parameters {
            let original=canonical.0.pointer(&parameter.pointer).unwrap().as_object().unwrap();
            let node=restored.0.pointer_mut(&parameter.pointer).unwrap().as_object_mut().unwrap();
            match original.get("description") {
                Some(value)=>{node.insert("description".into(),value.clone());},
                None=>{node.remove("description");},
            }
        }
        restored
    }

    #[test]
    fn all_guides_render_formal_schemas_and_valid_examples_without_constraint_copies() {
        assert_eq!(tool_model_guides().len(),20);
        for host_os in ["windows","macos","linux"] {
            let mut names=BTreeSet::new();
            let mut actions=BTreeSet::new();
            for guide in tool_model_guides() {
                let canonical=canonical_workspace_schema(&guide.capability_id,&guide.action_id);
                let original=canonical.clone();
                let rendered=guide.render(&canonical,host_os).unwrap();
                assert_eq!(restore_guide_annotations(&rendered.input_schema,&canonical,guide),canonical,
                    "{} / {host_os}: guide changed a formal constraint",guide.action_id);
                assert_eq!(canonical,original,"render must not edit a registered schema");
                assert!(names.insert(platform_tool_name(&guide.capability_id,&guide.action_id)));
                assert!(actions.insert((&guide.capability_id,&guide.action_id)));
                let validator=jsonschema::validator_for(&rendered.input_schema.0).unwrap();
                for example in &guide.examples {
                    assert!(validator.is_valid(&example.arguments.0),"{} / {}: example rejected by actual schema",guide.action_id,example.label);
                }
            }
        }
    }

    #[test]
    fn annotation_restoration_does_not_hide_a_real_description_parameter_constraint() {
        let guide=tool_model_guide("workspace.process","workspace.process/exec").unwrap();
        let mut canonical=canonical_process_schema("workspace.process/exec");
        canonical.0["properties"]["description"]=json!({"type":"string","minLength":4});
        let rendered=guide.render(&canonical,std::env::consts::OS).unwrap();
        assert_eq!(restore_guide_annotations(&rendered.input_schema,&canonical,guide),canonical);
        let mut altered=rendered.input_schema;
        altered.0["properties"]["description"]["minLength"]=json!(0);
        assert_ne!(restore_guide_annotations(&altered,&canonical,guide),canonical,
            "a real parameter named description must remain part of the full structural comparison");
    }

    #[test]
    fn rendered_read_formats_keep_neutral_defaults_and_reject_incompatible_options() {
        let tools=guided_workspace_exposures(std::env::consts::OS);
        let read=&tools.iter().find(|tool|tool.definition.name=="read_file").unwrap().definition;
        let validator=jsonschema::validator_for(&read.input_schema.0).unwrap();
        for input in [
            json!({"path":".","format":"instruction_scope","recursive":false,"missing_ok":false}),
            json!({"path":"file.txt","recursive":false,"start_line":2,"line_count":10}),
            json!({"path":"image.png","format":"image","recursive":false,"missing_ok":false}),
            json!({"path":"   "}),
            json!({"path":"protected","format":"instruction_scope","execution_identity":"administrator"}),
            json!({"path":"protected/AGENTS.md","execution_identity":"administrator","offset":0,"limit":128}),
            json!({"path":"protected/image.png","format":"image","execution_identity":"administrator"}),
        ] {assert!(validator.is_valid(&input),"{input}");}
        for input in [
            json!({"path":".","format":"instruction_scope","missing_ok":true}),
            json!({"path":"file.txt","recursive":true}),
            json!({"path":"image.png","format":"image","start_line":1}),
            json!({"path":"file.txt","start_line":1,"offset":0}),
            json!({"path":""}),json!({"path":"invalid\u{0000}path"}),
            json!({"path":"file.txt","execution_identity":"root"}),
            json!({"path":"protected","format":"instruction_scope","execution_identity":"administrator","missing_ok":true}),
        ] {assert!(!validator.is_valid(&input),"{input}");}
        assert!(read.description.contains("sha256 always identifies the entire source"));
        assert!(read.description.contains("call read_file alone on its directory"));
        assert!(read.description.contains("source reads with fresh call IDs"));
        assert!(read.description.contains("never use the default text format for AGENTS.md"));
        assert!(read.input_schema.0["properties"]["expected_sha256"]["description"].as_str().unwrap().contains("whole-source"));
        assert_eq!(read.input_schema.0["properties"]["execution_identity"]["default"], "current_user");
        assert!(read.input_schema.0["properties"]["execution_identity"]["description"].as_str().unwrap().contains("every format"));
        let binding = &tools.iter().find(|tool| tool.definition.name == "read_file").unwrap();
        assert_eq!(binding.capability_id.as_ref(), "workspace.files");
        assert_eq!(binding.action_id.as_ref(), "workspace.files/read");
    }

    #[test]
    fn rendered_push_artifact_and_patch_rules_use_current_formal_contracts() {
        let tools=guided_workspace_exposures(std::env::consts::OS);
        let schema=|name:&str| &tools.iter().find(|tool|tool.definition.name==name).unwrap().definition.input_schema.0;
        let push=jsonschema::validator_for(schema("git_push")).unwrap();
        assert!(push.is_valid(&json!({"remote":"origin","refspec":"HEAD:refs/heads/main","force":false})));
        for input in [json!({"remote":"origin","refspec":"main"}),json!({"remote":"origin","refspec":"HEAD:refs/heads/main","force":true})] {
            assert!(!push.is_valid(&input),"{input}");
        }
        assert_eq!(schema("read_artifact")["properties"]["limit"]["default"],16384);
        let patch=&tools.iter().find(|tool|tool.definition.name=="apply_patch").unwrap().definition;
        assert!(patch.description.contains("lines=[{kind:remove,text:old},{kind:add,text:new}]"));
        let hunk=&schema("apply_patch")["properties"]["files"]["items"]["properties"]["hunks"];
        assert!(hunk["description"].as_str().unwrap().contains("context is unchanged text"));
        let validator=jsonschema::validator_for(schema("apply_patch")).unwrap();
        let guard=json!({"files":[{"path":"a.txt","expected_source":{"kind":"existing","sha256":"a".repeat(64)},
            "hunks":[{"old_start":1,"old_lines":1,"new_start":1,"new_lines":1,
                "lines":[{"kind":"remove","text":"old"},{"kind":"add","text":"new"}]}]}]});
        assert!(validator.is_valid(&guard));
        let mut invalid=guard;invalid["files"][0]["expected_source"]["sha256"]=json!("short");
        assert!(!validator.is_valid(&invalid));
    }

    #[test]
    fn rendered_launch_rules_cover_three_targets_and_the_actual_native_shell() {
        for host_os in ["windows","macos","linux"] {
            let tools=guided_workspace_exposures(host_os);
            for name in ["exec_command","start_process"] {
                let definition=&tools.iter().find(|tool|tool.definition.name==name).unwrap().definition;
                let schema=&definition.input_schema.0;
                assert_eq!(schema["oneOf"].as_array().unwrap().len(),3);
                let validator=jsonschema::validator_for(schema).unwrap();
                for input in [
                    json!({"component":"officecli","args":["a path with spaces","$HOME","$(command)","line\nbreak"]}),
                    json!({"command":"program with spaces","args":["literal"]}),
                    json!({"cmd":"script text","args":[]}),
                ] {assert!(validator.is_valid(&input),"{name}: {input}");}
                for input in [json!({"component":"officecli","cmd":"script"}),json!({"component":"node","command":"node"}),
                    json!({"cmd":"script","args":["forbidden"]}),json!({"component":"unapproved"})] {
                    assert!(!validator.is_valid(&input),"{name}: {input}");
                }
                let args=schema["properties"]["args"]["description"].as_str().unwrap();
                assert!(args.contains("command")&&args.contains("component"));
                assert!(!args.contains("Valid only with command"));
                let cmd=schema["properties"]["cmd"]["description"].as_str().unwrap();
                assert!(cmd.contains(if host_os=="windows" {"Windows PowerShell 5.1"} else {"/bin/sh -c"}));
                if host_os!="windows" {assert!(!cmd.contains("cmd.exe"));}
            }
        }
    }

    #[test]
    fn rendered_text_and_process_lifecycle_keep_exact_bytes_and_one_owned_handle() {
        let tools=guided_workspace_exposures(std::env::consts::OS);
        let definition=|name:&str| &tools.iter().find(|tool|tool.definition.name==name).unwrap().definition;
        let content=definition("write_file").input_schema.0["properties"]["content"]["description"].as_str().unwrap();
        assert!(content.contains("no trailing newline is added"));
        assert!(content.contains(r#"{"content":"first\nsecond\n"}"#));
        let stdin=&definition("write_process_stdin").input_schema.0;
        let append=stdin["properties"]["append_newline"]["description"].as_str().unwrap();
        assert!(append.contains("even if input already ends in LF"));
        let validator=jsonschema::validator_for(stdin).unwrap();
        for input in [json!({"process_id":"owned","input":"你好 MAC-B","append_newline":true}),
            json!({"process_id":"owned","input":"你好 MAC-B\n","append_newline":false}),
            json!({"process_id":"owned","input":"你好 MAC-B\n","append_newline":true})] {assert!(validator.is_valid(&input));}
        assert_eq!(stdin["properties"]["append_newline"]["default"],false);
        assert!(definition("exec_command").description.contains("does not return a live handle"));
        assert!(definition("start_process").description.contains("same Turn"));
        assert!(definition("start_process").description.contains("Never use shell backgrounding"));
        assert!(definition("start_process").description.contains("close_process_stdin"));
        assert_eq!(definition("start_process").input_schema.0["properties"]["wait_ms"]["maximum"],30000,
            "current canonical start wait bounds must not inherit the retired model template's zero limit");
        assert_eq!(definition("resize_process").input_schema.0["properties"]["cols"]["maximum"],32767);
        assert!(definition("cancel_process").description.contains("cleanup.reaped=true"));
    }

    #[test]
    fn rendered_workspace_paths_do_not_invent_a_conversation_or_project_prefix() {
        let tools=guided_workspace_exposures(std::env::consts::OS);
        for name in ["read_file","write_file","delete_path"] {
            let schema=&tools.iter().find(|tool|tool.definition.name==name).unwrap().definition.input_schema.0;
            let path=schema["properties"]["path"]["description"].as_str().unwrap();
            assert!(path.contains("native absolute path"),"{name}: local absolute paths lost");
            assert!(path.contains("relative to the Session default working directory"),"{name}: default-directory provenance lost");
            assert!(path.contains("Do not prepend project or conversation display names"),"{name}: root provenance lost");
        }
        // Project business identity remains exact even though local file and
        // process access no longer stops at that project's root.
        for name in ["git_stage","publish_artifact"] {
            let schema=&tools.iter().find(|tool|tool.definition.name==name).unwrap().definition.input_schema.0;
            let path=schema["properties"]["path"]["description"].as_str().unwrap();
            assert!(path.contains("without prepending a project or conversation display name"),"{name}: project provenance lost");
        }
        let search=&tools.iter().find(|tool|tool.definition.name=="search_files").unwrap().definition.input_schema.0;
        assert!(search["properties"]["path"]["description"].as_str().unwrap().contains("Session default working directory"));
        for name in ["exec_command","start_process"] {
            let schema=&tools.iter().find(|tool|tool.definition.name==name).unwrap().definition.input_schema.0;
            let cwd=schema["properties"]["cwd"]["description"].as_str().unwrap();
            assert!(cwd.contains("Native absolute directory"),"{name}: absolute cwd guidance lost");
            assert!(cwd.contains("relative to the Session default working directory"),"{name}: default cwd provenance lost");
            assert!(cwd.contains("not a local access boundary"),"{name}: obsolete cwd restriction returned");
        }
    }

    #[test]
    fn generated_platform_tool_names_are_stable_bounded_and_action_specific() {
        let navigate = platform_tool_name("browser", "browser/navigate");
        assert_eq!(navigate, platform_tool_name("browser", "browser/navigate"));
        assert_ne!(navigate, platform_tool_name("browser", "browser/observe"));
        assert_eq!(navigate, "browser_navigate");
        assert!(navigate.len() <= 64);
        assert!(
            navigate
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        );
    }

    #[test]
    fn presentation_catalogue_covers_every_builtin_function_action_and_keeps_native_names() {
        let registrations = [
            nomifun_agent_domain_wave1::registrations().unwrap(),
            nomifun_agent_domain_wave2::registrations().unwrap(),
            nomifun_agent_domain_wave3::registrations().unwrap(),
            nomifun_agent_domain_wave4::registrations().unwrap(),
            nomifun_agent_domain_wave5::registrations().unwrap(),
        ];
        for registration in registrations.into_iter().flatten() {
            for capability in registration.metadata.manifest.payload.contributions.capabilities {
                for action in capability.contributions.actions {
                    if action.presentation == ToolPresentationKind::FunctionTool {
                        assert!(nomifun_agent_contracts::tool_presentation::tool_presentation(
                            capability.id.as_ref(), action.action_id.as_ref()).is_some(),
                            "missing tool presentation: {} / {}", capability.id.as_ref(), action.action_id.as_ref());
                    }
                }
            }
        }
        for exposure in guided_workspace_exposures(std::env::consts::OS) {
            assert_eq!(platform_tool_name(exposure.capability_id.as_ref(), exposure.action_id.as_ref()),
                exposure.definition.name);
        }
    }

    struct UnselectedSchemas;
    #[async_trait::async_trait]
    impl NomiPluginToolSchemaResolver for UnselectedSchemas {
        async fn resolve(&self,_:&nomifun_agent_contracts::ResolvedCapability,_:&nomifun_agent_contracts::CanonicalSchemaRef)
            ->Result<StrictJsonValue,String>{Err("no Plugin selected".into())}
    }
    #[async_trait::async_trait]
    impl NomiPlatformBuiltinToolSchemaResolver for UnselectedSchemas {
        async fn resolve(&self,_:&nomifun_agent_contracts::ResolvedCapability,_:&nomifun_agent_contracts::CanonicalSchemaRef)
            ->Result<StrictJsonValue,String>{Err("workspace standard action must not use the generic resolver".into())}
    }

    #[tokio::test]
    async fn full_workspace_tool_plan_and_partial_grants_use_only_formal_actions_and_new_guides() {
        let all=[("workspace.files",nomifun_agent_domain_wave2::WORKSPACE_FILES_ACTION_IDS),
            ("workspace.vcs",nomifun_agent_domain_wave2::WORKSPACE_VCS_ACTION_IDS),
            ("workspace.process",nomifun_agent_domain_wave2::WORKSPACE_PROCESS_ACTION_IDS),
            ("workspace.artifacts",nomifun_agent_domain_wave2::WORKSPACE_ARTIFACTS_ACTION_IDS)];
        for selected in [all.as_slice(),&[("workspace.files",&["workspace.files/read"][..])][..]] {
            let (snapshot,registry,_)=platform_snapshot(vec![nomifun_agent_domain_wave2::workspace_execution_registration().unwrap()],
                selected,&["workspace","process_session"]);
            let active=nomifun_agent_kernel::SessionCapabilityState::new(&snapshot).snapshot().unwrap();
            let plan=compile(&snapshot,&active,&registry,&UnselectedSchemas,&UnselectedSchemas,&BTreeSet::new()).await.unwrap();
            assert_eq!(plan.len(),if selected.len()==4 {20} else {1});
            for definition in plan.model_definitions() {
                let binding=plan.binding(&definition.name).unwrap();
                let cap=registry.capability(&binding.capability_id).unwrap();
                let action=cap.manifest.contributions.actions.iter().find(|action|action.action_id==binding.action_id).unwrap();
                assert_eq!(binding.canonical_input_schema_ref,action.input_schema);
                assert_eq!(binding.schema_digest,nomifun_engine_core::input_schema_digest(&definition.input_schema).unwrap());
                let guide=tool_model_guide(binding.capability_id.as_ref(),binding.action_id.as_ref()).unwrap();
                let canonical=nomifun_agent_domain_wave2::resolve_action_schema(binding.capability_id.as_ref(),&action.input_schema).unwrap();
                assert_eq!(restore_guide_annotations(&definition.input_schema,&canonical,guide),canonical);
                assert_eq!(binding.definition.deferred,false);
                assert_eq!(definition.name,platform_tool_name(binding.capability_id.as_ref(),binding.action_id.as_ref()));
            }
            if selected.len()==1 {assert!(plan.binding("read_file").is_some());assert!(plan.binding("write_file").is_none());}
        }
    }

    #[test]
    fn missing_or_cross_action_guide_cannot_fall_back_to_generic_standard_tool_data() {
        let canonical=canonical_process_schema("workspace.process/exec");
        assert!(standard_tool_exposure(&"workspace.process".into(),&"workspace.process/exec".into(),&canonical,
            std::env::consts::OS,None).unwrap_err().to_string().contains("no current model guide"));
        assert!(standard_tool_exposure(&"workspace.process".into(),&"workspace.process/exec".into(),&canonical,
            std::env::consts::OS,tool_model_guide("workspace.process","workspace.process/start")).is_err());
    }

    #[tokio::test]
    async fn rendered_foreign_host_fact_preserves_the_actual_owner_pre_dispatch_guard() {
        use nomifun_engine_core::{EngineToolInvoker,KernelEngineToolInvoker};
        let (snapshot,materialized,registry)=platform_snapshot(vec![nomifun_agent_domain_wave2::workspace_execution_registration().unwrap()],
            &[("workspace.process",&["workspace.process/exec"])],&["process_session"]);
        let active=std::sync::Arc::new(nomifun_agent_kernel::SessionCapabilityState::new(&snapshot));
        let observed=active.snapshot().unwrap();
        let canonical=canonical_process_schema("workspace.process/exec");
        let foreign=if std::env::consts::OS=="windows" {"linux"} else {"windows"};
        let exposure=standard_tool_exposure(&"workspace.process".into(),&"workspace.process/exec".into(),&canonical,
            foreign,tool_model_guide("workspace.process","workspace.process/exec")).unwrap();
        assert!(exposure.definition.description.starts_with(&format!("Workspace process host OS: {foreign}.")));
        let plan=compile_agent_tool_plan(&snapshot,&observed,&materialized,[exposure]).unwrap();
        let binding=plan.binding("exec_command").unwrap().clone();
        let owner=snapshot.envelope.actor.clone();
        let session=nomifun_agent_contracts::AgentSessionId::from("host-fact-fixture");
        let port=KernelEngineToolInvoker::for_session(registry,std::sync::Arc::new(snapshot.clone()),active,
            owner.clone(),session.clone(),plan).unwrap();
        let result=port.invoke(nomifun_engine_core::EngineToolInvocation {
            agent_session_id:session,principal:owner,resolved_snapshot_ref:snapshot.snapshot_ref().clone(),
            active_set_generation:observed.generation,turn_operation_id:"host-fact-turn".into(),operation_id:"host-fact-operation".into(),
            idempotency_key:"host-fact-invocation".into(),correlation_id:"host-fact-call".into(),
            call:nomifun_chat_model_broker::ChatToolCall {call_id:"host-fact-call".into(),name:"exec_command".into(),
                arguments:StrictJsonValue(json!({"cmd":"THIS_MUST_NOT_RUN"})),provider_metadata:None},binding,
        },tokio_util::sync::CancellationToken::new()).await.unwrap();
        assert!(result.is_error);
        let output:serde_json::Value=serde_json::from_str(&result.output_text()).unwrap();
        assert_eq!(output["code"],"HOST_OS_COMMAND_MAPPING_ERROR");
        assert_eq!(output["state"],"not_started");
        assert_eq!(output["advertised_host_os"],foreign);
        assert_eq!(output["actual_host_os"],std::env::consts::OS);
    }

    #[test]
    fn union_input_schemas_pass_the_runtime_model_tool_gate() {
        for action_id in [
            nomifun_agent_domain_wave5::REQUIREMENTS_READ_ACTION_ID,
            nomifun_agent_domain_wave5::REQUIREMENTS_WRITE_ACTION_ID,
        ] {
            let schema = nomifun_agent_domain_wave5::action_input_schema_for(action_id).unwrap();
            assert!(concrete_object_schema(&schema.0, true), "{action_id}");
        }

        let ssh = nomifun_agent_domain_wave2::registrations()
            .unwrap()
            .into_iter()
            .flat_map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
            })
            .find(|capability| capability.id.as_ref() == nomifun_agent_domain_wave2::SSH_MODULE_ID)
            .unwrap();
        let read = ssh
            .contributions
            .actions
            .iter()
            .find(|action| action.action_id.as_ref() == "ssh/fs.read")
            .unwrap();
        let schema = nomifun_agent_domain_wave2::resolve_action_schema(
            nomifun_agent_domain_wave2::SSH_MODULE_ID,
            &read.input_schema,
        )
        .unwrap();
        assert!(concrete_object_schema(&schema.0, true));
    }

    fn assert_strict_model_tool_inputs(
        family: &str,
        registrations: Vec<nomifun_agent_kernel::PluginRegistration>,
        resolve: impl Fn(
            &str,
            &nomifun_agent_contracts::CanonicalSchemaRef,
        ) -> Result<StrictJsonValue, String>,
    ) {
        for capability in registrations.into_iter().flat_map(|registration| {
            registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
        }) {
            for action in capability
                .contributions
                .actions
                .iter()
                .filter(|action| action.presentation == ToolPresentationKind::FunctionTool)
            {
                let schema = resolve(capability.id.as_ref(), &action.input_schema)
                    .unwrap_or_else(|error| panic!("{family} {}: {error}", action.action_id.as_ref()));
                assert!(
                    concrete_object_schema(&schema.0, true),
                    "{family} action {} has no strict object input",
                    action.action_id.as_ref(),
                );
            }
        }
    }

    #[test]
    fn every_bundled_function_action_passes_the_runtime_model_tool_gate() {
        assert_strict_model_tool_inputs(
            "Wave 1",
            nomifun_agent_domain_wave1::registrations().unwrap(),
            nomifun_agent_domain_wave1::resolve_canonical_schema,
        );
        assert_strict_model_tool_inputs(
            "Wave 2",
            nomifun_agent_domain_wave2::registrations().unwrap(),
            nomifun_agent_domain_wave2::resolve_action_schema,
        );
        assert_strict_model_tool_inputs(
            "Wave 3",
            nomifun_agent_domain_wave3::registrations().unwrap(),
            nomifun_agent_domain_wave3::resolve_action_schema,
        );
        assert_strict_model_tool_inputs(
            "Wave 4",
            nomifun_agent_domain_wave4::registrations().unwrap(),
            |_capability_id, reference| {
                nomifun_agent_domain_wave4::resolve_capability_schema(reference)?
                    .ok_or_else(|| format!("schema {} is unavailable", reference.as_ref()))
            },
        );
        assert_strict_model_tool_inputs(
            "Wave 5",
            nomifun_agent_domain_wave5::registrations().unwrap(),
            nomifun_agent_domain_wave5::resolve_action_schema,
        );
    }

    #[test]
    fn display_kind_does_not_change_an_exact_plugin_action_grant() {
        let action_id = ActionId::from("mixed.module/run");
        let mut manifest = CapabilityManifest {
            id: CapabilityId::from("mixed.module"),
            contribution_id: "capability:mixed.module".into(),
            kind: CapabilityKind::Tool,
            package: PackageRef {
                id: "fixture.package".into(),
                version: "1.0.0".into(),
            },
            display: LocalizedMetadata {
                name: "Mixed".into(),
                description: "Action plus Context/Event contributions".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({"type":"object"})),
            contributions: CapabilityContributions {
                actions: vec![CapabilityActionDescriptor {
                    action_id: action_id.clone(),
                    input_schema: "schema://mixed/run/input@1".into(),
                    output_schema: "schema://mixed/run/output@1".into(),
                    effect_class: EffectClass::Pure,
                    presentation: ToolPresentationKind::FunctionTool,
                    required_resource_operations: Default::default(),
                }],
                context_schema_refs: vec!["schema://mixed/context@1".into()],
                event_schema_refs: vec!["schema://mixed/event@1".into()],
                ..Default::default()
            },
        };
        let allowed = BTreeSet::from([action_id.clone()]);
        for kind in [
            CapabilityKind::Tool,
            CapabilityKind::ContextContributor,
            CapabilityKind::EventSource,
        ] {
            manifest.kind = kind;
            assert_eq!(
                admitted_module_actions(&manifest, &allowed)
                    .map(|action| action.action_id.clone())
                    .collect::<Vec<_>>(),
                vec![action_id.clone()],
            );
        }
    }

}
