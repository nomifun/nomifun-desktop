//! Commands verified by the shared host Skill loader retain live authorization.
//! Artifact and resource IO belongs exclusively to that loader.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomi_agent::host_skills::{HostSkill, HostSkillAccess};
use nomifun_agent_contracts::ResolvedSkillLock;
use nomifun_agent_kernel::{CompiledSnapshot, KernelRegistry, SessionCapabilityState};

use crate::plugin_tools::{NomiPluginToolError, NomiPluginToolSession};

/// Body already verified by the host's shared Engine Skill loader. Resources
/// remain with that loader's typed resource tool (including image admission).
pub struct NomiVerifiedSkillCommand {
    pub lock: ResolvedSkillLock,
    pub markdown: String,
    pub description: String,
}

pub fn verified_skill_commands(
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Option<Arc<SessionCapabilityState>>,
    commands: Vec<NomiVerifiedSkillCommand>,
) -> Result<Vec<Arc<HostSkill>>, NomiPluginToolError> {
    let error = NomiPluginToolError::Contract;
    let mut hosted = Vec::new();
    let mut ids = BTreeSet::new();
    for command in commands {
        if !compiled.content().skill_locks.contains(&command.lock)
            || !ids.insert(command.lock.skill.id.clone())
            || nomifun_agent_contracts::digest_bytes(command.markdown.as_bytes()) != command.lock.body_digest
        {
            return Err(error("Skill command differs from the compiled body lock".into()));
        }
        let access = Arc::new(SkillAccess {
            kernel: kernel.clone(), compiled: compiled.clone(), active: active.clone(),
            lock: command.lock.clone(),
        });
        access.validate().map_err(error)?;
        // Upstream Engine Skills treat execution directives as inert reference
        // text. Such bodies remain selected context, but cannot become executable
        // slash commands. Never reinterpret hooks/fork/shell as host authority.
        if let Ok(skill) = HostSkill::read_only(
            command.lock.skill.id.as_ref(), &command.description, &command.markdown,
            BTreeMap::new(), access,
        ) {
            hosted.push(Arc::new(skill.command_only()));
        }
    }
    Ok(hosted)
}

struct SkillAccess {
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    // None is a description-only read, never a usable runtime authorization.
    active: Option<Arc<SessionCapabilityState>>,
    lock: ResolvedSkillLock,
}

impl SkillAccess {
    fn validate(&self) -> Result<(), String> {
        let registry = self.kernel.snapshot().map_err(|e| e.to_string())?;
        let lock = &self.lock;
        let current = registry
            .skill(&lock.skill.id)
            .ok_or_else(|| format!("Skill {} is no longer available", lock.skill.id.as_ref()))?;
        let required = current
            .definition
            .requires_capabilities
            .iter()
            .map(|value| value.id.clone())
            .collect::<BTreeSet<_>>();
        if current.definition.version != lock.skill.version
            || current.definition.body_ref.digest != lock.body_digest
            || current.contribution_lock != lock.contribution_lock
            || current.mount_id != lock.resolved_mount_id
            || current.source != lock.resolved_source
            || current.target_artifact_digest != lock.target_artifact_digest
            || required != lock.required_capabilities
        {
            return Err(format!(
                "Skill {} differs from its frozen source lock",
                lock.skill.id.as_ref()
            ));
        }
        let active = self
            .active
            .as_ref()
            .map(|state| state.snapshot())
            .transpose()
            .map_err(|e| e.to_string())?;
        if active
            .as_ref()
            .is_some_and(|state| state.resolved_snapshot_ref != *self.compiled.snapshot_ref())
        {
            return Err("Skill active set belongs to another Snapshot".into());
        }
        for required in &current.definition.requires_capabilities {
            let frozen = self
                .compiled
                .resolved_capability(&required.id)
                .ok_or_else(|| {
                    format!("Skill dependency {} is not selected", required.id.as_ref())
                })?;
            let live = registry.capability(&required.id).ok_or_else(|| {
                format!("Skill dependency {} is unavailable", required.id.as_ref())
            })?;
            if active
                .as_ref()
                .is_some_and(|state| !state.active.contains(&required.id))
                || frozen.capability.version != required.version
                || live.contribution_lock != frozen.contribution_lock
                || live.target_artifact_digest != frozen.target_artifact_digest
            {
                return Err(format!(
                    "Skill dependency {} is inactive or has changed",
                    required.id.as_ref()
                ));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl HostSkillAccess for SkillAccess {
    async fn authorize(&self) -> Result<(), String> {
        if self.active.is_none() {
            return Err("Skill discovery descriptors cannot authorize execution".into());
        }
        self.validate()
    }
}

impl NomiPluginToolSession {
    pub fn with_verified_skill_commands(
        mut self,
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        commands: Vec<NomiVerifiedSkillCommand>,
    ) -> Result<Self, NomiPluginToolError> {
        if self.resolved_snapshot_ref() != compiled.snapshot_ref() || !self.host_skills.is_empty() {
            return Err(NomiPluginToolError::Contract("Skill command Session differs or is already bound".into()));
        }
        let active = self.capability_state().ok_or_else(||
            NomiPluginToolError::Contract("Skill command Session has no active set".into()))?;
        self.host_skills = Arc::from(verified_skill_commands(kernel, compiled, Some(active), commands)?);
        Ok(self)
    }

    /// Command-only descriptors used by runtime bootstrap, guarded on every read.
    /// Resources are exposed separately by the shared loader's typed resource tool.
    pub fn package_skills(&self) -> &[Arc<HostSkill>] {
        &self.host_skills
    }
}
