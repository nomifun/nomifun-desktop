//! Frozen package Skills use the existing Session and Nomi Skill tool, not
//! directory discovery or a second registry. Artifact IO belongs to the host.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomi_agent::host_skills::{HostSkill, HostSkillAccess};
use nomifun_agent_contracts::{ContributionSourceKind, ResolvedSkillLock, SkillDefinition};
use nomifun_agent_kernel::{CompiledSnapshot, KernelRegistry, SessionCapabilityState};
use sha2::{Digest, Sha256};

use crate::plugin_tools::{NomiPluginToolError, NomiPluginToolSession};

pub const MAX_SKILL_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_SESSION_SKILL_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SESSION_SKILL_FILES: usize = 128;

pub struct NomiPluginSkillArtifact {
    pub definition: SkillDefinition,
    pub files: BTreeMap<String, Vec<u8>>,
}

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

#[async_trait]
pub trait NomiPluginSkillArtifactResolver: Send + Sync {
    /// Read only the exact declared files, with bounded IO and digest checks.
    async fn resolve(&self, lock: &ResolvedSkillLock) -> Result<NomiPluginSkillArtifact, String>;
}

struct SkillAccess {
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    // None is a description-only read, never a usable runtime authorization.
    active: Option<Arc<SessionCapabilityState>>,
    lock: ResolvedSkillLock,
}

impl SkillAccess {
    fn validate(&self) -> Result<SkillDefinition, String> {
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
        Ok(current.definition.clone())
    }
}

#[async_trait]
impl HostSkillAccess for SkillAccess {
    async fn authorize(&self) -> Result<(), String> {
        if self.active.is_none() {
            return Err("Skill discovery descriptors cannot authorize execution".into());
        }
        self.validate().map(|_| ())
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

    /// Read-only descriptors used by the runtime bootstrap; their access guard
    /// remains attached for every body/resource read.
    pub fn package_skills(&self) -> &[Arc<HostSkill>] {
        &self.host_skills
    }

    /// Only the authenticated host Session provider calls this. No body or
    /// provenance is accepted from build-extra/model input.
    pub async fn with_package_skills(
        mut self,
        kernel: Arc<KernelRegistry>,
        compiled: Arc<CompiledSnapshot>,
        resolver: Arc<dyn NomiPluginSkillArtifactResolver>,
    ) -> Result<Self, NomiPluginToolError> {
        let error = NomiPluginToolError::Contract;
        if self.resolved_snapshot_ref() != compiled.snapshot_ref() {
            return Err(error("Skill Snapshot differs from its Session".into()));
        }
        let active = self
            .capability_state()
            .ok_or_else(|| error("Skill Session has no active set".into()))?;
        self.host_skills =
            Arc::from(load_package_skills(kernel, compiled, Some(active), resolver).await?);
        Ok(self)
    }
}

/// Describe selected package commands before runtime creation. This shares the
/// exact artifact parser and source checks with execution, but creates no active
/// set and cannot execute the resulting descriptors. Commands are suggestions,
/// not grants: runtime configuration and current authority still govern use.
pub async fn discover_package_skill_commands(
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    resolver: Arc<dyn NomiPluginSkillArtifactResolver>,
) -> Result<Vec<nomifun_api_types::SlashCommandItem>, NomiPluginToolError> {
    let mut commands = load_package_skills(kernel, compiled, None, resolver)
        .await?
        .iter()
        .filter(|skill| skill.metadata().user_invocable)
        .map(|skill| nomifun_api_types::SlashCommandItem {
            command: skill.command_name(),
            description: skill.metadata().description.clone(),
        })
        .collect::<Vec<_>>();
    commands.sort_by(|left, right| left.command.cmp(&right.command));
    Ok(commands)
}

async fn load_package_skills(
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Option<Arc<SessionCapabilityState>>,
    resolver: Arc<dyn NomiPluginSkillArtifactResolver>,
) -> Result<Vec<Arc<HostSkill>>, NomiPluginToolError> {
    let error = NomiPluginToolError::Contract;
    let mut hosted = Vec::new();
    let mut ids = BTreeSet::new();
    let mut total_bytes = 0usize;
    let mut total_files = 0usize;
    for lock in &compiled.content().skill_locks {
        if lock.contribution_lock.source_kind == ContributionSourceKind::PlatformBuiltin {
            continue; // Existing bundled directory adapter; not a package fallback.
        }
        if lock.contribution_lock.source_kind != ContributionSourceKind::PluginMount
            || lock.contribution_lock.mount_id.as_ref() != Some(&lock.resolved_mount_id)
            || !ids.insert(lock.skill.id.clone())
        {
            return Err(error(format!(
                "unsupported or duplicate Skill source {}",
                lock.skill.id.as_ref()
            )));
        }
        let access = Arc::new(SkillAccess {
            kernel: Arc::clone(&kernel),
            compiled: Arc::clone(&compiled),
            active: active.clone(),
            lock: lock.clone(),
        });
        let definition = access.validate().map_err(error)?;
        let expected_files = definition
            .resources
            .len()
            .checked_add(1)
            .ok_or_else(|| error("Skill file count overflow".into()))?;
        total_files = total_files
            .checked_add(expected_files)
            .filter(|value| *value <= MAX_SESSION_SKILL_FILES)
            .ok_or_else(|| error("Session Skill file count limit exceeded".into()))?;
        let artifact = resolver
            .resolve(lock)
            .await
            .map_err(|e| error(format!("Skill {}: {e}", lock.skill.id.as_ref())))?;
        // The source may have been withdrawn while artifact IO was pending.
        access.validate().map_err(error)?;
        if artifact.definition != definition || artifact.files.len() != expected_files {
            return Err(error(format!(
                "Skill {} artifact definition/files mismatch",
                lock.skill.id.as_ref()
            )));
        }
        let mut names = BTreeSet::new();
        for reference in std::iter::once(&definition.body_ref)
            .chain(definition.resources.iter().map(|r| &r.artifact))
        {
            let path = &reference.normalized_relative_path;
            if path.contains('\\')
                || path.contains(':')
                || path
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
                || !names.insert(path.to_lowercase())
            {
                return Err(error(format!(
                    "Skill {} has unsafe or colliding path {path}",
                    lock.skill.id.as_ref()
                )));
            }
            let bytes = artifact
                .files
                .get(path)
                .ok_or_else(|| error(format!("missing Skill file {path}")))?;
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .filter(|n| *n <= MAX_SESSION_SKILL_BYTES)
                .ok_or_else(|| error("Session Skill byte limit exceeded".into()))?;
            if bytes.len() > MAX_SKILL_FILE_BYTES
                || hex::encode(Sha256::digest(bytes)) != reference.digest.as_ref()
            {
                return Err(error(format!(
                    "Skill file {path} exceeds limit or differs from its digest"
                )));
            }
        }
        let mut files = artifact.files;
        let body = files
            .remove(&definition.body_ref.normalized_relative_path)
            .expect("validated Skill body");
        let body = String::from_utf8(body).map_err(|_| {
            error(format!(
                "Skill {} body is not UTF-8",
                lock.skill.id.as_ref()
            ))
        })?;
        let skill = HostSkill::read_only(
            lock.skill.id.as_ref(),
            &definition.display.description,
            &body,
            files,
            access,
        )
        .map_err(error)?;
        hosted.push(Arc::new(skill));
    }
    Ok(hosted)
}
