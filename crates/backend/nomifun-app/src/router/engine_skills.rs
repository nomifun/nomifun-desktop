//! Agent Skill projection for the current immutable Agent snapshot.
//!
//! Unified Plugins expose Action + Binding only. Historical N1 package Skill
//! locks therefore become explicitly unavailable instead of being decoded by
//! a retired Plugin Artifact format. The global Agent Skill contracts remain
//! owned by the Agent platform and other Skill Library consumers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_kernel::{CompiledSnapshot, MaterializedRegistry};
use nomifun_common::AppError;
use nomifun_engine_core::EngineContextResource;
use serde_json::Value;

#[derive(Default)]
pub struct SelectedSkills {
    pub(super) ids: BTreeSet<String>,
    pub(super) instructions: Vec<String>,
    pub(super) resources: Arc<BTreeMap<String, EngineContextResource>>,
    pub(super) commands: Vec<nomifun_ai_agent::plugin_skills::NomiVerifiedSkillCommand>,
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
        if ids.len() > 16 || ids.iter().any(|id| !self.ids.contains(id)) {
            return Err(error(
                "requested Skill is not in the Agent's immutable selected Skill locks",
            ));
        }
        Ok(())
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
    if snapshot.content().skill_locks.is_empty() {
        return Ok(SelectedSkills::default());
    }
    Err(error(
        "historical Plugin package Skill locks are unavailable; select a current Agent Skill",
    ))
}

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Agent Skills: {value}"))
}
