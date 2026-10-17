//! Immutable Skill data projected from the canonical Agent Snapshot.
//! Library content is host-captured data; package Skills retain exact Registry
//! provenance. Neither source grants tools, runs scripts, or opens mutable paths.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use nomifun_agent_contracts::{CapabilityId, ContributionSourceKind, FrozenSkillContent,
    PluginSourceKind, ResolvedPackageSkillLock, ResolvedSkillLock, digest_bytes};
use nomifun_agent_kernel::{CompiledSnapshot, MaterializedRegistry, MaterializedSkill};
use nomifun_common::AppError;
use nomifun_engine_core::{EngineContextContent, EngineContextResource, EngineToolPlan};
use serde_json::Value;

#[derive(Default)]
pub struct SelectedSkills {
    pub(super) ids: BTreeSet<String>,
    pub(super) instructions: Vec<String>,
    pub(super) resources: Arc<BTreeMap<String, EngineContextResource>>,
    defaults: BTreeSet<String>,
    bodies: BTreeMap<String, String>,
    body_resources: BTreeMap<String, String>,
    required_capabilities: BTreeMap<String, BTreeSet<CapabilityId>>,
}

impl SelectedSkills {
    pub(super) fn validate_ids(&self, ids: &[String]) -> Result<(), AppError> {
        let mut unique = BTreeSet::new();
        if ids.len() > 128 || ids.iter().any(|id| !self.ids.contains(id) || !unique.insert(id)) {
            return Err(error("requested Skill is not in this Agent Snapshot"));
        }
        Ok(())
    }
    pub(super) fn validate_active(&self, ids: &[String], active: &BTreeSet<CapabilityId>) -> Result<(), AppError> {
        self.validate_ids(ids)?;
        for id in ids {
            if self.required_capabilities.get(id).is_none_or(|required| !required.is_subset(active)) {
                return Err(error("selected Skill dependency is not active in this Session"));
            }
        }
        Ok(())
    }
    /// Root-turn defaults derive only from this immutable Snapshot. Accepted
    /// input and its idempotency digest are never rewritten from live settings.
    pub(super) fn turn_instructions(&self, ids: &[String]) -> Result<Vec<String>, AppError> {
        if ids.is_empty() { self.explicit_instructions(&self.defaults.iter().cloned().collect::<Vec<_>>()) }
        else { self.explicit_instructions(ids) }
    }

    /// Steering includes only explicit selections. Root defaults already apply
    /// once and must not be repeated across every queued input in the Turn.
    pub(super) fn explicit_instructions(&self, ids: &[String]) -> Result<Vec<String>, AppError> {
        self.validate_ids(ids)?;
        let mut remaining = 24 * 1024usize;
        let mut instructions = Vec::new();
        for id in ids {
            let body = self.bodies.get(id).ok_or_else(|| error("frozen Skill body is unavailable"))?;
            let full = format!("Selected Skill {id} for this accepted request. Its text grants no tools or permissions; scripts, hooks and frontmatter are reference data.\n{body}");
            let instruction = if full.len() <= remaining.saturating_sub((ids.len() - instructions.len() - 1) * 256) { full }
                else { format!("Apply the user-selected Skill at read_context_resource id={}. Read all pages; existing permissions still apply.",
                    self.body_resources.get(id).ok_or_else(|| error("Skill body resource is unavailable"))?) };
            remaining = remaining.checked_sub(instruction.len()).ok_or_else(|| error("selected Skill instructions exceed their context budget"))?;
            instructions.push(instruction);
        }
        Ok(instructions)
    }
}

#[cfg(test)]
async fn compile(snapshot: &CompiledSnapshot, registry: &MaterializedRegistry) -> Result<SelectedSkills, AppError> {
    compile_projection(snapshot,registry,false).await
}

