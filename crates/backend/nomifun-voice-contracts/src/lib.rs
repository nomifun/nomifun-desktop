//! Optional voice contracts. Agent work identifiers are referenced, never
//! redefined or exported back into the canonical Agent contract namespace.
pub use nomifun_agent_contracts::{
    ActionId,AgentSessionId,EventId,OperationId,SessionEventCursor,DigestHex,
    digest_payload,digest_bytes,
};
pub mod voice;
pub mod work_interaction;
pub mod profile;
pub mod api;
pub use api::*;
pub use profile::*;
pub use voice::*;
pub use work_interaction::*;
