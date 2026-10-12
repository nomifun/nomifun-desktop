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
    AgentToolExposure, AgentToolPlan, compile_agent_tool_plan, standard_agent_tool_exposures,
};
use nomifun_common::AppError;
use nomifun_agent_contracts::tool_presentation::{
    namespaced_tool_name, platform_tool_name, relative_action,
};

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
    let mut exposures = standard_agent_tool_exposures();
    retain_exact_platform_actions(&mut exposures, &active.active, &allowed_platform_actions);
    let mut exposures = resource_ready_exposures(snapshot, exposures)?;
    for exposure in &mut exposures {
        let capability = registry
            .capability(&exposure.capability_id)
            .ok_or_else(|| error("tool is not materialized"))?;
        let action = capability
            .manifest
            .contributions
            .actions
            .iter()
            .find(|action| action.action_id == exposure.action_id)
            .ok_or_else(|| error("tool action is unavailable"))?;
        let schema = nomifun_agent_domain_wave2::resolve_action_schema(
            exposure.capability_id.as_ref(),
            &action.input_schema,
        )
        .map_err(error)?;
        // Every standard builtin has a concrete contract. Fail assembly if a
        // future owner accidentally falls back to an open/opaque object again;
        // otherwise the model loses the fields needed to invoke the tool.
        if !concrete_object_schema(
            &schema.0,
            exposure.action_id.as_ref() == "workspace.vcs/status",
        ) {
            return Err(error(format!(
                "{} has no strict canonical tool schema",
                exposure.capability_id.as_ref()
            )));
        }
        exposure.definition.input_schema = standard_presentation_schema(
            exposure.capability_id.as_ref(), schema, &exposure.definition.input_schema,
        );
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
        if action_resources_bound(
            snapshot,
            &exposure.capability_id,
            &exposure.action_id,
        )?
        {
            resource_ready.push(exposure);
        }
    }
    Ok(resource_ready)
}