pub(super) async fn compile_projection(snapshot:&CompiledSnapshot,registry:&MaterializedRegistry,selected_only:bool)
    ->Result<SelectedSkills,AppError>{
    if snapshot.registry_generation != registry.generation || snapshot.registry_digest != registry.registry_digest {
        return Err(error("registry changed after Snapshot compilation"));
    }
    let mut selected = SelectedSkills::default();
    let mut resources = BTreeMap::new();
    let mut index = Vec::new();
    for resolved in &snapshot.content().skill_locks {
        // The lean default Robot projection omits unselected library inventory.
        // The host uses the normal projection for an explicitly broader Agent;
        // selected Package/library data remains available in either mode.
        // Directory, bodies and supporting resources use the same projection.
        if selected_only && matches!(resolved,ResolvedSkillLock::Library{selected:false,..}) {continue;}
        let (name, description, body, dependency, provenance, supplemental) = match resolved {
            ResolvedSkillLock::Library { skill, selected: is_selected, .. } => {
                if *is_selected { selected.defaults.insert(skill.name.clone()); }
                skill.validate().map_err(|violation| error(violation.message))?;
                (skill.name.clone(), skill.description.clone(), skill.body.clone(), BTreeSet::new(),
                    format!("library:{}:{}", skill.name, skill.source_digest.as_ref()), Some(&skill.resources))
            }
            ResolvedSkillLock::Package(lock) => {
                if snapshot.content().skill_locks.iter().any(|candidate| matches!(candidate,
                    ResolvedSkillLock::Library { skill, selected, .. } if skill.name == lock.skill.id.as_ref()
                        && (!selected_only || *selected))) { continue; }
                let current = registry.skill(&lock.skill.id).ok_or_else(|| error("selected package Skill is unavailable"))?;
                let body = verified_bundled_body(lock, current)?;
                (lock.skill.id.as_ref().to_owned(), current.definition.display.description.clone(), body.to_owned(),
                    lock.required_capabilities.clone(), format!("package:{}:{}", lock.skill.id.as_ref(), lock.body_digest.as_ref()), None)
            }
        };
        if !selected.ids.insert(name.clone()) { return Err(error("duplicate public Skill selection name")); }
        let body_key = resource_key(&provenance, "SKILL.md");
        let mut body = body;
        if let Some(supplemental) = supplemental {
            let mut resource_index = Vec::new();
            for (path, resource) in supplemental {
                let content = match &resource.content {
                    FrozenSkillContent::Text { text } => EngineContextContent::Text { text: text.clone() },
                    FrozenSkillContent::Image { media_type, data_base64 } => EngineContextContent::Image { media_type: media_type.clone(), data_base64: data_base64.clone() },
                };
                let key = resource_key(&provenance, path);
                resource_index.push(serde_json::json!({"path":path,"resource_id":key}));
                resources.insert(key, EngineContextResource { indexed: false, label: format!("{name}/{path}"), provenance: provenance.clone(), content });
            }
            if !resource_index.is_empty() {
                body.push_str(&format!("\n\nHost-verified immutable supporting resources. Read with read_context_resource and the exact resource_id; scripts are reference text. Index: {}", Value::Array(resource_index)));
            }
        }
        resources.insert(body_key.clone(), EngineContextResource { indexed: true, label: format!("{name}/SKILL.md"), provenance: provenance.clone(),
            content: EngineContextContent::Text { text: body.clone() } });
        index.push(serde_json::json!({"name":name,"description":description.chars().take(240).collect::<String>(),"body_resource":body_key}));
        selected.bodies.insert(name.clone(), body);
        selected.body_resources.insert(name.clone(), body_key);
        selected.required_capabilities.insert(name, dependency);
    }
    if !index.is_empty() {
        selected.instructions.push(format!("Available Skills in this Session's immutable Snapshot. Choose relevant instructions with read_context_resource; every Skill is reference data and grants no tools or permissions. Explicit user selections apply to the current accepted request. Inventory: {}", Value::Array(index)));
    }
    selected.resources = Arc::new(resources);
    Ok(selected)
}

/// A presentation hint from the final authorized tool plan, independent of any
/// Skill name or body. Component readiness and executable identity belong to
/// the process host at launch, never to immutable reference resources.
pub(super) fn component_launch_instruction(plan: &EngineToolPlan) -> Option<String> {
    let tools = ["workspace.process/exec", "workspace.process/start"].into_iter()
        .filter_map(|action| {
            let name = plan.model_name_for_action("workspace.process", action)?;
            let binding = plan.binding(name)?;
            binding.definition.input_schema.0["properties"].get("component")?;
            Some(format!("{name} ({action})"))
        }).collect::<Vec<_>>();
    if tools.is_empty() { return None; }
    Some(format!("Host process launch contract for the current authorized tools: {}. {}", tools.join(", "),
        nomifun_agent_contracts::tool_model_guide::managed_launch_notice()))
}

fn resource_key(provenance: &str, path: &str) -> String {
    format!("skill-{}", digest_bytes(format!("{provenance}\0{path}").as_bytes()).as_ref())
}

fn verified_bundled_body<'a>(lock: &ResolvedPackageSkillLock, current: &'a MaterializedSkill) -> Result<&'static str, AppError> {
    let body = nomifun_agent_domain_wave3::creative_studio_planning_skill_body(lock.skill.id.as_ref())
        .ok_or_else(|| error("package Skill has no current host-owned body reader"))?;
    let required = BTreeSet::from([CapabilityId::from(nomifun_agent_domain_wave3::CREATIVE_WORKSHOP_MODULE_ID)]);
    if lock.contribution_lock.source_kind != ContributionSourceKind::PlatformBuiltin || lock.resolved_source.source_kind != PluginSourceKind::Bundled
        || current.definition.package.id.as_ref() != nomifun_agent_domain_wave3::WORKSHOP_PACKAGE_ID
        || current.definition.id != lock.skill.id || current.definition.version != lock.skill.version
        || current.definition.body_ref.digest != lock.body_digest || digest_bytes(body.as_bytes()) != lock.body_digest
        || current.contribution_lock != lock.contribution_lock || current.mount_id != lock.resolved_mount_id
        || current.source != lock.resolved_source || current.target_artifact_digest != lock.target_artifact_digest
        || lock.required_capabilities != required || !current.definition.resources.is_empty() {
        return Err(error("selected package Skill differs from its frozen source lock"));
    }
    Ok(body)
}
fn error(value: impl std::fmt::Display) -> AppError { AppError::Conflict(format!("Agent Skills: {value}")) }

#[cfg(test)]
mod tests {
    use super::*;
    async fn library_snapshot(default_selected: bool) -> SelectedSkills {
        named_library_snapshot(default_selected, "guide").await
    }

    async fn named_library_snapshot(default_selected: bool, name: &str) -> SelectedSkills {
        named_library_snapshot_with_body(default_selected, name, "FROZEN_DEFAULT_BODY").await
    }

