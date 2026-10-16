//! Presentation within an already compiled frozen ToolPlan, never authority
//! selection. Large catalogs retain their full capabilities and exact schemas.
use nomifun_agent_runtime::{
    AgentToolPlan, INITIAL_VISIBLE_TOOL_DEFINITIONS, MAX_VISIBLE_TOOL_DEFINITIONS,
    VISIBLE_TOOL_DEFINITION_BYTE_BUDGET,
};
use nomifun_common::AppError;

const INITIAL_TOOL_BYTES: usize = VISIBLE_TOOL_DEFINITION_BYTE_BUDGET;
const MAX_VISIBLE_TOOLS: usize = MAX_VISIBLE_TOOL_DEFINITIONS;
// Shared runtime limits reserve the existing control tools and one complete
// ToolSearch result. They bound presentation, never the authorized catalog.
const INITIAL_VISIBLE_TOOLS: usize = INITIAL_VISIBLE_TOOL_DEFINITIONS;
pub(super) fn project(plan: AgentToolPlan, discovery_available: bool) -> Result<AgentToolPlan, AppError> {
    project_with_policy(plan, discovery_available, false)
}

/// A robot-only interaction has already selected its device and Action grants.
/// Keep its bounded concrete surface in the initial window instead of spending
/// a model round discovering that same device again. Non-deferred definitions
/// also remain visible when later ToolSearch results replace deferred schemas.
pub(super) fn project_robot_interaction(plan: AgentToolPlan, discovery_available: bool) -> Result<AgentToolPlan, AppError> {
    project_with_policy(plan, discovery_available, true)
}

fn project_with_policy(plan: AgentToolPlan, discovery_available: bool, robot_interaction: bool) -> Result<AgentToolPlan, AppError> {
    let definitions = plan.model_definitions();
    let visible = definitions.iter().filter(|definition| !definition.deferred).collect::<Vec<_>>();
    if !discovery_available {
        if visible.len() > MAX_VISIBLE_TOOLS {
            return Err(AppError::Conflict(
                "Nomi tool presentation exceeds 128 visible actions and ToolSearch is unavailable".into(),
            ));
        }
        return Ok(plan);
    }
    let bytes = serde_json::to_vec(&visible)
        .map_err(|error| AppError::Internal(error.to_string()))?.len();
    if visible.len() <= INITIAL_VISIBLE_TOOLS && bytes <= INITIAL_TOOL_BYTES {
        return Ok(plan);
    }
    let mut bindings = definitions.iter().map(|definition| {
        let mut binding = plan.binding(&definition.name).expect("compiled tool has a binding").clone();
        // Keep ordinary workspace work and the general-purpose web/delegation
        // entry points ready. Heavy business/provider schemas remain in the
        // exact same plan and are revealed by the selected ToolSearch policy.
        let initially_ready = if robot_interaction {
            matches!(binding.capability_id.as_ref(), "robot" | "companion.memory")
        } else {
            matches!(binding.capability_id.as_ref(),
                "workspace.files" | "workspace.vcs" | "workspace.process" | "web.research" | "agent.collaboration")
        };
        if !initially_ready {
            binding.definition.deferred = true;
        }
        binding
    }).collect::<Vec<_>>();
    // Spend the schema budget on ordinary file work before rich process,
    // delegation and patch definitions. Alphabetical order let apply_patch
    // consume the budget before write_file, hiding a basic file capability.
    // Oversized definitions stay in the exact plan for discovery.
    // Give each granted Robot Action its smallest fitting representative before
    // spending the remainder on a large motion/extension catalog. The group
    // identities come from the host-frozen mapping, never tool-name guesses.
    let mut representatives = std::collections::BTreeMap::new();
    if robot_interaction {
        for binding in bindings.iter().filter(|binding| binding.capability_id.as_ref() == "robot" && !binding.definition.deferred) {
            let bytes = serde_json::to_vec(&binding.definition)
                .map_err(|error| AppError::Internal(error.to_string()))?.len();
            if bytes + 2 <= INITIAL_TOOL_BYTES {
                let candidate = (bytes, binding.model_name.clone());
                let selected = representatives.entry(binding.action_id.clone()).or_insert_with(|| candidate.clone());
                if candidate < *selected { *selected = candidate; }
            }
        }
    }
    let representatives = representatives.into_values().map(|(_, name)| name).collect::<std::collections::BTreeSet<_>>();
    bindings.sort_by_key(|binding| (
        if robot_interaction {
            robot_visibility_priority(binding) + if representatives.contains(&binding.model_name) { 0 } else { 4 }
        } else { initial_visibility_priority(binding) },
        binding.model_name.clone(),
    ));
    let mut visible_count = 0;
    let mut visible_bytes = 2; // the surrounding JSON array
    for binding in &mut bindings {
        if binding.definition.deferred {
            continue;
        }
        let definition_bytes = serde_json::to_vec(&binding.definition)
            .map_err(|error| AppError::Internal(error.to_string()))?.len();
        let next_bytes = visible_bytes + definition_bytes + usize::from(visible_count > 0);
        if visible_count >= INITIAL_VISIBLE_TOOLS || next_bytes > INITIAL_TOOL_BYTES {
            binding.definition.deferred = true;
        } else {
            visible_count += 1;
            visible_bytes = next_bytes;
        }
    }
    AgentToolPlan::new(bindings).map_err(|error| AppError::Internal(error.to_string()))
}

