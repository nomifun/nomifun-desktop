use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    SessionEventKind, SessionEventPersistence, SessionEventRegistryEntry,
    SessionEventRegistryPayload,
};

use crate::error::SessionStoreError;

const CANONICAL_SESSION_EVENT_REGISTRY: &str =
    include_str!("../../nomifun-agent-contracts/contracts/events/session-event-registry.json");

#[derive(Clone, Debug)]
pub(crate) struct EventRegistry {
    payload: SessionEventRegistryPayload,
    entries: BTreeMap<(String, u32), SessionEventRegistryEntry>,
}

impl EventRegistry {
    pub(crate) fn canonical() -> Result<Self, SessionStoreError> {
        let payload: SessionEventRegistryPayload =
            serde_json::from_str(CANONICAL_SESSION_EVENT_REGISTRY)?;
        payload
            .validate()
            .map_err(|error| SessionStoreError::Registry(error.to_string()))?;

        let entries = payload
            .entries
            .iter()
            .cloned()
            .map(|entry| ((entry.kind.0.clone(), entry.version), entry))
            .collect();
        Ok(Self { payload, entries })
    }

    pub(crate) fn payload(&self) -> &SessionEventRegistryPayload {
        &self.payload
    }

    pub(crate) fn entry(
        &self,
        kind: &SessionEventKind,
        version: u32,
    ) -> Result<&SessionEventRegistryEntry, SessionStoreError> {
        self.entries.get(&(kind.0.clone(), version)).ok_or_else(|| {
            SessionStoreError::InvalidEvent(format!(
                "unregistered event kind/version {}/{}",
                kind.0, version
            ))
        })
    }

    pub(crate) fn is_transient(
        &self,
        kind: &SessionEventKind,
        version: u32,
    ) -> Result<bool, SessionStoreError> {
        Ok(matches!(
            self.entry(kind, version)?.persistence,
            SessionEventPersistence::TransientDiagnostic
        ))
    }
}
