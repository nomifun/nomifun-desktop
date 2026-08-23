//! Shared primitives: error types, enums, ID generation, crypto, timestamps, and pagination.
pub mod ansi;
pub mod constants;
pub mod agent_execution;

mod case_convert;
mod crypto;
pub mod dir_config;
pub mod dataset_roots;
mod enums;
mod error;
mod execution_authority;
pub mod factory_reset;
mod fsname;
mod hooks;
mod id;
mod idempotency;
pub mod miniapp_workspace;
mod scoped_auth;
mod pagination;
pub mod paths;
pub mod provider_lifecycle;
pub mod provider_usage;
pub mod stage_direction;
pub mod text_search;
mod timestamp;
mod types;
pub mod vision_registry;
pub mod zip_safe;

pub use case_convert::{camel_to_snake, normalize_keys_to_snake_case};
pub use constants::NOMI_AGENT_ID;
pub use idempotency::{MAX_IDEMPOTENCY_KEY_LEN, is_visible_ascii_key, required_idempotency_key};
pub use agent_execution::{
    AdaptationPolicy, AgentExecutionActor, AgentExecutionActorType, AgentExecutionEventKind,
    AgentExecutionReceipt, AgentExecutionStatus, AgentStepMode, AgentToolPolicy, ConversationExecutionRelation, DecisionPolicy,
    DelegationPolicy, ExecutionAttemptStatus, ExecutionStepKind, ExecutionStepStatus,
    ParticipantAssignmentSource, PlanGate, StepFailurePolicy, UnknownAgentExecutionValue,
    MAX_AGENT_DELEGATION_DEPTH, MAX_AGENT_EXECUTION_MODELS,
    MAX_AGENT_EXECUTION_PARALLELISM, MAX_AGENT_EXECUTION_PARTICIPANTS,
    MAX_AGENT_EXECUTION_STEPS,
};
pub use nomi_types::agent::{
    AgentDelegationTask, ParallelDelegationRequest, ParallelDelegationStrategy,
    apply_agent_role_context,
};
pub use crypto::{decrypt_string, encrypt_string};
pub use enums::{
    AgentKillReason, AgentType, ConversationSource, ConversationStatus, FileChangeOperation, McpServerStatus,
    McpSource, MessagePosition, MessageStatus, MessageType, PreviewContentType, ProtocolType,
};
pub use error::{AppError, ErrorChain, workspace_path_has_edge_whitespace_segment};
pub use execution_authority::ExecutionAuthority;
pub use fsname::sanitize_dir_segment;
pub use hooks::{OnConversationDelete, OnTerminalDelete, RequirementCreator};
pub use id::{
    AgentExecutionAttemptId, AgentExecutionId, AgentExecutionParticipantId,
    AgentExecutionStepId, AgentExecutionTemplateId, AgentExecutionTemplateParticipantId,
    AgentId, AttachmentId, ChannelPendingPromptId, ChannelPluginId, ChannelSessionId, ChannelUserId,
    CompanionEventId, CompanionEvolutionFeedbackId, CompanionId,
    CompanionMemoryId, CompanionSessionWindowId, CompanionSkillId, CompanionSkillPatternId,
    ConversationArtifactId, ConversationId, CreationTaskId, CreativeStudioConnectionId,
    CreativeStudioCanvasId, CreativeStudioNodeId, CreativeStudioProjectId,
    CreativeStudioTemplateId, CreativeStudioTemplateRunId, CreativeStudioTemplateStepId,
    CronJobId, CronJobRunId,
    CsAgentId, CsNoteId,
    EntityId, FigureId,
    IdmmInterventionId, KnowledgeBaseId, KnowledgeBindingId, McpServerId, MessageId,
    MiniAppId,
    PersistedArtifactId, PresetId, PresetTagId, PreviewSnapshotId, ProviderId,
    RemoteAgentId, TerminalId, UUID_STRING_LEN,
    RequirementId, SshHostId, UserId, UuidV7Error, WebhookId,
    WorkshopAssetId, generate_id, validate_uuidv7,
};
pub use scoped_auth::{
    LOOPBACK_CAPABILITY_RENEW_PATH, LOOPBACK_CAPABILITY_RENEWAL_MARGIN_SECS,
    LOOPBACK_CAPABILITY_REVOKE_PATH, LOOPBACK_CAPABILITY_TTL_SECS,
    LOOPBACK_CAPABILITY_VERSION, LoopbackCapabilityAccess,
    LoopbackCapabilityClaims, LoopbackCapabilityError, LoopbackCapabilityIssuer,
    LoopbackCapabilityLease, LoopbackCapabilityLeaseSet,
    LoopbackCapabilityRenewalRequest,
    LoopbackSessionBinding, LoopbackSessionKind, unix_time_secs,
};
pub use pagination::PaginatedResult;
pub use provider_lifecycle::{ProviderLifecycleBarrier, SharedProviderLifecycleBarrier};
pub use provider_usage::{ProviderInUseDetails, ProviderUsage, ProviderUsageFeature};
pub use timestamp::{TimestampMs, now_ms};
pub use types::{CommandSpec, Confirmation, ConfirmationOption, EnvVar, ProviderWithModel};
pub use vision_registry::VisionUnsupportedRegistry;