    async fn named_library_snapshot_with_body(default_selected: bool, name: &str, body: &str) -> SelectedSkills {
        use nomifun_agent_contracts::{FrozenLibrarySkill, LibrarySkillSource, PrincipalRef, ResolvedSnapshotContent,
            ResolvedSnapshotEnvelope, ResolvedSnapshotRef, RuntimeProfileKind, digest_payload};
        let registry = MaterializedRegistry::empty();
        let frozen = |name: &str, body: &str| FrozenLibrarySkill::new(name.into(), "Frozen test guide".into(),
            LibrarySkillSource::Custom, body.into(), BTreeMap::new()).unwrap();
        let content = ResolvedSnapshotContent { context_order:vec![], middleware_order:vec![], schema_version:"1.0.0".into(), resolver_version:"1.0.0".into(),
            preset_revision_ref:nomifun_agent_contracts::PresetRevisionRef { preset_id:"test-preset".into(),revision:1,revision_digest:"a".repeat(64).into() },
            required_runtime_protocol_version:"1.0.0".into(), required_runtime_profile:RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest:"b".repeat(64).into(),required_runtime_features:BTreeSet::new(),compiled_runtime_profile_digest:"c".repeat(64).into(),
            model_route_refs:BTreeMap::new(),chat_route_identity:None,enabled_capabilities:vec![],required_resource_kinds:BTreeSet::new(),capability_allowlist:BTreeSet::new(),
            skill_locks:vec![ResolvedSkillLock::library_selected(frozen(name,body),default_selected),
                ResolvedSkillLock::library(frozen("manual","EXPLICIT_MANUAL_BODY"))],mcp_tool_locks:vec![],resolved_role_providers:BTreeMap::new(),
            canonical_schema_manifest_digest:"d".repeat(64).into(),target_contribution_manifest_digest:"e".repeat(64).into() };
        let envelope = ResolvedSnapshotEnvelope { snapshot_ref:ResolvedSnapshotRef { snapshot_id:"test-snapshot".into(),snapshot_digest:digest_payload(&content).unwrap() },
            content,actor:PrincipalRef { principal_kind:"user".into(),principal_id:"test-owner".into() },scene:"test".into(),surface:"desktop".into(),audience:"user".into(),
            created_at_ms:1,resolver_run_id:"test-resolver".into(),availability_evidence_revision:"test".into() };
        envelope.validate().unwrap();
        let compiled=CompiledSnapshot { envelope,authority_policies:BTreeMap::new(),target_resource_bindings:vec![],registry_generation:registry.generation,registry_digest:registry.registry_digest.clone(),non_executable_capability_ids:Default::default() };
        compile(&compiled,&registry).await.unwrap()
    }


    #[tokio::test]
    async fn custom_office_skill_name_does_not_bind_or_prepare_a_host_component() {
        let before = nomifun_runtime::tool_preparation_snapshot(nomifun_runtime::ToolId::OfficeCli);
        let selected = named_library_snapshot(true, "officecli-docx").await;
        let after = nomifun_runtime::tool_preparation_snapshot(nomifun_runtime::ToolId::OfficeCli);
        assert_eq!(before, after, "Skill projection must not prepare or probe Office");
        assert!(selected.validate_active(&["officecli-docx".into()], &BTreeSet::new()).is_ok(),
            "reference Skills must not add process, file, or capability permissions");
        assert!(selected.required_capabilities["officecli-docx"].is_empty());
        assert_eq!(selected.resources.len(), 2);
        assert!(selected.resources.values().all(|resource| resource.provenance.starts_with("library:")));
        let body = &selected.bodies["officecli-docx"];
        assert_eq!(body, "FROZEN_DEFAULT_BODY");
        let resource = &selected.resources[&selected.body_resources["officecli-docx"]];
        let EngineContextContent::Text { text } = &resource.content else { panic!("frozen Skill body must be text"); };
        assert_eq!(text, body);
        assert_eq!(digest_bytes(text.as_bytes()), digest_bytes(b"FROZEN_DEFAULT_BODY"));
    }

    fn canonical_process_schema(action_id: &str) -> nomifun_agent_contracts::StrictJsonValue {
        let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
        let capability = registration.metadata.manifest.payload.contributions.capabilities.into_iter()
            .find(|capability| capability.id.as_ref() == "workspace.process").unwrap();
        let reference = &capability.contributions.actions.iter()
            .find(|action| action.action_id.as_ref() == action_id).unwrap().input_schema;
        nomifun_agent_domain_wave2::resolve_action_schema("workspace.process", reference).unwrap()
    }

    fn process_plan(action: &str, name: &str, component_schema: bool) -> EngineToolPlan {
        use nomifun_agent_contracts::StrictJsonValue;
        use nomifun_chat_model_broker::ChatToolDefinition;
        use nomifun_engine_core::{EngineEffectClass, EngineToolBinding, input_schema_digest};
        let mut input_schema = canonical_process_schema(action);
        if !component_schema {
            input_schema.0["properties"].as_object_mut().unwrap().remove("component");
        }
        EngineToolPlan::new([EngineToolBinding {
            model_name:name.into(), definition:ChatToolDefinition {name:name.into(),description:"Authorized process action".into(),input_schema:StrictJsonValue(input_schema.0.clone()),deferred:true},
            schema_digest:input_schema_digest(&input_schema).unwrap(),canonical_input_schema_ref:"schema://test/process".into(),
            capability_contract_digest:"b".repeat(64).into(),capability_id:"workspace.process".into(),action_id:action.into(),
            resource_binding_ids:BTreeSet::new(),effect_class:EngineEffectClass::ManagedEffect,parallel_safe:false,
        }]).unwrap()
    }

