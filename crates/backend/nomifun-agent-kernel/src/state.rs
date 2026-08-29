use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    PluginStateCompareAndSwapOutcome, PluginStateDeleteResponse, PluginStateEntry,
    PluginStateHandleDescriptor, PluginStateMethod, PluginStateNamespace, PluginStateSetResponse,
    PackageId, PluginMountId, ScopeKey, StateKey, StrictJsonValue, VersionString,
};
use thiserror::Error;

pub const MAX_PLUGIN_STATE_BYTES: usize = 64 * 1024;
pub const MAX_PLUGIN_STATE_KEY_BYTES: usize = 256;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PluginStateError {
    #[error("plugin state key must not be empty")]
    EmptyKey,
    #[error("plugin state key exceeds {MAX_PLUGIN_STATE_KEY_BYTES} bytes")]
    KeyTooLarge,
    #[error("plugin state value exceeds {MAX_PLUGIN_STATE_BYTES} bytes")]
    ValueTooLarge,
    #[error("plugin state value is not strict JSON")]
    InvalidJson,
    #[error("plugin state format version must not be empty")]
    EmptyFormatVersion,
    #[error("plugin state persistence failed: {0}")]
    Persistence(String),
    #[error("plugin state namespace is not owned by this handle")]
    NamespaceMismatch,
    #[error("plugin state store lock is poisoned")]
    LockPoisoned,
    #[error("plugin state revision counter is exhausted")]
    RevisionExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateIdentity {
    pub package_id: PackageId,
    pub mount_id: PluginMountId,
    pub scope_key: ScopeKey,
    pub state_key: StateKey,
}

impl StateIdentity {
    pub fn namespace(&self) -> PluginStateNamespace {
        PluginStateNamespace {
            package_id: self.package_id.clone(),
            mount_id: self.mount_id.clone(),
            scope_key: self.scope_key.clone(),
            state_key: self.state_key.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginStateSnapshot {
    entries: BTreeMap<StateIdentity, PluginStateEntry>,
    revisions: BTreeMap<StateIdentity, u64>,
}

impl PluginStateSnapshot {
    pub fn from_parts(
        entries: BTreeMap<StateIdentity, PluginStateEntry>,
        revisions: BTreeMap<StateIdentity, u64>,
    ) -> Result<Self, PluginStateError> {
        for (identity, entry) in &entries {
            if entry.namespace != identity.namespace()
                || revisions
                    .get(identity)
                    .is_none_or(|revision| *revision < entry.revision)
            {
                return Err(PluginStateError::NamespaceMismatch);
            }
        }
        Ok(Self { entries, revisions })
    }

    pub fn entry(
        &self,
        package_id: &PackageId,
        mount_id: &PluginMountId,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Option<&PluginStateEntry> {
        self.entries.get(&StateIdentity {
            package_id: package_id.clone(),
            mount_id: mount_id.clone(),
            scope_key: scope_key.clone(),
            state_key: state_key.clone(),
        })
    }

    pub fn revision(
        &self,
        package_id: &PackageId,
        mount_id: &PluginMountId,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> u64 {
        self.revisions
            .get(&StateIdentity {
                package_id: package_id.clone(),
                mount_id: mount_id.clone(),
                scope_key: scope_key.clone(),
                state_key: state_key.clone(),
            })
            .copied()
            .unwrap_or(0)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&StateIdentity, &PluginStateEntry)> {
        self.entries.iter()
    }

    pub fn revisions(&self) -> impl Iterator<Item = (&StateIdentity, &u64)> {
        self.revisions.iter()
    }
}

/// A persistence boundary for the kernel-owned state store.
///
/// Implementations may use memory, a v4-owned SQL table, or another host
/// persistence mechanism. The raw persistence object is never placed in a
/// PluginContext; plugins receive only a namespace-scoped handle.
pub trait PluginStatePersistence: Send + Sync {
    fn load(&self) -> Result<PluginStateSnapshot, PluginStateError>;
    fn save(&self, snapshot: &PluginStateSnapshot) -> Result<(), PluginStateError>;
}

#[derive(Clone, Default)]
pub struct InMemoryPluginStatePersistence {
    snapshot: Arc<Mutex<PluginStateSnapshot>>,
}

impl InMemoryPluginStatePersistence {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Result<PluginStateSnapshot, PluginStateError> {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| PluginStateError::LockPoisoned)
    }

    pub fn reopen(snapshot: PluginStateSnapshot) -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(snapshot)),
        }
    }
}

impl PluginStatePersistence for InMemoryPluginStatePersistence {
    fn load(&self) -> Result<PluginStateSnapshot, PluginStateError> {
        self.snapshot()
    }

    fn save(&self, snapshot: &PluginStateSnapshot) -> Result<(), PluginStateError> {
        *self
            .snapshot
            .lock()
            .map_err(|_| PluginStateError::LockPoisoned)? = snapshot.clone();
        Ok(())
    }
}

#[async_trait]
pub trait HostPluginStateApi: Send + Sync {
    async fn get(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Result<Option<PluginStateEntry>, PluginStateError>;

    async fn set(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
        state_format_version: &VersionString,
        value: StrictJsonValue,
    ) -> Result<PluginStateSetResponse, PluginStateError>;

    async fn delete(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Result<PluginStateDeleteResponse, PluginStateError>;

    async fn compare_and_swap(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
        expected_revision: u64,
        state_format_version: &VersionString,
        value: Option<StrictJsonValue>,
    ) -> Result<PluginStateCompareAndSwapOutcome, PluginStateError>;
}

/// Alias used by plugin-facing code and conformance tests.
pub trait PluginState: HostPluginStateApi {}

impl<T> PluginState for T where T: HostPluginStateApi + ?Sized {}

pub(crate) struct PluginStateStore {
    persistence: Arc<dyn PluginStatePersistence>,
    snapshot: Mutex<PluginStateSnapshot>,
}

impl PluginStateStore {
    pub(crate) fn new(
        persistence: Arc<dyn PluginStatePersistence>,
    ) -> Result<Arc<Self>, PluginStateError> {
        let snapshot = persistence.load()?;
        Ok(Arc::new(Self {
            persistence,
            snapshot: Mutex::new(snapshot),
        }))
    }

    #[cfg(test)]
    pub(crate) fn from_snapshot(
        snapshot: PluginStateSnapshot,
        persistence: Arc<dyn PluginStatePersistence>,
    ) -> Arc<Self> {
        Arc::new(Self {
            persistence,
            snapshot: Mutex::new(snapshot),
        })
    }

    pub(crate) fn handle(
        self: &Arc<Self>,
        package_id: PackageId,
        mount_id: PluginMountId,
        writer_package_version: VersionString,
    ) -> PluginStateHandle {
        PluginStateHandle {
            store: Arc::clone(self),
            writer_package_version,
            descriptor: PluginStateHandleDescriptor {
                package_id,
                mount_id,
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
        }
    }

    fn identity(
        &self,
        package_id: &PackageId,
        mount_id: &PluginMountId,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Result<StateIdentity, PluginStateError> {
        validate_key(scope_key.as_ref())?;
        validate_key(state_key.as_ref())?;
        Ok(StateIdentity {
            package_id: package_id.clone(),
            mount_id: mount_id.clone(),
            scope_key: scope_key.clone(),
            state_key: state_key.clone(),
        })
    }

    fn persist(
        &self,
        snapshot: &mut PluginStateSnapshot,
        previous: PluginStateSnapshot,
    ) -> Result<(), PluginStateError> {
        if let Err(error) = self.persistence.save(snapshot) {
            *snapshot = previous;
            return Err(error);
        }
        Ok(())
    }

    fn set_value(
        &self,
        identity: StateIdentity,
        writer_package_version: &VersionString,
        state_format_version: &VersionString,
        value: StrictJsonValue,
    ) -> Result<u64, PluginStateError> {
        validate_format_version(state_format_version)?;
        validate_value(&value)?;
        let mut snapshot = self
            .snapshot
            .lock()
            .map_err(|_| PluginStateError::LockPoisoned)?;
        let previous = snapshot.clone();
        let revision = snapshot
            .revisions
            .get(&identity)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(PluginStateError::RevisionExhausted)?;
        snapshot.revisions.insert(identity.clone(), revision);
        snapshot.entries.insert(
            identity.clone(),
            PluginStateEntry {
                namespace: identity.namespace(),
                revision,
                state_format_version: state_format_version.clone(),
                writer_package_version: writer_package_version.clone(),
                value,
            },
        );
        self.persist(&mut snapshot, previous)?;
        Ok(revision)
    }

    fn delete_value(&self, identity: StateIdentity) -> Result<bool, PluginStateError> {
        let mut snapshot = self
            .snapshot
            .lock()
            .map_err(|_| PluginStateError::LockPoisoned)?;
        if !snapshot.entries.contains_key(&identity) {
            return Ok(false);
        }
        let previous = snapshot.clone();
        let revision = snapshot
            .revisions
            .get(&identity)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(PluginStateError::RevisionExhausted)?;
        snapshot.entries.remove(&identity);
        snapshot.revisions.insert(identity, revision);
        self.persist(&mut snapshot, previous)?;
        Ok(true)
    }

    fn cas_value(
        &self,
        identity: StateIdentity,
        expected_revision: u64,
        writer_package_version: &VersionString,
        state_format_version: &VersionString,
        value: Option<StrictJsonValue>,
    ) -> Result<PluginStateCompareAndSwapOutcome, PluginStateError> {
        validate_format_version(state_format_version)?;
        if let Some(value) = &value {
            validate_value(value)?;
        }
        let mut snapshot = self
            .snapshot
            .lock()
            .map_err(|_| PluginStateError::LockPoisoned)?;
        let current_revision = snapshot
            .revisions
            .get(&identity)
            .copied()
            .unwrap_or(0);
        if current_revision != expected_revision {
            return Ok(PluginStateCompareAndSwapOutcome::Conflict {
                current_revision,
                current_value: snapshot
                    .entries
                    .get(&identity)
                    .map(|entry| entry.value.clone()),
            });
        }

        let previous = snapshot.clone();
        let revision = current_revision
            .checked_add(1)
            .ok_or(PluginStateError::RevisionExhausted)?;
        snapshot.revisions.insert(identity.clone(), revision);
        if let Some(value) = value {
            snapshot.entries.insert(
                identity.clone(),
                PluginStateEntry {
                    namespace: identity.namespace(),
                    revision,
                    state_format_version: state_format_version.clone(),
                    writer_package_version: writer_package_version.clone(),
                    value,
                },
            );
        } else {
            snapshot.entries.remove(&identity);
        }
        self.persist(&mut snapshot, previous)?;
        Ok(PluginStateCompareAndSwapOutcome::Applied { revision })
    }
}

#[derive(Clone)]
pub struct PluginStateHandle {
    store: Arc<PluginStateStore>,
    writer_package_version: VersionString,
    descriptor: PluginStateHandleDescriptor,
}

impl PluginStateHandle {
    pub fn descriptor(&self) -> &PluginStateHandleDescriptor {
        &self.descriptor
    }

    pub fn namespace(
        &self,
        scope_key: ScopeKey,
        state_key: StateKey,
    ) -> PluginStateNamespace {
        PluginStateNamespace {
            package_id: self.descriptor.package_id.clone(),
            mount_id: self.descriptor.mount_id.clone(),
            scope_key,
            state_key,
        }
    }

    pub async fn get_request(
        &self,
        request: nomifun_agent_contracts::PluginStateGetRequest,
    ) -> Result<nomifun_agent_contracts::PluginStateGetResponse, PluginStateError> {
        Ok(nomifun_agent_contracts::PluginStateGetResponse {
            entry: self.get(&request.scope_key, &request.state_key).await?,
        })
    }

    pub async fn set_request(
        &self,
        request: nomifun_agent_contracts::PluginStateSetRequest,
    ) -> Result<PluginStateSetResponse, PluginStateError> {
        self.set(
            &request.scope_key,
            &request.state_key,
            &request.state_format_version,
            request.value,
        )
        .await
    }

    pub async fn delete_request(
        &self,
        request: nomifun_agent_contracts::PluginStateDeleteRequest,
    ) -> Result<PluginStateDeleteResponse, PluginStateError> {
        self.delete(&request.scope_key, &request.state_key).await
    }

    pub async fn compare_and_swap_request(
        &self,
        request: nomifun_agent_contracts::PluginStateCompareAndSwapRequest,
    ) -> Result<PluginStateCompareAndSwapOutcome, PluginStateError> {
        self.compare_and_swap(
            &request.scope_key,
            &request.state_key,
            request.expected_revision,
            &request.state_format_version,
            request.value,
        )
        .await
    }
}

#[async_trait]
impl HostPluginStateApi for PluginStateHandle {
    async fn get(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Result<Option<PluginStateEntry>, PluginStateError> {
        let identity = self.store.identity(
            &self.descriptor.package_id,
            &self.descriptor.mount_id,
            scope_key,
            state_key,
        )?;
        let snapshot = self
            .store
            .snapshot
            .lock()
            .map_err(|_| PluginStateError::LockPoisoned)?;
        Ok(snapshot.entries.get(&identity).cloned())
    }

    async fn set(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
        state_format_version: &VersionString,
        value: StrictJsonValue,
    ) -> Result<PluginStateSetResponse, PluginStateError> {
        let identity = self.store.identity(
            &self.descriptor.package_id,
            &self.descriptor.mount_id,
            scope_key,
            state_key,
        )?;
        Ok(PluginStateSetResponse {
            revision: self.store.set_value(
                identity,
                &self.writer_package_version,
                state_format_version,
                value,
            )?,
        })
    }

    async fn delete(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Result<PluginStateDeleteResponse, PluginStateError> {
        let identity = self.store.identity(
            &self.descriptor.package_id,
            &self.descriptor.mount_id,
            scope_key,
            state_key,
        )?;
        Ok(PluginStateDeleteResponse {
            deleted: self.store.delete_value(identity)?,
        })
    }

    async fn compare_and_swap(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
        expected_revision: u64,
        state_format_version: &VersionString,
        value: Option<StrictJsonValue>,
    ) -> Result<PluginStateCompareAndSwapOutcome, PluginStateError> {
        let identity = self.store.identity(
            &self.descriptor.package_id,
            &self.descriptor.mount_id,
            scope_key,
            state_key,
        )?;
        self.store.cas_value(
            identity,
            expected_revision,
            &self.writer_package_version,
            state_format_version,
            value,
        )
    }
}

fn validate_key(value: &str) -> Result<(), PluginStateError> {
    if value.is_empty() {
        return Err(PluginStateError::EmptyKey);
    }
    if value.len() > MAX_PLUGIN_STATE_KEY_BYTES {
        return Err(PluginStateError::KeyTooLarge);
    }
    Ok(())
}

fn validate_format_version(value: &VersionString) -> Result<(), PluginStateError> {
    if value.as_ref().is_empty() {
        Err(PluginStateError::EmptyFormatVersion)
    } else {
        Ok(())
    }
}

fn validate_value(value: &StrictJsonValue) -> Result<(), PluginStateError> {
    let bytes = serde_json::to_vec(&value.0).map_err(|_| PluginStateError::InvalidJson)?;
    if bytes.len() > MAX_PLUGIN_STATE_BYTES {
        return Err(PluginStateError::ValueTooLarge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nomifun_agent_contracts::{PluginStateCompareAndSwapOutcome, StrictJsonValue};
    use serde_json::json;

    use super::*;

    fn handle(
        persistence: Arc<InMemoryPluginStatePersistence>,
    ) -> PluginStateHandle {
        let store = PluginStateStore::new(persistence).unwrap();
        store.handle(
            PackageId::from("sample.echo"),
            PluginMountId::from("sample-echo"),
            VersionString::from("1.0.0"),
        )
    }

    #[tokio::test]
    async fn state_has_four_methods_and_cas_conflict_is_non_destructive() {
        let persistence = Arc::new(InMemoryPluginStatePersistence::new());
        let state = handle(persistence);
        assert_eq!(state.descriptor().methods.len(), 4);
        let scope = ScopeKey::from("session-a");
        let key = StateKey::from("counter");
        let format = VersionString::from("1.0.0");

        let first = state
            .set(&scope, &key, &format, StrictJsonValue(json!({"n": 1})))
            .await
            .unwrap();
        assert_eq!(first.revision, 1);

        let conflict = state
            .compare_and_swap(
                &scope,
                &key,
                0,
                &format,
                Some(StrictJsonValue(json!({"n": 2}))),
            )
            .await
            .unwrap();
        assert_eq!(
            conflict,
            PluginStateCompareAndSwapOutcome::Conflict {
                current_revision: 1,
                current_value: Some(StrictJsonValue(json!({"n": 1}))),
            }
        );
        assert_eq!(
            state.get(&scope, &key).await.unwrap().unwrap().revision,
            1
        );
    }

    #[tokio::test]
    async fn state_reopens_from_persistent_snapshot_and_delete_advances_revision() {
        let persistence = Arc::new(InMemoryPluginStatePersistence::new());
        let state = handle(Arc::clone(&persistence));
        let scope = ScopeKey::from("session-a");
        let key = StateKey::from("value");
        let format = VersionString::from("1.0.0");
        state
            .set(&scope, &key, &format, StrictJsonValue(json!("before")))
            .await
            .unwrap();
        let snapshot = persistence.snapshot().unwrap();

        let reopened_store = PluginStateStore::from_snapshot(
            snapshot,
            Arc::clone(&persistence) as Arc<dyn PluginStatePersistence>,
        );
        let reopened = reopened_store.handle(
            PackageId::from("sample.echo"),
            PluginMountId::from("sample-echo"),
            VersionString::from("1.0.0"),
        );
        assert_eq!(
            reopened.get(&scope, &key).await.unwrap().unwrap().value,
            StrictJsonValue(json!("before"))
        );
        assert!(
            reopened
                .delete(&scope, &key)
                .await
                .unwrap()
                .deleted
        );
        let applied = reopened
            .compare_and_swap(
                &scope,
                &key,
                1,
                &format,
                Some(StrictJsonValue(json!("after"))),
            )
            .await
            .unwrap();
        assert_eq!(
            applied,
            PluginStateCompareAndSwapOutcome::Conflict {
                current_revision: 2,
                current_value: None,
            }
        );
    }

    #[tokio::test]
    async fn state_isolated_by_package_mount_scope_and_key() {
        let persistence = Arc::new(InMemoryPluginStatePersistence::new());
        let store = PluginStateStore::new(persistence).unwrap();
        let first = store.handle(
            PackageId::from("sample.echo"),
            PluginMountId::from("sample-echo"),
            VersionString::from("1.0.0"),
        );
        let other = store.handle(
            PackageId::from("other.package"),
            PluginMountId::from("other-mount"),
            VersionString::from("1.0.0"),
        );
        let scope = ScopeKey::from("same-scope");
        let key = StateKey::from("same-key");
        let format = VersionString::from("1.0.0");
        first
            .set(&scope, &key, &format, StrictJsonValue(json!("first")))
            .await
            .unwrap();
        other
            .set(&scope, &key, &format, StrictJsonValue(json!("other")))
            .await
            .unwrap();
        assert_eq!(
            first.get(&scope, &key).await.unwrap().unwrap().value,
            StrictJsonValue(json!("first"))
        );
        assert_eq!(
            other.get(&scope, &key).await.unwrap().unwrap().value,
            StrictJsonValue(json!("other"))
        );
    }
}
