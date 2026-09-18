//! Canonical AgentSession product adapters and shared projection types.
mod canonical_session_owner;
mod creation_ingress;
mod creative_studio_session;
mod product_agent;
mod turn_delivery;
pub mod model_failover;
pub use canonical_session_owner::{
    AgentMutationReceipt, AgentTurnReceipt, CanonicalAgentSessionOwner, OpenAgentSession,
    PreparedAgentSessionDelete,
};
pub use creation_ingress::{
    ConversationCreationPage, ConversationCreationResponse, SubmitConversationCreation,
    import_creation_files,
};
pub use creative_studio_session::{
    CreativeStudioAgentHistoryMessage, CreativeStudioAgentHistoryRole,
    CreativeStudioAgentHistoryStatus, CreativeStudioAgentModelRef,
    CreativeStudioCanvasAgentSessionBindingResponse,
    ResolveCreativeStudioCanvasAgentSessionRequest,
    ResolveCreativeStudioCanvasAgentSessionResponse,
};
pub use product_agent::{
    ProductAgentResolution, ProductAgentSnapshotResolver, ProductAgentTarget,
};
pub use turn_delivery::{
    BackgroundTaskRegistrar, IdempotentMessageDelivery, PublicTurnDeliveryState,
};