    #[test]
    fn component_launch_guidance_uses_only_current_authorized_launch_schemas() {
        assert!(component_launch_instruction(&EngineToolPlan::default()).is_none());
        assert!(component_launch_instruction(&process_plan("workspace.process/poll", "read_process", true)).is_none());
        assert!(component_launch_instruction(&process_plan("workspace.process/exec", "old_exec", false)).is_none());
        let plan = process_plan("workspace.process/start", "current_launch", true);
        let instruction = component_launch_instruction(&plan).unwrap();
        assert!(instruction.contains("current_launch (workspace.process/start)"));
        assert!(instruction.contains("component=officecli"));
        assert!(instruction.contains("takes precedence over Skill reference text"));
        assert!(!instruction.contains("old_exec"));
        assert!(instruction.contains("grants no tools or permissions"));
    }

    #[test]
    fn process_component_schema_matches_the_approved_supplier_catalog() {
        let schema = canonical_process_schema("workspace.process/exec");
        let approved = schema.0["properties"]["component"]["enum"].as_array().unwrap().iter()
            .map(|value| value.as_str().unwrap().to_owned()).collect::<BTreeSet<_>>();
        let inventory = nomifun_runtime::component_inventory();
        let supplied = inventory["components"].as_array().unwrap().iter()
            .map(|value| value["id"].as_str().unwrap().to_owned()).collect::<BTreeSet<_>>();
        assert_eq!(approved, supplied, "process Schema must track approved component IDs without linking the pure domain to the supplier");
    }

    #[tokio::test]
    async fn component_entry_is_visible_without_reading_a_large_skill_body() {
        let body = format!("{}\nLAST_PAGE_REFERENCE", "Long frozen reference text.\n".repeat(1800));
        let skills = named_library_snapshot_with_body(true, "officecli-docx", &body).await;
        let selected = skills.turn_instructions(&[]).unwrap();
        assert!(selected[0].contains("read_context_resource"));
        assert!(!selected[0].contains("LAST_PAGE_REFERENCE"));
        assert_eq!(skills.bodies["officecli-docx"], body);
        let plan = process_plan("workspace.process/exec", "managed_exec", true);
        let instruction = component_launch_instruction(&plan).unwrap();
        assert!(instruction.contains("managed_exec"));
        assert!(instruction.contains("component=officecli"));
        assert!(!instruction.contains(&skills.body_resources["officecli-docx"]));
    }

    #[tokio::test]
    async fn root_defaults_use_the_frozen_snapshot_without_rewriting_accepted_selection() {
        let frozen=library_snapshot(true).await;
        let accepted_ids=Vec::new();
        let instructions=frozen.turn_instructions(&accepted_ids).unwrap();
        assert!(instructions.iter().any(|text|text.contains("FROZEN_DEFAULT_BODY")));
        assert!(accepted_ids.is_empty());
        assert!(frozen.explicit_instructions(&accepted_ids).unwrap().is_empty(),"steering must not repeat root defaults");
        let explicit=frozen.turn_instructions(&["manual".into()]).unwrap();
        assert!(explicit.iter().any(|text|text.contains("EXPLICIT_MANUAL_BODY")));
        assert!(explicit.iter().all(|text|!text.contains("FROZEN_DEFAULT_BODY")));
    }

    #[tokio::test]
    async fn an_explicit_none_snapshot_keeps_inventory_readable_without_default_injection() {
        let none=library_snapshot(false).await;
        assert!(none.turn_instructions(&[]).unwrap().is_empty());
        assert!(none.ids.contains("guide"));
        assert!(none.explicit_instructions(&["guide".into()]).unwrap()[0].contains("FROZEN_DEFAULT_BODY"));
        let selected=library_snapshot(true).await;
        assert!(selected.turn_instructions(&[]).unwrap()[0].contains("FROZEN_DEFAULT_BODY"));
        assert!(none.turn_instructions(&[]).unwrap().is_empty(),"another Snapshot cannot change these defaults");
    }

