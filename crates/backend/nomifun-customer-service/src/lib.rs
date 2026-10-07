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
    CUSTOMER_SERVICE_AGENT_ACTION_IDS, CUSTOMER_SERVICE_HANDOFF_ACTION_ID,
    CUSTOMER_SERVICE_MODULE_ID, CUSTOMER_SERVICE_NOTES_READ_ACTION_ID,
    CUSTOMER_SERVICE_NOTES_WRITE_ACTION_ID, customer_service_action_input_schema,
    customer_service_action_resource_operation,
};
pub use dialogue::{
    CsDialogueEngine, CustomerServiceAgentPolicy, CustomerServiceAgentPolicyResolver,
    LiveTurnRunner, TurnRunner,
};
pub use routes::{CustomerServiceRouterState, customer_service_routes};
pub use service::{
    AgentCsNoteWriteInput, CreateCsAgentInput, CustomerServiceService, RequestCsHandoffInput,
};