fn robot_visibility_priority(binding: &nomifun_agent_runtime::AgentToolBinding) -> u8 {
    match (binding.capability_id.as_ref(), binding.action_id.as_ref()) {
        ("robot", "robot/motion") => 0,
        ("robot", "robot/device") => 1,
        ("robot", "robot/display" | "robot/vision") => 2,
        ("companion.memory", _) => 3,
        _ => 4,
    }
}

fn initial_visibility_priority(binding: &nomifun_agent_runtime::AgentToolBinding) -> u8 {
    match (binding.capability_id.as_ref(), binding.action_id.as_ref()) {
        ("workspace.files", "workspace.files/read" | "workspace.files/write" | "workspace.files/search") => 0,
        ("workspace.process", "workspace.process/poll") | ("web.research", "web.research/search" | "web.research/fetch") => 1,
        ("workspace.process", "workspace.process/exec") => 2,
        ("workspace.vcs", "workspace.vcs/status" | "workspace.vcs/diff")
        | ("workspace.process", "workspace.process/input" | "workspace.process/close_stdin" | "workspace.process/cancel") => 3,
        ("agent.collaboration", "agent/delegate" | "agent/fork" | "agent/request_user_decision") => 4,
        ("workspace.process", "workspace.process/start") => 5,
        ("workspace.files", "workspace.files/patch") | ("workspace.process", "workspace.process/resize") => 7,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nomifun_agent_contracts::{
        ActionId, CanonicalSchemaRef, CapabilityId, DigestHex, StrictJsonValue,
    };
    use nomifun_chat_model_broker::ChatToolDefinition;
    use nomifun_engine_core::{
        EngineEffectClass, EngineToolBinding, input_schema_digest,
    };
    use serde_json::json;

    use super::*;

    fn binding(name: &str, capability_id: &str, description: &str) -> EngineToolBinding {
        let schema = StrictJsonValue(json!({
            "type": "object",
            "properties": {"text": {"type": "string"}}
        }));
        EngineToolBinding {
            model_name: name.to_owned(),
            definition: ChatToolDefinition {
                name: name.to_owned(),
                description: description.to_owned(),
                input_schema: schema.clone(),
                deferred: false,
            },
            schema_digest: input_schema_digest(&schema).unwrap(),
            canonical_input_schema_ref: CanonicalSchemaRef::from(format!(
                "schema://{capability_id}/{name}/input"
            )),
            capability_contract_digest: DigestHex::from("b".repeat(64)),
            capability_id: CapabilityId::from(capability_id),
            action_id: ActionId::from(format!("{capability_id}/{name}")),
            resource_binding_ids: BTreeSet::new(),
            effect_class: EngineEffectClass::ReadOnly,
            parallel_safe: true,
        }
    }

    #[test]
    fn large_catalog_preserves_every_exact_tool_and_defers_heavy_schemas() {
        let plan = AgentToolPlan::new([
            binding("mcp_lookup", "nomi.mcp.v1.example", "Application MCP tool"),
            binding("crm_lookup", "crm.contacts", &"x".repeat(INITIAL_TOOL_BYTES)),
            binding("files_read", "workspace.files", "read"),
        ]).unwrap();
        let projected = project(plan.clone(), true).unwrap();
        assert_eq!(projected.len(), plan.len());
        for name in ["mcp_lookup", "crm_lookup"] {
            let mut binding = projected.binding(name).unwrap().clone();
            assert!(binding.definition.deferred);
            binding.definition.deferred = false;
            assert_eq!(&binding, plan.binding(name).unwrap());
        }
        assert!(!projected.binding("files_read").unwrap().definition.deferred);
    }

    #[test]
    fn general_preset_with_sixty_mcp_tools_retains_all_ready_authority() {
        use nomifun_agent_contracts::{OfficialPresetKey, official_preset_seed_manifest_payload, tool_presentation::platform_tool_name};

        let manifest = official_preset_seed_manifest_payload();
        let general = &manifest.templates[&OfficialPresetKey::AssistantGeneral];
        assert_eq!(general.enabled_capabilities.iter().map(|selection| selection.action_allowlist.len()).sum::<usize>(), 89);
        // A normal installation may have no KB, SSH or desktop adapter yet.
        // The other seventy Actions have automatic or zero resource needs.
        let mut bindings = general.enabled_capabilities.iter()
            .filter(|selection| !matches!(selection.capability.id.as_ref(), "knowledge" | "ssh" | "browser" | "computer"))
            .flat_map(|selection| selection.action_allowlist.iter().map(|action| {
                let mut native = binding(
                    &platform_tool_name(selection.capability.id.as_ref(), action.as_ref()),
                    selection.capability.id.as_ref(),
                    "Native official Action",
                );
                native.action_id = action.clone();
                native
            }))
            .collect::<Vec<_>>();
        assert_eq!(bindings.len(), 70);
        bindings.extend((0..60).map(|index| binding(
            &format!("mcp_fixture_tool_{index:02}"),
            &format!("mcp.fixture.tool_{index:02}"),
            "Frozen MCP Action",
        )));
        let original = AgentToolPlan::new(bindings).unwrap();
        let projected = project(original.clone(), true).unwrap();
        assert_eq!(projected.len(), 130);
        let visible = projected.model_definitions().into_iter().filter(|definition| !definition.deferred).collect::<Vec<_>>();
        assert!(visible.len() <= INITIAL_VISIBLE_TOOLS);
        assert!(serde_json::to_vec(&visible).unwrap().len() <= INITIAL_TOOL_BYTES);
        assert!(!projected.binding("read_file").unwrap().definition.deferred);
        for definition in original.model_definitions() {
            let before = original.binding(&definition.name).unwrap();
            let mut after = projected.binding(&definition.name).unwrap().clone();
            if definition.name.starts_with("mcp_fixture") {
                assert!(after.definition.deferred);
            }
            after.definition.deferred = before.definition.deferred;
            assert_eq!(&after, before, "presentation must retain exact authority and schema for {}", definition.name);
        }
        assert!(project(original, false).is_err(), "without discovery all 130 tools would be directly visible");
    }

    #[test]
    fn tiny_definitions_over_the_visible_count_are_deferred() {
        let original = AgentToolPlan::new((0..(MAX_VISIBLE_TOOLS + 1)).map(|index| {
            let mut tool = binding(&format!("tiny_{index:03}"), "workspace.files", "");
            tool.definition.input_schema = StrictJsonValue(json!({"type":"object"}));
            tool.schema_digest = input_schema_digest(&tool.definition.input_schema).unwrap();
            tool
        })).unwrap();
        assert!(serde_json::to_vec(&original.model_definitions()).unwrap().len() < INITIAL_TOOL_BYTES);
        let projected = project(original.clone(), true).unwrap();
        assert_eq!(projected.len(), MAX_VISIBLE_TOOLS + 1);
        assert_eq!(projected.model_definitions().iter().filter(|definition| !definition.deferred).count(), INITIAL_VISIBLE_TOOLS);
        for definition in original.model_definitions() {
            let before = original.binding(&definition.name).unwrap();
            let mut after = projected.binding(&definition.name).unwrap().clone();
            after.definition.deferred = false;
            assert_eq!(&after, before);
        }
        assert!(project(original, false).is_err());
    }

    #[test]
    fn no_discovery_does_not_reject_a_large_schema_that_is_within_the_visible_count() {
        let original = AgentToolPlan::new([binding("large_read", "workspace.files", &"x".repeat(INITIAL_TOOL_BYTES))]).unwrap();
        assert_eq!(project(original.clone(), false).unwrap(), original);
        let projected = project(original, true).unwrap();
        assert!(projected.binding("large_read").unwrap().definition.deferred);
    }

    fn robot_binding(name: &str, action: &str, description: &str) -> EngineToolBinding {
        let mut tool = binding(name, "robot", description);
        tool.action_id = action.into();
        tool.resource_binding_ids.insert("exact-selected-device".into());
        tool.effect_class = EngineEffectClass::ExternalUncertainEffect;
        tool.parallel_safe = false;
        tool
    }

    #[test]
    fn robot_interaction_keeps_motion_and_device_schema_visible_under_byte_pressure() {
        let original = AgentToolPlan::new([
            robot_binding("bound_arm_pose", "robot/motion", "Move the bound arm"),
            robot_binding("bound_status", "robot/device", "Read the bound device status"),
            robot_binding("bound_extension", "robot/device", &"x".repeat(INITIAL_TOOL_BYTES)),
            binding("recall", "companion.memory", "Recall memory"),
        ]).unwrap();
        let projected = project_robot_interaction(original.clone(), true).unwrap();
        for name in ["bound_arm_pose", "bound_status"] {
            assert!(!projected.binding(name).unwrap().definition.deferred,
                "a non-deferred base schema survives later ToolSearch window replacement");
        }
        assert!(projected.binding("bound_extension").unwrap().definition.deferred);
        for definition in original.model_definitions() {
            let before = original.binding(&definition.name).unwrap();
            let mut after = projected.binding(&definition.name).unwrap().clone();
            after.definition.deferred = before.definition.deferred;
            assert_eq!(&after, before, "presentation must preserve schema, resource and Effect authority");
        }
        let ordinary = project(original, true).unwrap();
        assert!(ordinary.binding("bound_arm_pose").unwrap().definition.deferred,
            "the broad Companion presentation policy must remain unchanged");
    }

    #[test]
    fn large_robot_catalog_reserves_discovery_slots_without_preactivating_every_tool() {
        let mut tools = (0..128).map(|index| robot_binding(
            &format!("motion_{index:03}"), "robot/motion", "move",
        )).collect::<Vec<_>>();
        tools.push(robot_binding("status", "robot/device", "status"));
        let projected = project_robot_interaction(AgentToolPlan::new(tools).unwrap(), true).unwrap();
        let visible = projected.model_definitions().into_iter().filter(|definition| !definition.deferred).collect::<Vec<_>>();
        assert!(visible.len() <= INITIAL_VISIBLE_TOOLS);
        assert!(serde_json::to_vec(&visible).unwrap().len() <= INITIAL_TOOL_BYTES);
        assert!(!projected.binding("motion_000").unwrap().definition.deferred);
        assert!(!projected.binding("status").unwrap().definition.deferred,
            "a large motion group must leave room for another granted Action");
        assert!(projected.model_definitions().iter().any(|definition| definition.deferred));
        assert_eq!(projected.len(), 129, "the complete frozen catalog remains searchable");
    }

    #[test]
    fn robot_action_representatives_prefer_small_schemas_to_keep_other_actions_ready() {
        let original = AgentToolPlan::new([
            robot_binding("a_rich_motion", "robot/motion", &"x".repeat(INITIAL_TOOL_BYTES - 1024)),
            robot_binding("z_simple_motion", "robot/motion", "A small pose schema"),
            robot_binding("status", "robot/device", &"s".repeat(1024)),
            robot_binding("display", "robot/display", &"d".repeat(1024)),
        ]).unwrap();
        let projected = project_robot_interaction(original.clone(), true).unwrap();
        for name in ["z_simple_motion", "status", "display"] {
            assert!(!projected.binding(name).unwrap().definition.deferred, "small granted Action must fit: {name}");
        }
        assert!(projected.binding("a_rich_motion").unwrap().definition.deferred,
            "alphabetically early rich schemas must not consume other Action representatives' budget");
        assert_eq!(projected.len(), original.len());
        assert_eq!(project_robot_interaction(original, true).unwrap(), projected, "size/name tie-breaking is deterministic");
    }

    #[test]
    fn robot_presentation_does_not_manufacture_unbound_or_revoked_actions() {
        let no_device = AgentToolPlan::new([binding("recall", "companion.memory", "memory")]).unwrap();
        let no_device = project_robot_interaction(no_device, true).unwrap();
        assert!(!no_device.contains_capability("robot"));
        let display_only = AgentToolPlan::new([robot_binding("display", "robot/display", "show")]).unwrap();
        let projected = project_robot_interaction(display_only.clone(), true).unwrap();
        assert_eq!(projected, display_only);
        assert!(projected.model_name_for_action("robot", "robot/motion").is_none());
        assert!(projected.model_name_for_action("robot", "robot/device").is_none());
    }

    #[tokio::test]
    async fn real_native_schemas_keep_basic_file_work_visible_before_heavy_actions() {
        use std::{collections::BTreeMap, sync::Arc};
        use nomifun_agent_contracts::*;
        use nomifun_agent_kernel::{AgentPresetCompiler, CompileRequest, CompilerEnvironment,
            InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy, SessionCapabilityState};
        use nomifun_ai_agent::{NomiPlatformBuiltinToolSchemaResolver, NomiPluginToolSchemaResolver};

        struct NativeSchemas;
        #[async_trait::async_trait]
        impl NomiPluginToolSchemaResolver for NativeSchemas {
            async fn resolve(&self, _: &ResolvedCapability, _: &CanonicalSchemaRef) -> Result<StrictJsonValue, String> {
                Err("this native fixture selects no Plugin tools".into())
            }
        }
        #[async_trait::async_trait]
        impl NomiPlatformBuiltinToolSchemaResolver for NativeSchemas {
            async fn resolve(&self, capability: &ResolvedCapability, reference: &CanonicalSchemaRef) -> Result<StrictJsonValue, String> {
                match capability.capability.id.as_ref() {
                    "web.research" => nomifun_agent_domain_wave1::resolve_canonical_schema("web.research", reference),
                    "agent.collaboration" => nomifun_agent_domain_wave5::resolve_action_schema("agent.collaboration", reference),
                    id => nomifun_agent_domain_wave2::resolve_action_schema(id, reference),
                }
            }
        }
        let kernel = KernelRegistry::new(MaterializationPolicy::stable("1.0.0"),
            Arc::new(InMemoryPluginStatePersistence::new())).unwrap();
        let mut registrations = vec![nomifun_agent_domain_wave2::workspace_execution_registration().unwrap()];
        registrations.extend(nomifun_agent_domain_wave1::registrations().unwrap());
        registrations.extend(nomifun_agent_domain_wave5::registrations().unwrap());
        let registry = kernel.replace_all(registrations).unwrap();
        let selected = ["workspace.files", "workspace.process", "workspace.vcs", "workspace.artifacts", "web.research", "agent.collaboration"];
        let owner = PrincipalRef { principal_kind: "user".into(), principal_id: "presentation-owner".into() };
        let mut revision = AgentPresetRevision {
            reference: PresetRevisionRef { preset_id: "native-presentation".into(), revision: 1, revision_digest: "".into() },
            payload: AgentPresetRevisionPayload {
                context_order: vec![], middleware_order: vec![], schema_version: "1.0.0".into(),
                model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new(),
                enabled_capabilities: selected.iter().map(|id| {
                    let capability = registry.capability(&CapabilityId::from(*id)).unwrap();
                    CapabilitySelection { capability: CapabilityRef { id: (*id).into() },
                        action_allowlist: capability.manifest.contributions.actions.iter().map(|action| action.action_id.clone()).collect() }
                }).collect(), skill_bindings: vec![], system_role_provider_overrides: BTreeMap::new(),
                persona: String::new(), instructions: String::new(), starter_prompts: vec![], runtime_policy: Default::default(),
            },
            contribution_locks: selected.iter().map(|id| registry.capability(&CapabilityId::from(*id)).unwrap().contribution_lock.clone()).collect(),
            created_by: owner.principal_id.clone().into(), created_at_ms: 1, reason: None,
        };
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        let snapshot = AgentPresetCompiler::compile(&registry, &CompilerEnvironment {
            resolver_version: "1.0.0".into(), required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: "a".repeat(64).into(),
            available_runtime_features: registry.capabilities.values().flat_map(|capability|
                capability.manifest.requires_runtime_features.iter().map(|feature| feature.id.clone())).collect(),
            installation_role_bindings: BTreeMap::new(), canonical_schema_manifest_digest: "a".repeat(64).into(),
            target_contribution_manifest_digest: registry.registry_digest.clone(), host_target: "x86_64-pc-windows-msvc".into(),
            host_surface: "desktop".into(), availability_evidence_revision: "native-presentation-test".into(),
        }, CompileRequest { revision, principal: owner.clone(), scene: "native-presentation".into(), surface: "desktop".into(),
            audience: "owner".into(), created_at_ms: 1, resolver_run_id: "native-presentation-resolve".into(),
        }).unwrap();
        let mut operations = BTreeMap::<ResourceKind, BTreeSet<String>>::new();
        for capability in &snapshot.content().enabled_capabilities {
            for action in &capability.actions {
                for (kind, required) in action.required_resource_operations.as_ref().unwrap() {
                    operations.entry(kind.clone()).or_default().extend(required.iter().cloned());
                }
            }
        }
        let bindings = operations.into_iter().map(|(kind, operations)| TypedResourceBinding {
            binding_id: format!("presentation-{}", kind.as_ref()).into(), resource_kind: kind,
            resource_id: "presentation-resource".into(), owner_id: owner.principal_id.clone(),
            operations, connection_config_ref: None, typed_parameters: BTreeMap::new(),
        }).collect();
        let snapshot = snapshot.with_target_resource_bindings(&owner, bindings).unwrap();
        let active = SessionCapabilityState::new(&snapshot).snapshot().unwrap();
        // Use production assembly so the canonical union schemas, workspace
        // guidance and platform process descriptions all consume real bytes.
        let original = super::super::agent_tool_surface::compile(&snapshot, &active, &registry,
            &NativeSchemas, &NativeSchemas, &BTreeSet::new()).await.unwrap();
        assert!(serde_json::to_vec(&original.model_definitions()).unwrap().len() > INITIAL_TOOL_BYTES);
        let projected = project(original.clone(), true).unwrap();
        for name in ["read_file", "write_file", "search_files", "poll_process", "exec_command"] {
            assert!(!projected.binding(name).unwrap().definition.deferred, "basic native work must be initially visible: {name}");
        }
        assert!(["apply_patch", "start_process"].iter().any(|name|
            projected.binding(name).unwrap().definition.deferred), "heavy native definitions must leave room for basic file work");
        let visible = projected.model_definitions().into_iter().filter(|definition| !definition.deferred).collect::<Vec<_>>();
        assert!(visible.len() <= INITIAL_VISIBLE_TOOLS);
        assert!(serde_json::to_vec(&visible).unwrap().len() <= INITIAL_TOOL_BYTES);
        assert_eq!(original.len(), projected.len());
        for definition in original.model_definitions() {
            let before = original.binding(&definition.name).unwrap();
            let mut after = projected.binding(&definition.name).unwrap().clone();
            after.definition.deferred = before.definition.deferred;
            assert_eq!(&after, before, "presentation changed the native binding or schema for {}", definition.name);
        }
    }
}
