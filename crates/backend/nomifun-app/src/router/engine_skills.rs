//! Agent Skill projection for the current immutable Agent snapshot.
//!
//! The Creative Studio planning Skills are bundled from the Skill Library's
//! actual SKILL.md files into the Workshop package. Their exact body digests
//! must match both the Snapshot and the current Registry before a turn can use
//! them. Historical Plugin package Skill locks remain unavailable.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_contracts::{
    CapabilityId, ContributionSourceKind, PluginSourceKind, ResolvedSkillLock, SkillId,
    digest_bytes,
};
use nomifun_agent_kernel::{CompiledSnapshot, MaterializedRegistry, MaterializedSkill};
use nomifun_common::AppError;
use nomifun_engine_core::EngineContextResource;
use serde_json::Value;

#[derive(Default)]
pub struct SelectedSkills {
    pub(super) ids: BTreeSet<String>,
    pub(super) instructions: Vec<String>,
    pub(super) resources: Arc<BTreeMap<String, EngineContextResource>>,
    pub(super) commands: Vec<nomifun_ai_agent::plugin_skills::NomiVerifiedSkillCommand>,
    bundled_bodies: BTreeMap<String, String>,
    required_capabilities: BTreeMap<String, BTreeSet<CapabilityId>>,
}

impl SelectedSkills {
    pub fn ids(&self) -> &BTreeSet<String> {
        &self.ids
    }

    pub fn instructions(&self) -> &[String] {
        &self.instructions
    }

    pub fn resources(&self) -> &Arc<BTreeMap<String, EngineContextResource>> {
        &self.resources
    }

    pub(super) fn validate_extra(&self, extra: &Value) -> Result<(), AppError> {
        for key in ["skills", "session_enabled_skills"] {
            if let Some(value) = extra.get(key) {
                let ids: Vec<String> = serde_json::from_value(value.clone()).map_err(error)?;
                self.validate_ids(&ids)?;
            }
        }
        Ok(())
    }

