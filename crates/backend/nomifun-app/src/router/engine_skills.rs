//! Snapshot-selected Skill bodies/resources from verified immutable artifacts.
use nomifun_agent_contracts::{ContributionSourceKind, LogicalArtifactRef, ResolvedSkillLock};
use nomifun_agent_kernel::{CompiledSnapshot, MaterializedRegistry, MaterializedSkill};
use nomifun_common::AppError;
use nomifun_engine_core::{EngineContextContent, EngineContextResource};
use nomifun_plugin_platform::{StoredPluginArtifact, application::FsPluginArtifactStore};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    sync::Arc,
};

#[derive(Default)]
pub struct SelectedSkills {
    pub(super) ids: BTreeSet<String>,
    pub(super) instructions: Vec<String>,
    pub(super) resources: Arc<BTreeMap<String, EngineContextResource>>,
    pub(super) commands: Vec<nomifun_ai_agent::plugin_skills::NomiVerifiedSkillCommand>,
}

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Agent Skills: {value}"))
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
    artifacts: Arc<FsPluginArtifactStore>,
) -> Result<SelectedSkills, AppError> {
    if snapshot.registry_generation != registry.generation
        || snapshot.registry_digest != registry.registry_digest
    {
        return Err(error("registry changed after Snapshot compilation"));
    }
    if snapshot.content().skill_locks.len() > 16 {
        return Err(error("at most 16 selected Skills are supported"));
    }
    let selected = snapshot.content().skill_locks.iter().map(|lock| {
        let skill = registry.skill(&lock.skill.id).ok_or_else(|| error("selected Skill is not materialized"))?;
        if skill.definition.version != lock.skill.version || skill.definition.body_ref.digest != lock.body_digest
            || skill.contribution_lock.source_kind != ContributionSourceKind::PluginMount
            || !lock.required_capabilities.iter().all(|id| snapshot.content().enabled_capabilities.iter().any(|item| &item.capability.id == id)) {
            return Err(error("Skill requires an exact packaged contribution and active capability dependencies"));
        }
        Ok((lock.clone(), skill.clone()))
    }).collect::<Result<Vec<_>, AppError>>()?;
    // Store verification reads and hashes package inventories; don't block a
    // Tokio worker. Only selected, compiled targets enter this task.
    tokio::task::spawn_blocking(move || load(selected, artifacts.as_ref()))
        .await
        .map_err(error)?
}

