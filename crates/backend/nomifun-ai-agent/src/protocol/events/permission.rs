use nomifun_common::Confirmation;
use serde::{Deserialize, Serialize};

/// Payload of [`super::AgentStreamEvent::Permission`].
///
/// A transparent newtype rather than a bare [`Confirmation`] so the event keeps
/// a named payload type at every call site. This used to be a two-arm untagged
/// enum whose other arm carried an external protocol's own permission-request
/// frame; that arm is gone, and every surviving runtime raises an approval as a
/// `Confirmation` directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionEventData(pub Confirmation);

impl PermissionEventData {
    pub fn confirmation(&self) -> &Confirmation {
        &self.0
    }

    pub fn into_confirmation(self) -> Confirmation {
        self.0
    }
}

impl From<Confirmation> for PermissionEventData {
    fn from(confirmation: Confirmation) -> Self {
        Self(confirmation)
    }
}