    #[test]
    fn current_package_skill_body_still_requires_its_exact_registry_provenance() {
        use nomifun_agent_kernel::{InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy};
        let kernel = KernelRegistry::new(MaterializationPolicy::stable(nomifun_agent_domain_wave3::CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new())).unwrap();
        let registry = kernel.replace_all(nomifun_agent_domain_wave3::registrations().unwrap()).unwrap();
        let current = registry.skill(&nomifun_agent_contracts::SkillId::from("creative-studio-canvas")).unwrap();
        let mut lock = ResolvedPackageSkillLock { skill: nomifun_agent_contracts::SkillRef { id:current.definition.id.clone(), version:current.definition.version.clone() },
            body_digest:current.definition.body_ref.digest.clone(), required_capabilities:current.definition.requires_capabilities.iter().map(|value|value.id.clone()).collect(),
            contribution_lock:current.contribution_lock.clone(), resolved_mount_id:current.mount_id.clone(), resolved_source:current.source.clone(),target_artifact_digest:current.target_artifact_digest.clone() };
        assert!(verified_bundled_body(&lock,current).unwrap().contains("Creative Studio Canvas"));
        lock.body_digest = "0".repeat(64).into();assert!(verified_bundled_body(&lock,current).is_err());
        lock.body_digest = current.definition.body_ref.digest.clone();lock.required_capabilities.clear();
        assert!(verified_bundled_body(&lock,current).is_err());
    }
    #[test]
    fn selecting_the_entire_bounded_inventory_still_fits_the_turn_instruction_budget() {
        let mut skills = SelectedSkills::default(); let mut names = Vec::new();
        for number in 0..128 {
            let name = format!("{number:03}{}", "x".repeat(125));
            skills.ids.insert(name.clone()); skills.bodies.insert(name.clone(), "large body".repeat(12000));
            skills.body_resources.insert(name.clone(), format!("skill-{}", "a".repeat(64))); names.push(name);
        }
        let instructions = skills.turn_instructions(&names).unwrap();
        assert_eq!(instructions.len(),128);
        assert!(instructions.iter().map(String::len).sum::<usize>() <= 24 * 1024);
    }
    #[test]
    fn selections_are_bounded_by_snapshot_and_large_bodies_remain_readable() {
        let mut skills = SelectedSkills::default();
        for name in ["first", "second"] {
            skills.ids.insert(name.into()); skills.required_capabilities.insert(name.into(), BTreeSet::new());
            skills.bodies.insert(name.into(), "x".repeat(16 * 1024)); skills.body_resources.insert(name.into(), format!("body-{name}"));
        }
        assert!(skills.validate_ids(&["unknown".into()]).is_err());
        assert!(skills.validate_ids(&["first".into(), "first".into()]).is_err());
        let instructions = skills.turn_instructions(&["first".into(), "second".into()]).unwrap();
        assert_eq!(instructions.len(), 2);
        assert!(instructions[1].contains("body-second"));
        assert!(instructions.iter().map(String::len).sum::<usize>() <= 24 * 1024);
        assert!(skills.validate_active(&["first".into()], &BTreeSet::new()).is_ok());
    }

    mod robot_projection {
        use super::*;
        use async_trait::async_trait;
        use nomifun_agent_contracts::{
            FrozenLibrarySkill, FrozenSkillResource, LibrarySkillSource, PrincipalRef,
            ResolvedSnapshotContent, ResolvedSnapshotEnvelope, ResolvedSnapshotRef,
            RuntimeProfileKind, StrictJsonValue, digest_payload,
        };
        use nomifun_agent_runtime::{
            AgentEngine, AgentEngineBuild, AgentEngineError, AgentEngineEvent, AgentEventSink,
            AgentModelPort, AgentModelStream, AgentToolInvocation, AgentToolInvoker,
            AgentToolResult, AgentTurnRequest, AgentTurnTerminal,
        };
        use nomifun_chat_model_broker::{
            ChatContentPart, ChatFinishReason, ChatMessage, ChatModelError, ChatModelEvent,
            ChatModelRequest, ChatRole, ChatToolCall, ChatToolChoice,
        };
        use std::sync::Mutex;
        use tokio_util::sync::CancellationToken;

        const HIDDEN_BODY: &str = "UNSELECTED_FROZEN_DESKTOP_BODY";
        const SUPPORT_BODY: &str = "EXPLICIT_ROBOT_SUPPORTING_REFERENCE";

        fn resolved_capability(id: &CapabilityId, registry: &MaterializedRegistry) -> nomifun_agent_contracts::ResolvedCapability {
                let current = registry.capability(id).expect("the package dependency is genuinely materialized");
                assert!(current.manifest.requires.is_empty(), "this fixture uses independent materialized capabilities");
                nomifun_agent_contracts::ResolvedCapability {
                    consumption: Default::default(), dependency_refs: Vec::new(),
                    capability: nomifun_agent_contracts::CapabilityRef { id: id.clone() },
                    source_package: current.manifest.package.clone(), contribution_id: current.contribution_id.clone(),
                    contribution_lock: current.contribution_lock.clone(), resolved_mount_id: Some(current.mount_id.clone()),
                    resolved_source: current.source.clone(), target_artifact_digest: current.target_artifact_digest.clone(),
                    schema_digest: current.schema_digest.clone(), dependency_path: vec![id.clone()],
                    required_runtime_features: current.manifest.requires_runtime_features.iter().map(|feature| feature.id.clone()).collect(),
                    display_name: None, description: None, actions: current.manifest.contributions.actions.clone(),
                    required_resource_kinds: current.manifest.contributions.resource_kinds.clone(),
                    action_allowlist: current.manifest.contributions.actions.iter().map(|action| action.action_id.clone()).collect(),
                }
        }

