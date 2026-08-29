use std::collections::BTreeMap;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentBindingValue, AgentPreset, AgentPresetId, AgentPresetRevision, PresetRevisionRef,
    RemoteBinding, RemoteBindingId, ResolvedSnapshotEnvelope, UserId,
};
use tokio::sync::RwLock;

use crate::ControlPlaneError;

#[derive(Clone, Debug)]
pub struct StoredPreset {
    pub preset: AgentPreset,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentBindingTarget {
    pub target_kind: String,
    pub target_id: String,
}

#[derive(Clone, Debug)]
pub struct StoredAgentBinding {
    pub target: AgentBindingTarget,
    pub owner_user_id: UserId,
    pub value: AgentBindingValue,
}

#[async_trait]
pub trait ControlPlaneStore: Send + Sync {
    async fn list_presets(&self, owner: &UserId) -> Result<Vec<StoredPreset>, ControlPlaneError>;
    async fn get_preset(
        &self,
        preset_id: &AgentPresetId,
    ) -> Result<Option<StoredPreset>, ControlPlaneError>;
    async fn insert_preset(&self, preset: StoredPreset) -> Result<(), ControlPlaneError>;
    async fn insert_preset_with_revision(
        &self,
        preset: StoredPreset,
        revision: AgentPresetRevision,
        snapshot: ResolvedSnapshotEnvelope,
    ) -> Result<StoredPreset, ControlPlaneError>;
    async fn update_preset(&self, preset: StoredPreset) -> Result<(), ControlPlaneError>;
    async fn get_revision(
        &self,
        reference: &PresetRevisionRef,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError>;
    async fn get_revision_number(
        &self,
        preset_id: &AgentPresetId,
        revision: u64,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError>;
    async fn append_revision(
        &self,
        expected_current: Option<&PresetRevisionRef>,
        revision: AgentPresetRevision,
        snapshot: ResolvedSnapshotEnvelope,
        display_name: String,
        description: Option<String>,
    ) -> Result<StoredPreset, ControlPlaneError>;
    async fn get_snapshot(
        &self,
        reference: &PresetRevisionRef,
    ) -> Result<Option<ResolvedSnapshotEnvelope>, ControlPlaneError>;
    async fn list_agent_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<StoredAgentBinding>, ControlPlaneError>;
    async fn get_agent_binding(
        &self,
        target: &AgentBindingTarget,
    ) -> Result<Option<StoredAgentBinding>, ControlPlaneError>;
    async fn put_agent_binding(
        &self,
        binding: StoredAgentBinding,
        expected_binding_version: Option<u64>,
    ) -> Result<StoredAgentBinding, ControlPlaneError>;
    async fn list_remote_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<RemoteBinding>, ControlPlaneError>;
    async fn get_remote_binding(
        &self,
        binding_id: &RemoteBindingId,
    ) -> Result<Option<RemoteBinding>, ControlPlaneError>;
    async fn insert_remote_binding(
        &self,
        binding: RemoteBinding,
    ) -> Result<RemoteBinding, ControlPlaneError>;
    async fn update_remote_binding(
        &self,
        binding: RemoteBinding,
        expected_binding_version: u64,
        expected_agent_binding_digest: &str,
    ) -> Result<RemoteBinding, ControlPlaneError>;
    async fn delete_remote_binding(
        &self,
        owner: &UserId,
        binding_id: &RemoteBindingId,
    ) -> Result<(), ControlPlaneError>;
}

#[derive(Default)]
struct InMemoryState {
    presets: BTreeMap<AgentPresetId, StoredPreset>,
    revisions: BTreeMap<(AgentPresetId, u64), AgentPresetRevision>,
    snapshots: BTreeMap<(AgentPresetId, u64), ResolvedSnapshotEnvelope>,
    agent_bindings: BTreeMap<AgentBindingTarget, StoredAgentBinding>,
    remote_bindings: BTreeMap<RemoteBindingId, RemoteBinding>,
}

#[derive(Default)]
pub struct InMemoryControlPlaneStore {
    state: RwLock<InMemoryState>,
}

impl InMemoryControlPlaneStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ControlPlaneStore for InMemoryControlPlaneStore {
    async fn list_presets(&self, owner: &UserId) -> Result<Vec<StoredPreset>, ControlPlaneError> {
        Ok(self
            .state
            .read()
            .await
            .presets
            .values()
            .filter(|stored| stored.preset.owner_user_id.as_ref() == Some(owner))
            .cloned()
            .collect())
    }

    async fn get_preset(
        &self,
        preset_id: &AgentPresetId,
    ) -> Result<Option<StoredPreset>, ControlPlaneError> {
        Ok(self.state.read().await.presets.get(preset_id).cloned())
    }

    async fn insert_preset(&self, preset: StoredPreset) -> Result<(), ControlPlaneError> {
        let mut state = self.state.write().await;
        if state.presets.contains_key(&preset.preset.preset_id) {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                "AgentPreset already exists",
            ));
        }
        state
            .presets
            .insert(preset.preset.preset_id.clone(), preset);
        Ok(())
    }