    pub(super) fn validate_ids(&self, ids: &[String]) -> Result<(), AppError> {
        let mut unique = BTreeSet::new();
        if ids.len() > 16
            || ids
                .iter()
                .any(|id| !self.ids.contains(id) || !unique.insert(id))
        {
            return Err(error(
                "requested Skill is not in the Agent's immutable selected Skill locks",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_active(
        &self,
        ids: &[String],
        active: &BTreeSet<CapabilityId>,
    ) -> Result<(), AppError> {
        self.validate_ids(ids)?;
        for id in ids {
            if self
                .required_capabilities
                .get(id)
                .is_none_or(|required| !required.is_subset(active))
            {
                return Err(error("selected Skill dependency is not active in this Session"));
            }
        }
        Ok(())
    }

    /// Only the accepted turn's chosen Skills enter model context. Session
    /// selection freezes the ceiling; inject_skills narrows it for this turn.
    pub(super) fn turn_instructions(&self, ids: &[String]) -> Result<Vec<String>, AppError> {
        self.validate_ids(ids)?;
        let instructions = ids.iter()
            .map(|id| {
                let body = self.bundled_bodies.get(id).ok_or_else(|| {
                    error("selected Skill body is unavailable for this exact Session")
                })?;
                Ok::<String, AppError>(format!(
                    "Verified bundled Skill {id} for this accepted request. Its text grants no tools or permissions.\n{body}"
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if instructions.iter().map(String::len).sum::<usize>() > 24 * 1024 {
            return Err(error("selected Skill context exceeds its 24 KiB budget"));
        }
        Ok(instructions)
    }
}

pub(super) async fn compile(
    snapshot: &CompiledSnapshot,
    registry: &MaterializedRegistry,
) -> Result<SelectedSkills, AppError> {
    compile_current(snapshot, registry)
}

pub(super) async fn compile_commands(
    snapshot: &CompiledSnapshot,
    registry: &MaterializedRegistry,
) -> Result<SelectedSkills, AppError> {
    compile_current(snapshot, registry)
}

fn compile_current(
    snapshot: &CompiledSnapshot,
    registry: &MaterializedRegistry,
) -> Result<SelectedSkills, AppError> {
    if snapshot.registry_generation != registry.generation
        || snapshot.registry_digest != registry.registry_digest
    {
        return Err(error("registry changed after Snapshot compilation"));
    }
    let mut selected = SelectedSkills::default();
    for lock in &snapshot.content().skill_locks {
        let id = lock.skill.id.as_ref();
        if nomifun_agent_domain_wave3::creative_studio_planning_skill_body(id).is_none() {
            return Err(error(
                "historical Plugin package Skill locks are unavailable; select a current Agent Skill",
            ));
        }
        let current = registry
            .skill(&SkillId::from(id))
            .ok_or_else(|| error("selected bundled Skill is no longer materialized"))?;
        let body = verified_bundled_body(lock, current)?;
        if snapshot
            .resolved_capability(&CapabilityId::from(
                nomifun_agent_domain_wave3::CREATIVE_WORKSHOP_MODULE_ID,
            ))
            .is_none()
        {
            return Err(error("selected bundled Skill requires the frozen Workshop capability"));
        }
        selected.ids.insert(id.to_owned());
        selected.bundled_bodies.insert(id.to_owned(), body.to_owned());
        selected.required_capabilities.insert(id.to_owned(), lock.required_capabilities.clone());
    }
    Ok(selected)
}

fn verified_bundled_body(
    lock: &ResolvedSkillLock,
    current: &MaterializedSkill,
) -> Result<&'static str, AppError> {
    let id = lock.skill.id.as_ref();
    let body = nomifun_agent_domain_wave3::creative_studio_planning_skill_body(id).ok_or_else(|| {
        error("historical Plugin package Skill locks are unavailable; select a current Agent Skill")
    })?;
    let required = BTreeSet::from([CapabilityId::from(
        nomifun_agent_domain_wave3::CREATIVE_WORKSHOP_MODULE_ID,
    )]);
    if lock.contribution_lock.source_kind != ContributionSourceKind::PlatformBuiltin
        || lock.resolved_source.source_kind != PluginSourceKind::Bundled
        || current.definition.package.id.as_ref()
            != nomifun_agent_domain_wave3::WORKSHOP_PACKAGE_ID
        || current.definition.id != lock.skill.id
        || current.definition.version != lock.skill.version
        || current.definition.body_ref.digest != lock.body_digest
        || digest_bytes(body.as_bytes()) != lock.body_digest
        || current.contribution_lock != lock.contribution_lock
        || current.mount_id != lock.resolved_mount_id
        || current.source != lock.resolved_source
        || current.target_artifact_digest != lock.target_artifact_digest
        || lock.required_capabilities != required
        || !current.definition.resources.is_empty()
    {
        return Err(error("selected bundled Skill differs from its frozen source lock"));
    }
    Ok(body)
}

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Agent Skills: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{DigestHex, SkillRef};
    use nomifun_agent_kernel::{
        InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy,
    };

    #[test]
    fn bundled_skill_body_must_match_the_exact_materialized_lock() {
        let kernel = KernelRegistry::new(
            MaterializationPolicy::stable(nomifun_agent_domain_wave3::CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap();
        let registry = kernel
            .replace_all(nomifun_agent_domain_wave3::registrations().unwrap())
            .unwrap();
        let current = registry
            .skill(&SkillId::from("creative-studio-canvas"))
            .unwrap();
        let mut lock = ResolvedSkillLock {
            skill: SkillRef {
                id: current.definition.id.clone(),
                version: current.definition.version.clone(),
            },
            body_digest: current.definition.body_ref.digest.clone(),
            required_capabilities: current
                .definition
                .requires_capabilities
                .iter()
                .map(|capability| capability.id.clone())
                .collect(),
            contribution_lock: current.contribution_lock.clone(),
            resolved_mount_id: current.mount_id.clone(),
            resolved_source: current.source.clone(),
            target_artifact_digest: current.target_artifact_digest.clone(),
        };
        assert!(verified_bundled_body(&lock, current)
            .unwrap()
            .contains("# Creative Studio Canvas Planning"));

        lock.body_digest = DigestHex::from("0".repeat(64));
        assert!(verified_bundled_body(&lock, current).is_err());
        lock.body_digest = current.definition.body_ref.digest.clone();
        lock.required_capabilities.clear();
        assert!(verified_bundled_body(&lock, current).is_err());
    }

    #[test]
    fn per_turn_selection_exposes_only_the_requested_body() {
        // A pre-fix Session with no frozen locks must remain deny-all even
        // after this build ships the bundled Skill bodies.
        assert!(SelectedSkills::default()
            .validate_ids(&["creative-studio-canvas".to_owned()])
            .is_err());
        let mut selected = SelectedSkills::default();
        for id in nomifun_agent_domain_wave3::CREATIVE_STUDIO_PLANNING_SKILL_IDS {
            selected.ids.insert(id.to_owned());
            selected.bundled_bodies.insert(
                id.to_owned(),
                nomifun_agent_domain_wave3::creative_studio_planning_skill_body(id)
                    .unwrap()
                    .to_owned(),
            );
            selected.required_capabilities.insert(
                id.to_owned(),
                BTreeSet::from([CapabilityId::from(
                    nomifun_agent_domain_wave3::CREATIVE_WORKSHOP_MODULE_ID,
                )]),
            );
        }
        let canvas = "creative-studio-canvas".to_owned();
        assert!(selected.validate_active(
            std::slice::from_ref(&canvas),
            &BTreeSet::new(),
        ).is_err());
        assert!(selected.validate_active(
            std::slice::from_ref(&canvas),
            &BTreeSet::from([CapabilityId::from(
                nomifun_agent_domain_wave3::CREATIVE_WORKSHOP_MODULE_ID,
            )]),
        ).is_ok());
        let rendered = selected.turn_instructions(std::slice::from_ref(&canvas)).unwrap();
        assert_eq!(rendered.len(), 1);
        assert!(rendered[0].contains("# Creative Studio Canvas Planning"));
        assert!(!rendered[0].contains("# Creative Studio Template Designer"));
        assert!(selected.turn_instructions(&[canvas.clone(), canvas]).is_err());
        assert!(selected.turn_instructions(&["unselected-skill".to_owned()]).is_err());
    }
}