        fn snapshot(locks: Vec<ResolvedSkillLock>, registry: &MaterializedRegistry) -> CompiledSnapshot {
            let capability_allowlist: BTreeSet<CapabilityId> = locks.iter().filter_map(|lock| match lock {
                ResolvedSkillLock::Package(lock) => Some(lock.required_capabilities.iter().cloned()),
                _ => None,
            }).flatten().collect();
            let enabled_capabilities = capability_allowlist.iter().map(|id| resolved_capability(id, registry)).collect();
            let content = ResolvedSnapshotContent {
                context_order: vec![], middleware_order: vec![], schema_version: "1.0.0".into(),
                resolver_version: "1.0.0".into(),
                preset_revision_ref: nomifun_agent_contracts::PresetRevisionRef {
                    preset_id: "frozen-robot-skills".into(), revision: 1,
                    revision_digest: "a".repeat(64).into(),
                },
                required_runtime_protocol_version: "1.0.0".into(),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: "b".repeat(64).into(),
                required_runtime_features: BTreeSet::new(),
                compiled_runtime_profile_digest: "c".repeat(64).into(),
                model_route_refs: BTreeMap::new(), chat_route_identity: None,
                enabled_capabilities, required_resource_kinds: BTreeSet::new(),
                capability_allowlist, skill_locks: locks, mcp_tool_locks: vec![],
                resolved_role_providers: BTreeMap::new(),
                canonical_schema_manifest_digest: "d".repeat(64).into(),
                target_contribution_manifest_digest: "e".repeat(64).into(),
            };
            let envelope = ResolvedSnapshotEnvelope {
                snapshot_ref: ResolvedSnapshotRef { snapshot_id: "frozen-robot-skill-snapshot".into(),
                    snapshot_digest: digest_payload(&content).unwrap() },
                content, actor: PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() },
                scene: "test".into(), surface: "desktop".into(), audience: "user".into(),
                created_at_ms: 1, resolver_run_id: "skill-projection-fixture".into(),
                availability_evidence_revision: "fixture".into(),
            };
            envelope.validate().unwrap();
            CompiledSnapshot { envelope, authority_policies: BTreeMap::new(),
                target_resource_bindings: vec![], registry_generation: registry.generation,
                registry_digest: registry.registry_digest.clone(), non_executable_capability_ids: Default::default() }
        }

        fn locks(selected: usize) -> Vec<ResolvedSkillLock> {
            (0..12).map(|number| {
                let chosen = number < selected;
                let skill = FrozenLibrarySkill::new(format!("frozen-guide-{number:02}"),
                    format!("Frozen directory reference {number}"), LibrarySkillSource::Custom,
                    if chosen { format!("EXPLICIT_ROBOT_BODY_{number}") } else { HIDDEN_BODY.repeat(600) },
                    BTreeMap::from([("references/guide.md".into(), FrozenSkillResource::new(
                        FrozenSkillContent::Text { text: if chosen { SUPPORT_BODY.into() }
                            else { format!("UNSELECTED_SUPPORT_{number}") } }).unwrap())]),
                ).unwrap();
                ResolvedSkillLock::library_selected(skill, chosen)
            }).collect()
        }

        fn budgets(label: &str, skills: &SelectedSkills) -> (usize, usize, usize) {
            let directory_bytes = skills.instructions.iter().map(String::len).sum::<usize>();
            let resource_bytes = skills.resources.values().map(|resource| match &resource.content {
                EngineContextContent::Text { text } => text.len(),
                EngineContextContent::Image { data_base64, .. } => data_base64.len(),
            }).sum::<usize>();
            eprintln!("{label}: directory_utf8_bytes={directory_bytes}, resources={}, resource_bytes={resource_bytes}", skills.resources.len());
            (directory_bytes, skills.resources.len(), resource_bytes)
        }

        #[tokio::test]
        async fn old_frozen_lean_robot_inventory_projection_omits_unselected_directory_and_readable_bytes() {
            let registry = MaterializedRegistry::empty();
            for chosen in [0, 2] {
                let frozen = snapshot(locks(chosen), &registry);
                let original = serde_json::to_vec(&frozen.envelope).unwrap();
                let ordinary = compile(&frozen, &registry).await.unwrap();
                let robot = compile_projection(&frozen, &registry, true).await.unwrap();
                let (ordinary_directory, ordinary_count, ordinary_bytes) = budgets("ordinary", &ordinary);
                let (robot_directory, robot_count, robot_bytes) = budgets("lean Robot", &robot);
                assert_eq!(ordinary.ids.len(), 12);
                assert_eq!(ordinary_count, 24);
                assert!(ordinary_directory > 2048 && ordinary_bytes > 100 * 1024);
                assert_eq!(robot.ids.len(), chosen);
                assert_eq!(robot_count, chosen * 2);
                if chosen == 0 {
                    assert_eq!((robot_directory, robot_bytes), (0, 0));
                    assert!(robot.turn_instructions(&[]).unwrap().is_empty());
                } else {
                    assert!(robot_directory < 1024 && robot_bytes < 2048);
                    assert_eq!(robot.turn_instructions(&[]).unwrap().len(), chosen);
                    for number in 0..chosen {
                        let name = format!("frozen-guide-{number:02}");
                        assert_eq!(robot.bodies[&name], ordinary.bodies[&name]);
                        assert!(robot.bodies[&name].contains("references/guide.md"));
                        assert!(robot.resources.values().any(|resource|
                            resource.label == format!("{name}/references/guide.md")
                                && matches!(&resource.content, EngineContextContent::Text { text } if text == SUPPORT_BODY)));
                    }
                }
                assert!(robot.resources.values().all(|resource| match &resource.content {
                    EngineContextContent::Text { text } => !text.contains(HIDDEN_BODY) && !text.contains("UNSELECTED_SUPPORT_"),
                    EngineContextContent::Image { .. } => false,
                }));
                assert!(!robot.instructions.join("\n").contains("frozen-guide-11"));
                assert_eq!(serde_json::to_vec(&frozen.envelope).unwrap(), original,
                    "the selected-only runtime projection must never rewrite old frozen locks");
            }
        }