    async fn insert_preset_with_revision(
        &self,
        preset: StoredPreset,
        revision: AgentPresetRevision,
        snapshot: ResolvedSnapshotEnvelope,
    ) -> Result<StoredPreset, ControlPlaneError> {
        let mut state = self.state.write().await;
        let preset_id = preset.preset.preset_id.clone();
        let key = (preset_id.clone(), revision.reference.revision);
        if state.presets.contains_key(&preset_id)
            || state.revisions.contains_key(&key)
            || state.snapshots.contains_key(&key)
            || revision.reference.preset_id != preset_id
            || snapshot.content.preset_revision_ref != revision.reference
            || preset.preset.current_stable_revision.as_ref() != Some(&revision.reference)
        {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                "atomic Preset/Revision/Snapshot insert contract did not match",
            ));
        }
        state.revisions.insert(key.clone(), revision);
        state.snapshots.insert(key, snapshot);
        state.presets.insert(preset_id, preset.clone());
        Ok(preset)
    }

    async fn update_preset(&self, preset: StoredPreset) -> Result<(), ControlPlaneError> {
        self.state
            .write()
            .await
            .presets
            .insert(preset.preset.preset_id.clone(), preset);
        Ok(())
    }

    async fn get_revision(
        &self,
        reference: &PresetRevisionRef,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
        Ok(self
            .state
            .read()
            .await
            .revisions
            .get(&(reference.preset_id.clone(), reference.revision))
            .filter(|revision| revision.reference.revision_digest == reference.revision_digest)
            .cloned())
    }

    async fn get_revision_number(
        &self,
        preset_id: &AgentPresetId,
        revision: u64,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
        Ok(self
            .state
            .read()
            .await
            .revisions
            .get(&(preset_id.clone(), revision))
            .cloned())
    }

    async fn append_revision(
        &self,
        expected_current: Option<&PresetRevisionRef>,
        revision: AgentPresetRevision,
        snapshot: ResolvedSnapshotEnvelope,
        display_name: String,
        description: Option<String>,
    ) -> Result<StoredPreset, ControlPlaneError> {
        let mut state = self.state.write().await;
        let current = state
            .presets
            .get(&revision.reference.preset_id)
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "PRESET_REVISION_DIGEST_MISMATCH",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "AgentPreset does not exist",
                )
            })?;
        if current.preset.current_stable_revision.as_ref() != expected_current {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                "expected_current_revision does not match the current immutable revision",
            ));
        }

        let key = (
            revision.reference.preset_id.clone(),
            revision.reference.revision,
        );
        if state.revisions.contains_key(&key) {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                "revision number already exists",
            ));
        }
        state.revisions.insert(key.clone(), revision.clone());
        state.snapshots.insert(key, snapshot);
        let stored = state
            .presets
            .get_mut(&revision.reference.preset_id)
            .expect("preset existence was checked under the same write lock");
        stored.preset.current_stable_revision = Some(revision.reference);
        stored.preset.display_name = display_name;
        stored.preset.description = description;
        Ok(stored.clone())
    }

    async fn get_snapshot(
        &self,
        reference: &PresetRevisionRef,
    ) -> Result<Option<ResolvedSnapshotEnvelope>, ControlPlaneError> {
        Ok(self
            .state
            .read()
            .await
            .snapshots
            .get(&(reference.preset_id.clone(), reference.revision))
            .filter(|snapshot| {
                snapshot.content.preset_revision_ref.revision_digest
                    == reference.revision_digest
            })
            .cloned())
    }

    async fn list_agent_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<StoredAgentBinding>, ControlPlaneError> {
        Ok(self
            .state
            .read()
            .await
            .agent_bindings
            .values()
            .filter(|binding| &binding.owner_user_id == owner)
            .cloned()
            .collect())
    }

    async fn get_agent_binding(
        &self,
        target: &AgentBindingTarget,
    ) -> Result<Option<StoredAgentBinding>, ControlPlaneError> {
        Ok(self
            .state
            .read()
            .await
            .agent_bindings
            .get(target)
            .cloned())
    }

    async fn put_agent_binding(
        &self,
        binding: StoredAgentBinding,
        expected_binding_version: Option<u64>,
    ) -> Result<StoredAgentBinding, ControlPlaneError> {
        let mut state = self.state.write().await;
        if let Some(existing) = state.agent_bindings.get(&binding.target) {
            if expected_binding_version != Some(existing.value.binding_version) {
                return Err(ControlPlaneError::canonical(
                    "PRESET_REVISION_DIGEST_MISMATCH",
                    axum::http::StatusCode::CONFLICT,
                    "agent binding version changed",
                ));
            }
        } else if expected_binding_version.is_some() {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                "agent binding does not exist at the expected version",
            ));
        }
        state
            .agent_bindings
            .insert(binding.target.clone(), binding.clone());
        Ok(binding)
    }

    async fn list_remote_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<RemoteBinding>, ControlPlaneError> {
        Ok(self
            .state
            .read()
            .await
            .remote_bindings
            .values()
            .filter(|binding| &binding.owner_user_id == owner)
            .cloned()
            .collect())
    }

    async fn get_remote_binding(
        &self,
        binding_id: &RemoteBindingId,
    ) -> Result<Option<RemoteBinding>, ControlPlaneError> {
        Ok(self
            .state
            .read()
            .await
            .remote_bindings
            .get(binding_id)
            .cloned())
    }

    async fn insert_remote_binding(
        &self,
        binding: RemoteBinding,
    ) -> Result<RemoteBinding, ControlPlaneError> {
        self.state
            .write()
            .await
            .remote_bindings
            .insert(binding.remote_binding_id.clone(), binding.clone());
        Ok(binding)
    }

    async fn update_remote_binding(
        &self,
        binding: RemoteBinding,
        expected_binding_version: u64,
        expected_agent_binding_digest: &str,
    ) -> Result<RemoteBinding, ControlPlaneError> {
        let mut state = self.state.write().await;
        let existing = state
            .remote_bindings
            .get(&binding.remote_binding_id)
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "REMOTE_BINDING_NOT_FOUND",
                    axum::http::StatusCode::NOT_FOUND,
                    "RemoteBinding does not exist",
                )
            })?;
        if existing.agent_binding.binding_version != expected_binding_version {
            return Err(ControlPlaneError::canonical(
                "REMOTE_BINDING_VERSION_CONFLICT",
                axum::http::StatusCode::CONFLICT,
                "RemoteBinding version changed",
            ));
        }
        let digest = nomifun_agent_contracts::digest_payload(&existing.agent_binding)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        if digest.as_ref() != expected_agent_binding_digest {
            return Err(ControlPlaneError::canonical(
                "REMOTE_BINDING_DIGEST_CONFLICT",
                axum::http::StatusCode::CONFLICT,
                "RemoteBinding digest changed",
            ));
        }
        state
            .remote_bindings
            .insert(binding.remote_binding_id.clone(), binding.clone());
        Ok(binding)
    }

    async fn delete_remote_binding(
        &self,
        owner: &UserId,
        binding_id: &RemoteBindingId,
    ) -> Result<(), ControlPlaneError> {
        let mut state = self.state.write().await;
        let binding = state.remote_bindings.get(binding_id).ok_or_else(|| {
            ControlPlaneError::canonical(
                "REMOTE_BINDING_NOT_FOUND",
                axum::http::StatusCode::NOT_FOUND,
                "RemoteBinding does not exist",
            )
        })?;
        if &binding.owner_user_id != owner {
            return Err(ControlPlaneError::canonical(
                "REMOTE_BINDING_NOT_FOUND",
                axum::http::StatusCode::NOT_FOUND,
                "RemoteBinding does not exist",
            ));
        }
        state.remote_bindings.remove(binding_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stored_preset_has_no_template_foreign_key() {
        let source = include_str!("store.rs");
        let start = source.find("pub struct StoredPreset").unwrap();
        let end = source[start..].find("}\n").unwrap() + start;
        let stored_preset = &source[start..=end];
        assert!(!stored_preset.contains(&("source_template".to_owned() + "_key")));
        assert_eq!(stored_preset.matches("pub preset: AgentPreset").count(), 1);
    }
}