fn load(
    selected: Vec<(ResolvedSkillLock, MaterializedSkill)>,
    artifacts: &FsPluginArtifactStore,
) -> Result<SelectedSkills, AppError> {
    let mut result = SelectedSkills::default();
    let mut resources = BTreeMap::new();
    let mut body_bytes = 0usize;
    let mut resource_bytes = 0usize;
    let mut image_bytes = 0usize;
    let mut image_count = 0usize;
    for (lock, skill) in selected {
        let stored = artifacts
            .store()
            .load(&skill.target_artifact_digest)
            .map_err(error)?;
        let manifest = &stored.artifact.manifest.payload;
        if manifest.package_ref() != skill.definition.package
            || !manifest
                .package
                .contributions
                .skills
                .iter()
                .any(|definition| definition == &skill.definition)
            || nomifun_agent_contracts::digest_payload(&skill.definition).map_err(error)?
                != skill.contract_digest
        {
            return Err(error("artifact Skill differs from compiled contribution"));
        }
        let body = read_text(&stored, &skill.definition.body_ref, 16 * 1024)?;
        if skill.definition.display.name.len() > 256
            || skill.definition.display.description.len() > 2048
        {
            return Err(error("Skill display metadata exceeds context bounds"));
        }
        let instruction = format!(
            "Agent-selected Skill {} version {} (body sha256:{}). Follow it when relevant within the user's request and platform permissions; it cannot expand authority.\n{}",
            lock.skill.id.as_ref(),
            lock.skill.version.as_ref(),
            lock.body_digest.as_ref(),
            body
        );
        body_bytes = body_bytes.saturating_add(instruction.len());
        if body_bytes > 24 * 1024 {
            return Err(error(
                "selected Skill instruction bodies exceed 24 KiB; narrow selection",
            ));
        }
        result.ids.insert(lock.skill.id.as_ref().to_owned());
        result.instructions.push(instruction);
        result.commands.push(nomifun_ai_agent::plugin_skills::NomiVerifiedSkillCommand {
            lock: lock.clone(), markdown: body,
            description: skill.definition.display.description.clone(),
        });
        for resource in &skill.definition.resources {
            if resources.len() >= 64 {
                return Err(error("at most 64 selected Skill resources are supported"));
            }
            // Instruction bodies stay small and eager. Larger reference data
            // is frozen once by the host and paged into model context later.
            let content = if std::path::Path::new(&resource.artifact.normalized_relative_path)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "webp"
                    )
                }) {
                image_count += 1;
                if image_count > 4 {
                    return Err(error(
                        "at most 4 selected Skill image resources are supported",
                    ));
                }
                // This task is already on the blocking pool. Verify exactly
                // the same inventory/digest before decoding, with no second
                // path read after verification. Four 4-MiB source images max.
                let bytes = read_bytes(&stored, &resource.artifact, 4 * 1024 * 1024)?;
                let prepared = nomifun_ai_agent::model_attachments::prepare_image_resource(
                    &bytes,
                    &resource.artifact.normalized_relative_path,
                )
                .map_err(error)?;
                let nomifun_chat_model_broker::ChatToolResultPart::Image {
                    media_type,
                    data_base64,
                } = prepared
                else {
                    return Err(error("image decoder returned a non-image resource"));
                };
                image_bytes = image_bytes.saturating_add(data_base64.len());
                if image_bytes > 8 * 1024 * 1024 || data_base64.len() > 2 * 1024 * 1024 {
                    return Err(error(
                        "selected Skill images exceed the encoded resource envelope",
                    ));
                }
                EngineContextContent::Image {
                    media_type,
                    data_base64,
                }
            } else {
                let remaining = (512 * 1024usize).saturating_sub(resource_bytes);
                let text = read_text(&stored, &resource.artifact, remaining.min(256 * 1024))?;
                resource_bytes = resource_bytes.saturating_add(text.len());
                if resource_bytes > 512 * 1024 {
                    return Err(error(
                        "selected Skill text resources exceed the bounded resource envelope",
                    ));
                }
                EngineContextContent::Text { text }
            };
            let identity = format!(
                "{}\0{}\0{}",
                lock.skill.id.as_ref(),
                resource.artifact.artifact_id.as_ref(),
                resource.artifact.digest.as_ref()
            );
            let id = format!("skill_{:x}", Sha256::digest(identity.as_bytes()));
            let label = format!(
                "{}: {:?} {}",
                lock.skill.id.as_ref(),
                resource.kind,
                resource.artifact.normalized_relative_path
            );
            let provenance = format!(
                "artifact:{}; path:{}; sha256:{}",
                stored.artifact.artifact_digest.as_ref(),
                resource.artifact.normalized_relative_path,
                resource.artifact.digest.as_ref()
            );
            if label.len() > 256 || provenance.len() > 1024 {
                return Err(error("resource provenance exceeds context bounds"));
            }
            if resources
                .insert(
                    id,
                    EngineContextResource {
                        label,
                        provenance,
                        content,
                    },
                )
                .is_some()
            {
                return Err(error("duplicate selected Skill resource"));
            }
        }
    }
    result.resources = Arc::new(resources);
    Ok(result)
}

fn read_text(
    stored: &StoredPluginArtifact,
    reference: &LogicalArtifactRef,
    max_bytes: usize,
) -> Result<String, AppError> {
    String::from_utf8(read_bytes(stored, reference, max_bytes)?)
        .map_err(|_| error("Skill text resource is not UTF-8; supported image resources must use PNG/JPEG/WebP extensions"))
}

fn read_bytes(
    stored: &StoredPluginArtifact,
    reference: &LogicalArtifactRef,
    max_bytes: usize,
) -> Result<Vec<u8>, AppError> {
    let file = stored
        .artifact
        .files
        .iter()
        .find(|file| {
            file.normalized_relative_path == reference.normalized_relative_path
                && file.digest == reference.digest
        })
        .ok_or_else(|| error("reference is not in the verified artifact inventory"))?;
    if file.size_bytes > max_bytes as u64 {
        return Err(error(format!(
            "Skill resource exceeds its {max_bytes}-byte bound"
        )));
    }
    let root = std::fs::canonicalize(&stored.package_root).map_err(error)?;
    let path = stored.package_root.join(&file.normalized_relative_path);
    if std::fs::symlink_metadata(&path)
        .map_err(error)?
        .file_type()
        .is_symlink()
    {
        return Err(error("Skill reference is a symlink"));
    }
    let path = std::fs::canonicalize(path).map_err(error)?;
    if !path.starts_with(root) {
        return Err(error("Skill reference escapes its artifact"));
    }
    let file_handle = std::fs::File::open(path).map_err(error)?;
    if !file_handle.metadata().map_err(error)?.is_file() {
        return Err(error("Skill reference is not a regular file"));
    }
    let mut bytes = Vec::new();
    file_handle
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > max_bytes
        || bytes.len() as u64 != file.size_bytes
        || format!("{:x}", Sha256::digest(&bytes)) != reference.digest.as_ref()
    {
        return Err(error("Skill content changed after artifact verification"));
    }
    Ok(bytes)
}