        #[tokio::test]
        async fn explicit_package_skill_survives_a_same_name_unselected_library_lock() {
            use nomifun_agent_kernel::{InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy};
            let kernel = KernelRegistry::new(MaterializationPolicy::stable(nomifun_agent_domain_wave3::CONTRACT_VERSION),
                Arc::new(InMemoryPluginStatePersistence::new())).unwrap();
            let registry = kernel.replace_all(nomifun_agent_domain_wave3::registrations().unwrap()).unwrap();
            let current = registry.skill(&nomifun_agent_contracts::SkillId::from("creative-studio-canvas")).unwrap();
            let lock = ResolvedPackageSkillLock {
                skill: nomifun_agent_contracts::SkillRef { id: current.definition.id.clone(), version: current.definition.version.clone() },
                body_digest: current.definition.body_ref.digest.clone(),
                required_capabilities: current.definition.requires_capabilities.iter().map(|item| item.id.clone()).collect(),
                contribution_lock: current.contribution_lock.clone(), resolved_mount_id: current.mount_id.clone(),
                resolved_source: current.source.clone(), target_artifact_digest: current.target_artifact_digest.clone(),
            };
            let alias = FrozenLibrarySkill::new("creative-studio-canvas".into(), "Unselected library alias".into(),
                LibrarySkillSource::Custom, HIDDEN_BODY.into(), BTreeMap::new()).unwrap();
            let frozen = snapshot(vec![ResolvedSkillLock::Package(lock), ResolvedSkillLock::library(alias)], &registry);
            let original = serde_json::to_vec(&frozen.envelope).unwrap();
            let ordinary = compile(&frozen, &registry).await.unwrap();
            assert_eq!(ordinary.bodies["creative-studio-canvas"], HIDDEN_BODY,
                "ordinary Agents keep the existing library-alias preference");
            let robot = compile_projection(&frozen, &registry, true).await.unwrap();
            assert_eq!(robot.ids.len(), 1);
            assert!(robot.bodies["creative-studio-canvas"].contains("Creative Studio Canvas"));
            assert!(!robot.bodies["creative-studio-canvas"].contains(HIDDEN_BODY));
            assert!(robot.resources.values().all(|resource| resource.provenance.starts_with("package:")));
            assert_eq!(serde_json::to_vec(&frozen.envelope).unwrap(), original);
        }

        struct ResourceProbeModel {
            target: String,
            requests: Mutex<Vec<ChatModelRequest>>,
        }

