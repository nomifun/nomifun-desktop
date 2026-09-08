use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentBindingValue, AgentPreset, AgentPresetId, AgentPresetRevision, PresetRevisionRef,
    RemoteBinding, RemoteBindingId, ResolvedSnapshotEnvelope, UserId,
};
use tokio::sync::RwLock;

use crate::ControlPlaneError;

#[derive(Clone, Debug)]
pub struct StoredPreset {
    pub session_only: bool,
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
    async fn retire_preset(
        &self,
        owner: &UserId,
        preset_id: &AgentPresetId,
    ) -> Result<(), ControlPlaneError>;
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
    retired_presets: BTreeMap<AgentPresetId, i64>,
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
        let state = self.state.read().await;
        Ok(state
            .presets
            .values()
            .filter(|stored| {
                stored.preset.owner_user_id.as_ref() == Some(owner)
                    && !state
                        .retired_presets
                        .contains_key(&stored.preset.preset_id)
            })
            .cloned()
            .collect())
    }

    async fn get_preset(
        &self,
        preset_id: &AgentPresetId,
    ) -> Result<Option<StoredPreset>, ControlPlaneError> {
        let state = self.state.read().await;
        if state.retired_presets.contains_key(preset_id) {
            return Ok(None);
        }
        Ok(state.presets.get(preset_id).cloned())
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
        let mut state = self.state.write().await;
        let preset_id = &preset.preset.preset_id;
        if !state.presets.contains_key(preset_id)
            || state.retired_presets.contains_key(preset_id)
        {
            return Err(agent_preset_not_found());
        }
        state.presets.insert(preset_id.clone(), preset);
        Ok(())
    }

    async fn retire_preset(
        &self,
        owner: &UserId,
        preset_id: &AgentPresetId,
    ) -> Result<(), ControlPlaneError> {
        let mut state = self.state.write().await;
        let Some(stored) = state.presets.get(preset_id) else {
            return Err(agent_preset_not_found());
        };
        if state.retired_presets.contains_key(preset_id)
            || stored.preset.owner_user_id.as_ref() != Some(owner)
            || stored.preset.source != nomifun_agent_contracts::AgentPresetSource::User
        {
            return Err(agent_preset_not_found());
        }
        state.agent_bindings.retain(|_, binding| {
            binding.value.preset_revision_ref.preset_id != *preset_id
        });
        state.remote_bindings.retain(|_, binding| {
            binding.agent_binding.preset_revision_ref.preset_id != *preset_id
        });
        state.retired_presets.insert(preset_id.clone(), now_ms());
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
        if state
            .retired_presets
            .contains_key(&revision.reference.preset_id)
        {
            return Err(agent_preset_not_found());
        }
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
        let preset_id = &binding.value.preset_revision_ref.preset_id;
        let active_owned_preset = state.presets.get(preset_id).is_some_and(|stored| {
            stored.preset.owner_user_id.as_ref() == Some(&binding.owner_user_id)
                && stored.preset.source == nomifun_agent_contracts::AgentPresetSource::User
                && !state.retired_presets.contains_key(preset_id)
        });
        if !active_owned_preset {
            return Err(agent_preset_not_found());
        }
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
        let mut state = self.state.write().await;
        let preset_id = &binding.agent_binding.preset_revision_ref.preset_id;
        let active_owned_preset = state.presets.get(preset_id).is_some_and(|stored| {
            stored.preset.owner_user_id.as_ref() == Some(&binding.owner_user_id)
                && stored.preset.source == nomifun_agent_contracts::AgentPresetSource::User
                && !state.retired_presets.contains_key(preset_id)
        });
        if !active_owned_preset {
            return Err(agent_preset_not_found());
        }
        state
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
        let preset_id = &binding.agent_binding.preset_revision_ref.preset_id;
        let active_owned_preset = state.presets.get(preset_id).is_some_and(|stored| {
            stored.preset.owner_user_id.as_ref() == Some(&binding.owner_user_id)
                && stored.preset.source == nomifun_agent_contracts::AgentPresetSource::User
                && !state.retired_presets.contains_key(preset_id)
        });
        if !active_owned_preset {
            return Err(agent_preset_not_found());
        }
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn agent_preset_not_found() -> ControlPlaneError {
    ControlPlaneError::canonical(
        "AGENT_PRESET_NOT_FOUND",
        axum::http::StatusCode::NOT_FOUND,
        "AgentPreset does not exist",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        AgentPresetSource, DigestHex, ResolvedSnapshotId, ResolvedSnapshotRef,
    };

    fn preset(id: &str, owner: Option<&UserId>, source: AgentPresetSource) -> StoredPreset {
        StoredPreset {
            session_only: false,
            preset: AgentPreset {
                preset_id: AgentPresetId::from(id),
                owner_user_id: owner.cloned(),
                source,
                display_name: id.to_owned(),
                description: None,
                current_stable_revision: None,
            },
        }
    }

    fn binding_value(preset_id: &str) -> AgentBindingValue {
        AgentBindingValue {
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from(preset_id),
                revision: 1,
                revision_digest: DigestHex::from("revision-digest"),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("snapshot"),
                snapshot_digest: DigestHex::from("snapshot-digest"),
            },
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        }
    }

    #[test]
    fn stored_preset_has_no_template_foreign_key() {
        let source = include_str!("store.rs");
        let start = source.find("pub struct StoredPreset").unwrap();
        let end = source[start..].find("}\n").unwrap() + start;
        let stored_preset = &source[start..=end];
        assert!(!stored_preset.contains(&("source_template".to_owned() + "_key")));
        assert_eq!(stored_preset.matches("pub preset: AgentPreset").count(), 1);
    }

    #[tokio::test]
    async fn in_memory_retirement_is_owner_scoped_and_hides_active_reads() {
        let store = InMemoryControlPlaneStore::new();
        let owner = UserId::from("owner-1");
        let other_owner = UserId::from("owner-2");
        let preset_id = AgentPresetId::from("preset-1");
        store
            .insert_preset(preset("preset-1", Some(&owner), AgentPresetSource::User))
            .await
            .unwrap();

        let other_error = store
            .retire_preset(&other_owner, &preset_id)
            .await
            .expect_err("another owner must not retire the Preset");
        assert_eq!(other_error.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(other_error.code().as_ref(), "AGENT_PRESET_NOT_FOUND");
        assert!(store.get_preset(&preset_id).await.unwrap().is_some());

        store.retire_preset(&owner, &preset_id).await.unwrap();
        assert!(store.get_preset(&preset_id).await.unwrap().is_none());
        assert!(store.list_presets(&owner).await.unwrap().is_empty());

        let repeated_error = store
            .retire_preset(&owner, &preset_id)
            .await
            .expect_err("a retired Preset must be indistinguishable from a missing Preset");
        assert_eq!(repeated_error.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(repeated_error.code().as_ref(), "AGENT_PRESET_NOT_FOUND");
    }

    #[tokio::test]
    async fn in_memory_retirement_rejects_official_and_clears_active_bindings() {
        let store = InMemoryControlPlaneStore::new();
        let owner = UserId::from("owner-1");
        let official_id = AgentPresetId::from("official-seed");
        store
            .insert_preset(preset(
                "official-seed",
                None,
                AgentPresetSource::Official,
            ))
            .await
            .unwrap();
        let official_error = store
            .retire_preset(&owner, &official_id)
            .await
            .expect_err("official seed rows are not product-deletable");
        assert_eq!(official_error.code().as_ref(), "AGENT_PRESET_NOT_FOUND");

        let preset_id = AgentPresetId::from("bound-preset");
        store
            .insert_preset(preset(
                preset_id.as_ref(),
                Some(&owner),
                AgentPresetSource::User,
            ))
            .await
            .unwrap();
        let target = AgentBindingTarget {
            target_kind: "conversation".to_owned(),
            target_id: "target-1".to_owned(),
        };
        store
            .put_agent_binding(
                StoredAgentBinding {
                    target: target.clone(),
                    owner_user_id: owner.clone(),
                    value: binding_value(preset_id.as_ref()),
                },
                None,
            )
            .await
            .unwrap();
        let remote_id = RemoteBindingId::from("remote-1");
        store
            .insert_remote_binding(RemoteBinding {
                remote_binding_id: remote_id.clone(),
                owner_user_id: owner.clone(),
                name: "Remote".to_owned(),
                agent_binding: binding_value(preset_id.as_ref()),
            })
            .await
            .unwrap();

        store.retire_preset(&owner, &preset_id).await.unwrap();
        assert!(store.get_preset(&preset_id).await.unwrap().is_none());
        assert!(store.get_agent_binding(&target).await.unwrap().is_none());
        assert!(
            store
                .get_remote_binding(&remote_id)
                .await
                .unwrap()
                .is_none()
        );
    }
}
