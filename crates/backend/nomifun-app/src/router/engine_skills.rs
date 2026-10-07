//! Immutable Skill data projected from the canonical Agent Snapshot.
//! Library content is host-captured data; package Skills retain exact Registry
//! provenance. Neither source grants tools, runs scripts, or opens mutable paths.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use nomifun_agent_contracts::{CapabilityId, ContributionSourceKind, FrozenSkillContent,
    PluginSourceKind, ResolvedPackageSkillLock, ResolvedSkillLock, digest_bytes};
use nomifun_agent_kernel::{CompiledSnapshot, MaterializedRegistry, MaterializedSkill};
use nomifun_common::AppError;
use nomifun_engine_core::{EngineContextContent, EngineContextResource};
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
    pub fn ids(&self) -> &BTreeSet<String> { &self.ids }
    pub fn instructions(&self) -> &[String] { &self.instructions }
    pub fn resources(&self) -> &Arc<BTreeMap<String, EngineContextResource>> { &self.resources }

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

pub(super) async fn compile(snapshot: &CompiledSnapshot, registry: &MaterializedRegistry) -> Result<SelectedSkills, AppError> {
    if snapshot.registry_generation != registry.generation || snapshot.registry_digest != registry.registry_digest {
        return Err(error("registry changed after Snapshot compilation"));
    }
    let mut selected = SelectedSkills::default();
    let mut resources = BTreeMap::new();
    let mut index = Vec::new();
    for resolved in &snapshot.content().skill_locks {
        let (name, description, body, dependency, provenance, supplemental) = match resolved {
            ResolvedSkillLock::Library { skill, selected: is_selected, .. } => {
                if *is_selected { selected.defaults.insert(skill.name.clone()); }
                skill.validate().map_err(|violation| error(violation.message))?;
                (skill.name.clone(), skill.description.clone(), skill.body.clone(), BTreeSet::new(),
                    format!("library:{}:{}", skill.name, skill.source_digest.as_ref()), Some(&skill.resources))
            }
            ResolvedSkillLock::Package(lock) => {
                if snapshot.content().skill_locks.iter().any(|candidate| matches!(candidate,
                    ResolvedSkillLock::Library { skill, .. } if skill.name == lock.skill.id.as_ref())) { continue; }
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
            skill_locks:vec![ResolvedSkillLock::library_selected(frozen("guide","FROZEN_DEFAULT_BODY"),default_selected),
                ResolvedSkillLock::library(frozen("manual","EXPLICIT_MANUAL_BODY"))],mcp_tool_locks:vec![],resolved_role_providers:BTreeMap::new(),
            canonical_schema_manifest_digest:"d".repeat(64).into(),target_contribution_manifest_digest:"e".repeat(64).into() };
        let envelope = ResolvedSnapshotEnvelope { snapshot_ref:ResolvedSnapshotRef { snapshot_id:"test-snapshot".into(),snapshot_digest:digest_payload(&content).unwrap() },
            content,actor:PrincipalRef { principal_kind:"user".into(),principal_id:"test-owner".into() },scene:"test".into(),surface:"desktop".into(),audience:"user".into(),
            created_at_ms:1,resolver_run_id:"test-resolver".into(),availability_evidence_revision:"test".into() };
        envelope.validate().unwrap();
        let compiled=CompiledSnapshot { envelope,authority_policies:BTreeMap::new(),target_resource_bindings:vec![],registry_generation:registry.generation,registry_digest:registry.registry_digest.clone() };
        compile(&compiled,&registry).await.unwrap()
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
        assert!(none.ids().contains("guide"));
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
}