        #[async_trait]
        impl AgentModelPort for ResourceProbeModel {
            async fn open_stream(&self, request: ChatModelRequest, _: CancellationToken)
                -> Result<AgentModelStream, ChatModelError> {
                let mut requests = self.requests.lock().unwrap();
                let step = requests.len();
                requests.push(request);
                let events = match step {
                    0 => vec![Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                        call_id: "probe-unselected-body".into(), name: "read_context_resource".into(),
                        arguments: StrictJsonValue(serde_json::json!({"id": self.target, "limit": 4096})),
                        provider_metadata: None,
                    } }), Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls })],
                    1 => vec![Ok(ChatModelEvent::OutputTextDelta { text: "The reference read was checked.".into(), source: None }),
                        Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed })],
                    _ => panic!("one resource probe unexpectedly needed another model request"),
                };
                Ok(Box::pin(futures_util::stream::iter(events)))
            }
        }

        struct NoExternalTools;

        #[async_trait]
        impl AgentToolInvoker for NoExternalTools {
            async fn invoke(&self, _: AgentToolInvocation, _: CancellationToken) -> Result<AgentToolResult, AgentEngineError> {
                panic!("an immutable Skill resource probe must not dispatch a platform tool");
            }
        }

        #[derive(Default)]
        struct ProbeEvents(Mutex<Vec<AgentEngineEvent>>);

        #[async_trait]
        impl AgentEventSink for ProbeEvents {
            async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
                self.0.lock().unwrap().push(event);
                Ok(())
            }
        }

        async fn run_resource_probe(skills: &SelectedSkills, target: &str)
            -> (Arc<ResourceProbeModel>, AgentToolResult) {
            let engine = AgentEngine::new(AgentEngineBuild {
                build_id: "selected-skill-projection".into(), build_digest: "a".repeat(64).into(),
            }).unwrap();
            let mut request = nomifun_chat_model_broker::recorded_conformance_fixtures().remove(0).request;
            let binding = engine.bind(request.causality.agent_session_id.clone(), "selected-skill-binding".into(),
                request.causality.resolved_snapshot_ref.clone()).unwrap();
            request.input.instructions = skills.instructions.clone();
            request.input.instructions.extend(skills.turn_instructions(&[]).unwrap());
            request.input.messages = vec![ChatMessage { role: ChatRole::User,
                content: vec![ChatContentPart::Text { text: "Check the requested immutable reference.".into() }], provider_round_id: None }];
            request.input.tools.clear();
            request.input.tool_choice = ChatToolChoice::Auto;
            let model = Arc::new(ResourceProbeModel { target: target.into(), requests: Mutex::new(Vec::new()) });
            let events = Arc::new(ProbeEvents::default());
            let session = engine.open_session(binding, model.clone(), Arc::new(NoExternalTools), Some(events.clone())).unwrap();
            let result = session.run_turn(AgentTurnRequest::new(request, EngineToolPlan::default(),
                PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() }, 0)
                .with_context_resources(skills.resources.clone()).with_max_model_steps(4)).await.unwrap();
            assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
            assert_eq!(model.requests.lock().unwrap().len(), 2);
            let read = events.0.lock().unwrap().iter().find_map(|event| match event {
                AgentEngineEvent::ToolCompleted { result, .. } if result.call_id.as_ref() == "probe-unselected-body" => Some(result.clone()),
                _ => None,
            }).expect("the real runtime must record the native resource-read result");
            (model, read)
        }

        #[tokio::test]
        async fn lean_robot_runtime_rejects_a_known_unselected_resource_id_while_ordinary_reader_can_read_it() {
            let registry = MaterializedRegistry::empty();
            let frozen = snapshot(locks(1), &registry);
            let ordinary = compile(&frozen, &registry).await.unwrap();
            let robot = compile_projection(&frozen, &registry, true).await.unwrap();
            let known_unselected_id = ordinary.body_resources["frozen-guide-11"].clone();
            assert!(!robot.resources.contains_key(&known_unselected_id));
            let (robot_model, robot_read) = run_resource_probe(&robot, &known_unselected_id).await;
            assert!(robot_read.is_error);
            assert!(robot_read.output_text().contains("Unknown context resource"));
            let robot_requests = robot_model.requests.lock().unwrap();
            assert!(robot_requests[0].input.tools.iter().any(|tool| tool.name == "read_context_resource"),
                "the probe exercises resource membership, not absence of the reader schema");
            let robot_data = serde_json::to_string(&*robot_requests).unwrap();
            assert!(!robot_data.contains(HIDDEN_BODY));
            assert!(!robot_data.contains("frozen-guide-11"));
            assert!(robot_data.contains("Unknown context resource"));
            drop(robot_requests);

            let (ordinary_model, ordinary_read) = run_resource_probe(&ordinary, &known_unselected_id).await;
            assert!(!ordinary_read.is_error);
            assert!(ordinary_read.output_text().contains(HIDDEN_BODY));
            assert!(serde_json::to_string(&*ordinary_model.requests.lock().unwrap()).unwrap().contains(HIDDEN_BODY),
                "the same known immutable ID remains readable through an ordinary Session's actual native reader");
        }

        #[tokio::test]
        async fn explicitly_expanded_robot_uses_normal_skill_discovery_and_can_read_the_known_resource() {
            use nomifun_agent_kernel::{InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy};
            let kernel = KernelRegistry::new(MaterializationPolicy::stable(nomifun_agent_domain_wave4::CONTRACT_VERSION),
                Arc::new(InMemoryPluginStatePersistence::new())).unwrap();
            let registry = kernel.replace_all(vec![
                nomifun_agent_domain_wave4::companion_registration().unwrap(),
                nomifun_agent_domain_wave1::companion_memory_registration().unwrap(),
                nomifun_agent_domain_wave4::robot_registration().unwrap(),
                nomifun_agent_domain_wave2::workspace_execution_registration().unwrap(),
            ]).unwrap();
            let mut frozen = snapshot(locks(1), &registry);
            for id in ["companion", "companion.memory", "robot"] {
                frozen.envelope.content.enabled_capabilities.push(resolved_capability(&id.into(), &registry));
            }
            frozen.envelope.content.capability_allowlist = frozen.content().enabled_capabilities.iter()
                .map(|capability| capability.capability.id.clone()).collect();
            frozen.envelope.content.required_resource_kinds = frozen.content().enabled_capabilities.iter()
                .flat_map(|capability| capability.required_resource_kinds.iter().cloned()).collect();
            frozen.envelope.snapshot_ref.snapshot_digest = digest_payload(frozen.content()).unwrap();
            frozen.envelope.validate().unwrap();
            assert!(crate::router::unified_runtime_host::robot_interaction::uses_lean_robot_context(true, frozen.content()),
                "the default narrow physical profile qualifies for the lean projection");
            frozen.envelope.content.enabled_capabilities.push(resolved_capability(&"workspace.files".into(), &registry));
            frozen.envelope.content.capability_allowlist = frozen.content().enabled_capabilities.iter()
                .map(|capability| capability.capability.id.clone()).collect();
            frozen.envelope.content.required_resource_kinds = frozen.content().enabled_capabilities.iter()
                .flat_map(|capability| capability.required_resource_kinds.iter().cloned()).collect();
            frozen.envelope.snapshot_ref.snapshot_digest = digest_payload(frozen.content()).unwrap();
            frozen.envelope.validate().unwrap();
            let original = serde_json::to_vec(&frozen.envelope).unwrap();
            let selected_only = crate::router::unified_runtime_host::robot_interaction::uses_lean_robot_context(true, frozen.content());
            assert!(!selected_only, "an explicitly selected desktop capability is not a lean physical profile");
            let expanded = compile_projection(&frozen, &registry, selected_only).await.unwrap();
            let ordinary = compile(&frozen, &registry).await.unwrap();
            assert_eq!(budgets("explicitly expanded Robot", &expanded), budgets("ordinary", &ordinary));
            assert_eq!(expanded.ids.len(), 12);
            assert_eq!(expanded.resources.len(), 24);
            let known_id = ordinary.body_resources["frozen-guide-11"].clone();
            let (model, read) = run_resource_probe(&expanded, &known_id).await;
            assert!(!read.is_error);
            assert!(read.output_text().contains(HIDDEN_BODY));
            assert!(serde_json::to_string(&*model.requests.lock().unwrap()).unwrap().contains(HIDDEN_BODY),
                "the Robot role does not permanently hide the normal directory or its readable resources");
            assert_eq!(serde_json::to_vec(&frozen.envelope).unwrap(), original);
        }
    }
}