fn standard_presentation_schema(
    capability_id: &str,
    mut canonical: nomifun_agent_contracts::StrictJsonValue,
    presentation: &nomifun_agent_contracts::StrictJsonValue,
) -> nomifun_agent_contracts::StrictJsonValue {
    // Guidance belongs to this model projection. Changing the registered
    // canonical schema also changes frozen contribution provenance for old
    // Sessions, even when only a description changed.
    let fields: &[&str] = match capability_id {
        "workspace.process" => &[
            "/properties/cmd", "/properties/command", "/properties/args", "/properties/cwd",
            "/properties/timeout_ms", "/properties/cursor", "/properties/wait_ms",
            "/properties/input", "/properties/append_newline", "/properties/tty",
        ],
        "workspace.files" => &[
            "/properties/path", "/properties/content",
            "/properties/files/items/properties/path",
            "/properties/files/items/properties/expected_source",
            "/properties/files/items/properties/expected_source/oneOf/2/properties/sha256",
            "/properties/files/items/properties/hunks/items",
        ],
        _ => &[],
    };
    for pointer in fields {
        if let Some(description) = presentation.0.pointer(pointer)
                .and_then(|property| property.get("description")).and_then(serde_json::Value::as_str)
            && let Some(property) = canonical.0.pointer_mut(pointer).and_then(serde_json::Value::as_object_mut)
        {
            property.insert("description".into(), serde_json::Value::String(description.to_owned()));
        }
    }
    canonical
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

fn retain_exact_platform_actions(
    exposures: &mut Vec<AgentToolExposure>,
    active_modules: &std::collections::BTreeSet<CapabilityId>,
    allowed_actions: &std::collections::BTreeMap<CapabilityId, std::collections::BTreeSet<ActionId>>,
) {
    exposures.retain(|exposure| {
        active_modules.contains(&exposure.capability_id)
            && allowed_actions
                .get(&exposure.capability_id)
                .is_some_and(|allowed| allowed.contains(&exposure.action_id))
    });
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

    fn canonical_process_schema(action_id: &str) -> StrictJsonValue {
        let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
        let capability = registration.metadata.manifest.payload.contributions.capabilities.into_iter()
            .find(|capability| capability.id.as_ref() == "workspace.process").unwrap();
        let reference = &capability.contributions.actions.iter()
            .find(|action| action.action_id.as_ref() == action_id).unwrap().input_schema;
        nomifun_agent_domain_wave2::resolve_action_schema("workspace.process", reference).unwrap()
    }

    fn creative_snapshot(selected: &[(&str, &[&str])]) -> (CompiledSnapshot, std::sync::Arc<MaterializedRegistry>) {
        use nomifun_agent_contracts::*;
        use nomifun_agent_kernel::*;
        let registry = KernelRegistry::new(MaterializationPolicy::stable("1.0.0"),
            std::sync::Arc::new(InMemoryPluginStatePersistence::new())).unwrap();
        let materialized = registry.replace_all(nomifun_agent_domain_wave3::registrations().unwrap()).unwrap();
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
        }).unwrap().with_target_resource_bindings(&owner, vec![TypedResourceBinding {
            binding_id: "creative-library-binding".into(), resource_kind: "asset_library".into(),
            resource_id: nomifun_agent_domain_wave3::CREATIVE_ASSET_LIBRARY_RESOURCE_ID.into(), owner_id: owner.principal_id.clone(),
            operations: BTreeSet::from(["read".into(), "write".into()]), connection_config_ref: None, typed_parameters: BTreeMap::new(),
        }]).unwrap();
        (snapshot, materialized)
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

    #[test]
    fn model_parameter_guidance_survives_canonical_schema_assembly() {
        let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
        let presentations = standard_agent_tool_exposures();
        for (capability_id,action_id,name,pointers) in [
            ("workspace.process","workspace.process/exec","exec_command",vec!["/properties/cmd","/properties/command","/properties/args","/properties/cwd"]),
            ("workspace.files","workspace.files/write","write_file",vec!["/properties/path","/properties/content"]),
            ("workspace.files","workspace.files/patch","apply_patch",vec![
                "/properties/files/items/properties/path",
                "/properties/files/items/properties/expected_source",
                "/properties/files/items/properties/expected_source/oneOf/2/properties/sha256",
                "/properties/files/items/properties/hunks/items",
            ]),
        ] {
            let capability = registration.metadata.manifest.payload.contributions.capabilities.iter()
                .find(|capability|capability.id.as_ref()==capability_id).unwrap();
            let reference = &capability.contributions.actions.iter().find(|action|action.action_id.as_ref()==action_id).unwrap().input_schema;
            let canonical = nomifun_agent_domain_wave2::resolve_action_schema(capability_id,reference).unwrap();
            let presentation = &presentations.iter().find(|tool|tool.definition.name==name).unwrap().definition.input_schema;
            let projected = standard_presentation_schema(capability_id,canonical.clone(),presentation);
            for pointer in pointers {
                let expected = presentation.0.pointer(pointer).unwrap().get("description").unwrap();
                assert_eq!(projected.0.pointer(pointer).unwrap().get("description"),Some(expected),"{name} {pointer}");
            }
            fn strip_descriptions(value:&mut serde_json::Value) {
                match value {
                    serde_json::Value::Object(object) => { object.remove("description"); for child in object.values_mut(){strip_descriptions(child);} }
                    serde_json::Value::Array(array) => {for child in array{strip_descriptions(child);}}
                    _ => {},
                }
            }
            let mut before = canonical.0.clone();
            let mut after = projected.0.clone();
            strip_descriptions(&mut before); strip_descriptions(&mut after);
            assert_eq!(before,after,"{name}: required/default/constraints/branches must stay exact");
            assert_eq!(canonical,nomifun_agent_domain_wave2::resolve_action_schema(capability_id,reference).unwrap());
        }
    }

    #[test]
    fn process_presentation_keeps_cursor_guidance_without_changing_canonical_contract() {
        let canonical = canonical_process_schema("workspace.process/poll");
        let original = canonical.clone();
        let presentation = standard_agent_tool_exposures().into_iter()
            .find(|tool| tool.definition.name == "poll_process").unwrap().definition.input_schema;
        let projected = standard_presentation_schema("workspace.process", canonical, &presentation);
        assert!(projected.0["properties"]["cursor"]["description"].as_str()
            .is_some_and(|value| value.contains("output.next_cursor")));
        assert_eq!(original, canonical_process_schema("workspace.process/poll"));
        let validator = jsonschema::validator_for(&projected.0).unwrap();
        for (args, valid) in [
            (json!({"process_id":"owned"}), true),
            (json!({"process_id":"owned","cursor":0,"wait_ms":30000}), true),
            (json!({"process_id":"owned","cursor":25,"wait_ms":30000}), true),
            (json!({"process_id":"owned","cursor":-1}), false),
            (json!({"process_id":"owned","wait_ms":30001}), false),
            (json!({"process_id":"owned","operation":"poll"}), false),
        ] {
            assert_eq!(validator.is_valid(&args), valid, "{args}");
        }
        let mut structural = projected.0.clone();
        for property in structural["properties"].as_object_mut().unwrap().values_mut() {
            property.as_object_mut().unwrap().remove("description");
        }
        assert_eq!(structural, original.0, "only presentation annotations may differ");
    }

    #[test]
    fn newline_guidance_is_model_only_and_keeps_registered_byte_contracts() {
        for (capability_id, action_id, name, field, expected) in [
            ("workspace.process", "workspace.process/input", "write_process_stdin", "append_newline", "even if input already ends in LF"),
            ("workspace.process", "workspace.process/start", "start_process", "tty", "exec_command still waits"),
            ("workspace.files", "workspace.files/write", "write_file", "content", "no trailing newline is added"),
        ] {
            let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
            let capability = registration.metadata.manifest.payload.contributions.capabilities.into_iter()
                .find(|capability| capability.id.as_ref() == capability_id).unwrap();
            let reference = &capability.contributions.actions.iter()
                .find(|action| action.action_id.as_ref() == action_id).unwrap().input_schema;
            let canonical = nomifun_agent_domain_wave2::resolve_action_schema(capability_id, reference).unwrap();
            let original = canonical.clone();
            let presentation = standard_agent_tool_exposures().into_iter()
                .find(|tool| tool.definition.name == name).unwrap().definition.input_schema;
            let projected = standard_presentation_schema(capability_id, canonical, &presentation);
            assert!(projected.0["properties"][field]["description"].as_str().is_some_and(|text| text.contains(expected)));
            let mut structural = projected.0.clone();
            // Restore only whitelisted descriptions, then require exact
            // identity, including defaults, limits and required fields.
            for property in ["cmd", "command", "args", "cwd", "timeout_ms", "cursor", "wait_ms",
                "input", "append_newline", "content", "path", "tty"] {
                if let Some(value) = structural["properties"].get_mut(property) {
                    if let Some(description) = original.0["properties"][property].get("description") {
                        value["description"] = description.clone();
                    } else { value.as_object_mut().unwrap().remove("description"); }
                }
            }
            assert_eq!(structural, original.0);
            assert_eq!(nomifun_agent_domain_wave2::resolve_action_schema(capability_id, reference).unwrap(), original);
            assert_eq!(standard_presentation_schema("other.module", original.clone(), &presentation), original);
        }
    }

    #[test]
    fn nested_presentation_copies_only_existing_descriptions_and_no_authority() {
        let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
        let capability = registration.metadata.manifest.payload.contributions.capabilities.iter()
            .find(|capability|capability.id.as_ref()=="workspace.files").unwrap();
        let reference = &capability.contributions.actions.iter()
            .find(|action|action.action_id.as_ref()=="workspace.files/patch").unwrap().input_schema;
        let canonical = nomifun_agent_domain_wave2::resolve_action_schema("workspace.files",reference).unwrap();
        let original = canonical.clone();
        let pointer = "/properties/files/items/properties/expected_source/oneOf/2/properties/sha256";
        let mut presentation = canonical.clone();
        presentation.0.pointer_mut(pointer).unwrap()["description"] = json!("Copy the observed full digest.");
        presentation.0.pointer_mut(pointer).unwrap()["pattern"] = json!(".*");
        presentation.0.pointer_mut(pointer).unwrap()["default"] = json!("invented");
        presentation.0["properties"]["files"]["maxItems"] = json!(99999);
        presentation.0["properties"]["files"]["items"]["required"] = json!([]);
        presentation.0["properties"]["files"]["items"]["properties"]["authority"] = json!({"description":"invented permission"});
        let projected = standard_presentation_schema("workspace.files",canonical.clone(),&presentation);
        assert_eq!(projected.0.pointer(pointer).unwrap()["description"],"Copy the observed full digest.");
        let mut restored = projected.clone();
        *restored.0.pointer_mut(pointer).unwrap() = canonical.0.pointer(pointer).unwrap().clone();
        assert_eq!(restored,original,"no defaults, patterns, required, limits or invented authority fields may cross");
        assert_eq!(standard_presentation_schema("unrelated.module",canonical.clone(),&presentation),canonical);
    }

    #[test]
    fn process_presentation_never_copies_defaults_constraints_or_foreign_module_guidance() {
        let canonical = canonical_process_schema("workspace.process/start");
        let presentation = StrictJsonValue(json!({"properties":{
            "timeout_ms":{"description":"Total lifetime, default 30000 ms.","default":600001,"maximum":999999},
            "wait_ms":{"description":false,"maximum":999999},
            "process_id":{"description":"invented field","type":"string"}
        },"required":["invented"]}));
        let projected = standard_presentation_schema("workspace.process", canonical.clone(), &presentation);
        assert_eq!(projected.0["properties"]["timeout_ms"]["maximum"], 600000);
        assert!(projected.0["properties"]["timeout_ms"].get("default").is_none());
        assert_eq!(projected.0["properties"]["wait_ms"], canonical.0["properties"]["wait_ms"]);
        assert!(projected.0["properties"].get("process_id").is_none());
        assert_eq!(projected.0.get("required"), canonical.0.get("required"));
        assert_eq!(standard_presentation_schema("other.module", canonical.clone(), &presentation), canonical);
        assert_eq!(canonical, canonical_process_schema("workspace.process/start"));
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
        for exposure in standard_agent_tool_exposures() {
            assert_eq!(platform_tool_name(exposure.capability_id.as_ref(), exposure.action_id.as_ref()),
                exposure.definition.name);
        }
    }

    #[test]
    fn partial_module_grant_exposes_only_the_exact_allowed_action() {
        let module = CapabilityId::from("workspace.files");
        let mut exposures = standard_agent_tool_exposures();
        retain_exact_platform_actions(
            &mut exposures,
            &BTreeSet::from([module.clone()]),
            &BTreeMap::from([(
                module.clone(),
                BTreeSet::from([ActionId::from("workspace.files/read")]),
            )]),
        );

        assert_eq!(exposures.len(), 1);
        assert_eq!(exposures[0].capability_id, module);
        assert_eq!(exposures[0].action_id.as_ref(), "workspace.files/read");
        assert_eq!(exposures[0].definition.name, "read_file");
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
