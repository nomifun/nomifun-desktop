//! 客服独立域 (customer-service domain).
//!
//! A standalone domain for serving strangers over IM channels. It shares NO
//! concepts with the desktop companion/conversation system: dialogues are the
//! domain's own aggregate, replies are produced by a disposable one-shot
//! engine session whose tool registry is fixed at construction time to three
//! read-only tools.

pub mod agent_capability;
pub mod dialogue;
pub mod routes;
pub mod service;
pub mod tools;

pub use agent_capability::{
    CustomerServiceAgentCapabilityOwner, CustomerServiceDialogueContext,
    customer_service_action_input_schema,
};
pub use dialogue::{CsDialogueEngine, LiveTurnRunner, TurnRunner};
pub use routes::{CustomerServiceRouterState, customer_service_routes};
pub use service::{
    AgentCsNoteWriteInput, CreateCsAgentInput, CustomerServiceService, RequestCsHandoffInput,
};
