//! Typed Remote ingress platform contract.
//!
//! Remote ingress authenticates an installation owner and routes four explicit
//! operations to an AgentSession. The transport itself is never an Agent grant
//! and cannot expand the target Session's compiled actions.

use serde::Serialize;

pub const REMOTE_INGRESS_PLATFORM_SERVICE_ID: &str = "remote.ingress";
pub const REMOTE_INGRESS_AGENT_ACTION_IDS: [&str; 0] = [];
pub const REMOTE_INGRESS_OPERATION_IDS: [&str; 4] = ["open", "turn", "observe", "cancel"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteIngressTransport {
    Mcp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RemoteIngressBindingSelection {
    Unbound,
    Bound {
        remote_binding_id: String,
        owner_user_id: String,
        transport: RemoteIngressTransport,
    },
}

impl RemoteIngressBindingSelection {
    pub fn bound(
        remote_binding_id: impl Into<String>,
        owner_user_id: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let remote_binding_id = remote_binding_id.into();
        let owner_user_id = owner_user_id.into();
        if remote_binding_id.trim().is_empty() || owner_user_id.trim().is_empty() {
            return Err("Remote binding and owner identities must not be blank");
        }
        if nomifun_common::UserId::try_from(owner_user_id.as_str()).is_err() {
            return Err("Remote binding owner identity is invalid");
        }
        Ok(Self::Bound {
            remote_binding_id,
            owner_user_id,
            transport: RemoteIngressTransport::Mcp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingress_is_a_typed_platform_transport_not_an_agent_grant() {
        assert!(REMOTE_INGRESS_AGENT_ACTION_IDS.is_empty());
        assert_eq!(
            REMOTE_INGRESS_OPERATION_IDS,
            ["open", "turn", "observe", "cancel"]
        );
    }

    #[test]
    fn binding_selection_keeps_bound_and_unbound_states_distinct() {
        let owner = nomifun_common::UserId::new().into_string();
        assert_eq!(
            RemoteIngressBindingSelection::Unbound,
            RemoteIngressBindingSelection::Unbound
        );
        assert!(matches!(
            RemoteIngressBindingSelection::bound("binding-a", owner).unwrap(),
            RemoteIngressBindingSelection::Bound {
                transport: RemoteIngressTransport::Mcp,
                ..
            }
        ));
        assert!(
            RemoteIngressBindingSelection::bound(
                " ",
                nomifun_common::UserId::new().into_string()
            )
            .is_err()
        );
    }
}
