//! All HTTP request/response DTOs shared across the API surface.
mod agent_build_extra;
mod agent_discovery;
mod agent_error;
mod agent_execution;
mod agent_execution_template;
mod agent_platform;
mod auth;
mod channel;
mod connection_test;
mod conversation;
mod cron;
mod custom_agent;
mod extension;
mod file;
mod idmm;
mod knowledge;
mod lifecycle;
mod managed_model;
mod mcp;
mod mcp_bridge;
mod model_capability;
pub mod model_protocol;
pub mod model_task;
mod office;
mod preset;
mod provider;
mod provider_connection;
mod provider_model;
mod requirement;
mod response;
mod serde_util;
mod session_ops;
mod shell;
mod skill;
mod system;
mod terminal;
mod webhook;
mod websocket;

pub use session_ops::{
    GetModelInfoResponse, ModelInfoEntry, ModelInfoPayload, SetModelRequest, SideQuestionRequest,
    SideQuestionResponse, WorkspaceBrowseQuery, WorkspaceEntry,
};
pub use agent_build_extra::{
    NomiBuildExtra, NomiGoalSpec, SessionMcpServer, SessionMcpTransport, SlashCommandItem,
    SummonConfig,
};
pub use agent_discovery::{
    AgentEnvEntry, AgentHandshake, AgentMetadata, AgentSource, AgentSourceInfo, BehaviorPolicy,
};
pub use agent_error::{
    AgentErrorCode, AgentErrorOwnership, AgentErrorResolution, AgentErrorResolutionKind,
    AgentErrorResolutionTarget, AgentStreamErrorData,
};
pub use agent_execution::{
    AddExecutionStepsRequest, AdjustAgentExecutionRequest, AdoptExecutionStepOutputRequest,
    AgentExecution, AgentExecutionChangedEvent, AgentExecutionDetail, AgentExecutionEvent,
    AgentExecutionEventsQuery, AnswerExecutionDecisionRequest, ConfigureExecutionStepRequest,
    CreateAgentExecutionRequest, ExecutionAttempt, ExecutionModelPool, ExecutionModelRef,
    ExecutionParticipant, ExecutionStep, ExecutionStepDependency, ExecutionStepProfile,
    JudgeAggregation, LoopStopPolicy, ParticipantCapability, ParticipantConstraints,
    PlannedExecution, PlannedExecutionStep, ReassignExecutionStepRequest,
    RenameAgentExecutionRequest, ReplanAgentExecutionRequest, RetryExecutionStepRequest,
    SteerExecutionStepRequest, StepControlPolicy, UpdateExecutionStepRequest, VerificationPolicy,
    VersionedAgentExecutionCommand,
};
pub use agent_execution_template::{
    AgentExecutionTemplate, AgentExecutionTemplateDetail, AgentExecutionTemplateParticipant,
    AgentExecutionTemplateParticipantInput, CreateAgentExecutionTemplateRequest,
    CreateExecutionFromTemplateRequest, UpdateAgentExecutionTemplateRequest,
};
pub use agent_platform::*;
pub use auth::{
    AuthStatusResponse, ChangePasswordRequest, ChangeUsernameRequest, ChangeUsernameResponse,
    LoginRequest, LoginResponse, PublicUser, QrLoginRequest, RefreshResponse, RefreshTokenRequest,
    UserInfoResponse, WebuiChangePasswordRequest, WebuiChangeUsernameRequest,
    WebuiChangeUsernameResponse, WebuiGenerateQrTokenResponse, WebuiResetPasswordResponse,
    WsTokenResponse,
};
pub use channel::{
    ApprovePairingRequest, BridgeResponse, CHANNEL_OWNER_DOMAIN_COMPANION,
    CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE, ChannelSessionResponse, ChannelUserResponse,
    DisablePluginRequest, EnablePluginRequest, EnablePluginResponse, PairingRequestResponse,
    GroupAccessMode, PairingRequestedPayload, PluginStatusChangedPayload, PluginStatusResponse,
    RejectPairingRequest, RevokeUserRequest, SetGroupAccessRequest, SyncChannelSettingsRequest,
    TestPluginExtraConfig, TestPluginRequest, TestPluginResponse, UserAuthorizedPayload,
};
pub use connection_test::TestBedrockConnectionRequest;
pub use conversation::{
    ActiveCountResponse, CloneConversationRequest, ConversationArtifactKind,
    ConversationArtifactListResponse, ConversationArtifactResponse, ConversationArtifactStatus,
    ConversationListResponse, ConversationMcpStatus, ConversationMcpStatusKind,
    ConversationResponse, ConversationRuntimeStateKind, ConversationRuntimeSummary,
    CreateConversationRequest, ListConversationsQuery, ListMessagesQuery, MessageListResponse,
    MessageResponse, MessageSearchItem, MessageSearchResponse, SearchMessagesQuery,
    SendMessageRequest, SendMessageResponse, UpdateConversationArtifactRequest,
    UpdateConversationRequest,
};
pub use cron::{
    CreateCronJobRequest, CronAgentConfigDto, CronJobExecutedEvent, CronJobMetadataDto,
    CronJobRemovedPayload, CronJobResponse, CronJobRunResponse, CronJobStateDto, CronScheduleDto,
    HasSkillResponse, ListCronJobsQuery, RunNowResponse, SaveCronSkillRequest,
    UpdateCronJobRequest,
};
pub use custom_agent::{
    CustomAgentAdvancedOverrides, CustomAgentUpsertRequest, DeleteCustomAgentResponse,
    SetEnabledRequest,
};
pub use extension::{
    DisableExtensionRequest, EnableExtensionRequest, ExtensionSummaryResponse, GetI18nRequest,
    GetPermissionsRequest, GetRiskLevelRequest, HubExtensionListItem, HubOperationResponse,
    HubUpdateInfo, InstallExtensionRequest, PermissionDetailResponse, PermissionSummaryResponse,
};
pub use file::{
    BrowseDirectoryQuery, BrowseDirectoryResponse, BrowseEntry, CancelZipRequest, CopyFilesRequest,
    CopyFilesResponse, CreateDirectoryRequest, FetchRemoteImageRequest, FileChangeInfoResponse,
    FileMetadataResponse, FileWatchRequest, GetFileMetadataRequest, GetImageBase64Request,
    ListWorkspaceFilesRequest, ReadFileRequest, RemoveEntryRequest, RenameRequest, RenameResponse,
    SnapshotBaselineRequest, SnapshotCompareResponse, SnapshotDiscardRequest, SnapshotInfoResponse,
    SnapshotMode, SnapshotStageRequest, SnapshotWorkspaceRequest, WorkspaceFlatFileResponse,
    WorkspaceOfficeWatchRequest, WriteFileRequest, ZipFileEntry, ZipRequest,
};
pub use idmm::{
    BlockedBehavior, BudgetConfig, BypassModelRef, CategoryMode, CategoryRules, DecisionStrategy,
    DecisionWatchConfig, FaultWatchConfig, IdmmConfig, IdmmRunState, IdmmState, IdmmTargetKind,
    InterventionRecord, ModelFailoverConfig, OpenQuestionRule, OptionRule, ScanScope,
    SetIdmmRequest, Tendency, WakeStrategy, WatchBase, WatchTier,
};
pub use knowledge::{
    CreateKnowledgeTagRequest, KnowledgeEmbeddingConfig, KnowledgeEntry, KnowledgeEntryKind,
    KnowledgeEntryCapabilities, KnowledgeEntryOrigin, KnowledgeEntrySourceInfo,
    KnowledgeEntrySourceRelationship, KnowledgeMountInfo, KnowledgeRerankConfig,
    KnowledgeRetrievalConfig, KnowledgeSource, KnowledgeSourceEntry, KnowledgeSourceMode,
    KnowledgeSourceSyncStatus, KnowledgeTag, KnowledgeTreeAccess,
    RelocateKnowledgeEntryConflictPolicy, RelocateKnowledgeEntryRequest,
    RelocateKnowledgeEntryResponse, UndoKnowledgeEntryRelocationRequest,
    UpdateKnowledgeTagRequest,
};
pub use lifecycle::{
    GitHubReleaseAsset, SystemInfoResponse, UpdateCheckRequest, UpdateCheckResult,
    UpdateReleaseInfo, UpdateWorkDirRequest,
};
pub use managed_model::{
    ManagedModel, ManagedModelHealthBatchResult, ManagedModelHealthErrorKind,
    ManagedModelHealthResult, ManagedModelHealthStatus, ManagedModelServiceAvailability,
    ManagedModelServiceStatus, SetManagedModelEnabledRequest, SetManagedModelServiceEnabledRequest,
};
pub use mcp::{
    BatchImportMcpServersRequest, CreateMcpServerRequest, DetectedMcpServerEntry,
    DetectedMcpServerResponse, ImportMcpServerRequest, McpAuthMethod, McpConnectionTestErrorCode,
    McpConnectionTestResult, McpServerId, McpServerResponse, McpToolResponse, McpTransport,
    OAuthCheckStatusRequest, OAuthLoginRequest, OAuthLoginResponse, OAuthLogoutRequest,
    OAuthStatusResponse, TestMcpConnectionRequest, UpdateMcpServerRequest,
};
pub use mcp_bridge::{
    GATEWAY_CALL_TOOL_OPERATION, GATEWAY_CAPABILITY_DOMAIN, GATEWAY_CREATE_CONVERSATION_TOOL,
    GATEWAY_LIST_TOOLS_OPERATION,
    GatewayCapabilityClaims, GatewayCapabilityScope, GatewayMcpChildConfig, GatewayMcpConfig,
    KNOWLEDGE_CAPABILITY_DOMAIN, KNOWLEDGE_READ_TOOL, KNOWLEDGE_SEARCH_TOOL, KNOWLEDGE_WRITE_TOOL,
    KnowledgeCapabilityClaims, KnowledgeCapabilityScope, KnowledgeMcpChildConfig,
    KnowledgeMcpConfig, OpenMcpConfig,
    REQUIREMENT_CAPABILITY_DOMAIN, REQUIREMENT_COMPLETE_TOOL, REQUIREMENT_UPDATE_STATUS_TOOL,
    RequirementCapabilityClaims, RequirementCapabilityScope, RequirementMcpChildConfig,
    RequirementMcpConfig, ScopedMcpChildBootstrap, ScopedMcpChildConfig,
};
pub use model_protocol::{
    AuthSchemeDescriptor, EndpointRootShape, ModelProtocolManifestResponse,
    PlatformPresetDescriptor, ProtocolDefaultConnection, ProtocolDescriptor,
    ProtocolEndpointDescriptor, ProtocolEndpointPurpose, ProtocolExecutorKind,
    ProtocolRecommendation, ProtocolScope, ProtocolTaskDescriptor, ProtocolTransportKind,
};
pub use model_task::{ModelTask, ModelTrait, infer_catalog_tasks_and_traits};
pub use office::{
    GetSnapshotContentRequest, ListSnapshotsRequest,
    PREVIEW_CAPABILITY_BYTES, PREVIEW_CAPABILITY_HEX_LEN, PreviewHistoryTargetDto,
    PreviewSnapshotInfoDto, PreviewState, PreviewStatusEvent, PreviewUrlResponse,
    SaveSnapshotRequest, SnapshotContentResponse, StartPreviewRequest,
    StopPreviewRequest, is_preview_capability,
};
pub use preset::{
    AgentPreference, CreatePresetRequest, CreatePresetTagRequest, ImportPresetsRequest,
    ImportPresetsResult, KnowledgeBaseBinding, ModelPreference, PresetImportError,
    PresetKnowledgePolicy, PresetOverrides, PresetResponse, PresetSource, PresetTagDimension,
    PresetTagResponse, PresetTarget, ResolvePresetRequest, ResolvedPresetSnapshot,
    SetPresetStateRequest, SkillBinding, UpdatePresetRequest, UpdatePresetTagRequest,
};
pub use provider::{
    BedrockAuthMethod, BedrockConfig, CloneProviderRequest, CreateProviderRequest,
    FetchModelsAnonymousRequest, FetchModelsRequest, FetchModelsResponse, HealthStatus, ModelInfo,
    ProbeCandidateResult, ProbeProviderConnectionAnonymousRequest, ProbeProviderConnectionRequest,
    ProbeProviderConnectionResponse, ProviderHealthCheckErrorKind, ProviderHealthCheckRequest,
    ProviderHealthCheckResponse, ProviderReachability, ProviderResponse, UpdateProviderRequest,
};
pub use provider_connection::{
    ProviderConnectionInput, ProviderConnectionResponse, SaveProviderConnectionRequest,
};
pub use provider_model::{
    CapabilityHealth, ProviderModelCapabilityInput, ProviderModelCapabilityResponse,
    ProviderModelInput, ProviderModelKeyRequest, ProviderModelResponse, SaveProviderModelRequest,
    validate_model_traits_unique,
};
pub use requirement::{
    AttachmentDto, AutoWorkConfigRequest, AutoWorkRunState, AutoWorkState, AutoWorkTargetKind,
    BatchDeleteRequest, BatchDeleteResponse, BoardResponse, CompleteRequest,
    CreateRequirementRequest, ListRequirementsQuery, NewAttachmentRef, Requirement,
    RequirementDeletedPayload, RequirementStatus, ResumeTagRequest, TagPausedPayload, TagSummary,
    UpdateRequirementRequest, UpdateStatusRequest,
};
pub use response::{ApiResponse, ErrorResponse};
pub use shell::{
    CheckToolInstalledRequest, CheckToolInstalledResponse, OpenExternalRequest, OpenFileRequest,
    OpenFolderWithRequest, ShowItemInFolderRequest, SpeechToTextConfig, SpeechToTextResult,
    TEXT_TO_SPEECH_PREFERENCE_KEY, TextToSpeechConfig, ToolType, TtsApiRequest,
};
pub use skill::{
    AddExternalPathRequest, BuiltinAutoSkillResponse, ExportSkillRequest,
    ExternalSkillSourceResponse, ImportSkillRequest, ImportSkillResponse, MaterializeSkillsRequest,
    MaterializeSkillsResponse, MaterializedSkillRef, NamedPathResponse, ReadBuiltinResourceRequest,
    ReadPresetRuleRequest, ReadSkillInfoRequest, ReadSkillInfoResponse, RemoveExternalPathRequest,
    ScanForSkillsRequest, ScanForSkillsResponse, ScannedSkillResponse, SetSkillTagsRequest,
    SkillListItemResponse, SkillMarketItemResponse, SkillMarketMcpConfigRequest,
    SkillMarketMcpConfigResponse, SkillMarketSyncRequest, SkillMarketSyncResponse,
    SkillPathsResponse, SkillSourceResponse, WritePresetRuleRequest,
};
pub use system::{
    ClientPreferencesResponse, SystemSettingsResponse, UpdateClientPreferencesRequest,
    UpdateSettingsRequest,
};
pub use terminal::{
    CreateTerminalRequest, TerminalExitEvent, TerminalInputRequest, TerminalOutputEvent,
    TerminalRemovedPayload, TerminalResizeRequest, TerminalSessionResponse, UpdateTerminalRequest,
};
pub use webhook::{
    CreateWebhookRequest, TagBinding, TagBindings, TagSetting, UpdateWebhookRequest,
    UpsertTagSettingRequest, Webhook, WebhookId, WebhookPlatform,
};
pub use websocket::WebSocketMessage;

#[cfg(test)]
mod public_contract_tests {
    use super::{AgentErrorResolution, AgentErrorResolutionKind, AgentErrorResolutionTarget};

    #[test]
    fn error_resolution_types_are_exported_from_crate_root() {
        let resolution = AgentErrorResolution::new(
            AgentErrorResolutionKind::Retry,
            Some(AgentErrorResolutionTarget::Feedback),
        );

        assert_eq!(resolution.kind, AgentErrorResolutionKind::Retry);
        assert_eq!(
            resolution.target,
            Some(AgentErrorResolutionTarget::Feedback)
        );
    }
}
