//! SQLite database layer: init, migrations, repository traits, and implementations.
mod database;
mod error;
pub mod models;
mod repository;

pub use database::{Database, init_database, init_database_memory};
pub use error::DbError;
pub use models::{
    AgentExecutionAttemptDetailRow, AgentExecutionAttemptRow, AgentExecutionDetailRows,
    AgentExecutionEventRow, AgentExecutionParticipantRow, AgentExecutionRow,
    AgentExecutionStepDependencyRow, AgentExecutionStepDetailRow, AgentExecutionStepRow,
    AgentExecutionTemplateDetailRows, AgentExecutionTemplateParticipantRow,
    AgentExecutionTemplateRow,
    AgentMetadataRow, ConnectorCredentialRow,
    ConversationArtifactRow,
    CreateKnowledgeTagParams, CreationTaskRow, CronJobRunRow, KnowledgeBaseRow, KnowledgeBindingRow,
    KnowledgeTagRow, SkillTagRow, TagSettingRow, TerminalSessionRow, UpdateAgentHandshakeParams,
    UpdateKnowledgeTagParams,
    UpsertAgentMetadataParams, UpsertSkillTagParams, WebhookRow,
    WorkshopAssetRow, WorkshopCanvasRow, ConversationExecutionLinkRow,
};
pub use models::{
    CreatePresetTagParams, PresetAgentPreferenceRow, PresetExampleRow,
    PresetKnowledgeBaseRow, PresetKnowledgePolicyRow, PresetLocalizationRow,
    PresetModelPreferenceRow, PresetRecord, PresetRow, PresetSkillBindingRow,
    PresetTagBindingRow, PresetTagRow, PresetUserStateRow, PresetWriteParams,
    UpdatePresetTagParams, UpsertPresetStateParams,
};
pub use models::{ModelProfileRow, UpsertModelProfileParams};
pub use repository::channel::UpdatePluginStatusParams;
pub use repository::conversation::{
    ConversationFilters, ConversationMessageProjection, ConversationRowUpdate, MessageRowUpdate,
    MessageSearchRow, SortOrder,
};
pub use repository::cron::{CRON_RUN_HISTORY_LIMIT, UpdateCronJobParams};
pub use repository::mcp_server::{CreateMcpServerParams, UpdateMcpServerParams};
pub use repository::oauth_token::UpsertOAuthTokenParams;
pub use repository::provider::{CreateProviderParams, UpdateProviderParams};
pub use repository::remote_agent::{CreateRemoteAgentParams, UpdateRemoteAgentParams};
pub use repository::{
    AdoptAgentExecutionStepOutputParams, AgentExecutionLeaseToken,
    AppendAgentExecutionStepsFromAttemptParams, AppendAgentExecutionStepsFromAttemptResult,
    AppendAgentExecutionStepsParams,
    AttemptConversationEffectParams, CreateAgentExecutionAttemptParams,
    CreateAgentExecutionParams, IAgentExecutionRepository,
    CreateAgentExecutionTemplateParams, IAgentExecutionTemplateRepository,
    NewAgentExecutionEvent, NewAgentExecutionParticipant, NewAgentExecutionStep,
    NewAgentExecutionStepDependency, ReconcileAgentExecutionPlanParams,
    NewAgentExecutionTemplateParticipant, UpdateAgentExecutionTemplateParams,
    LoopRepeatResetParams,
    RetryAgentExecutionStep, SettleAgentExecutionAttemptParams, UpdateAgentExecutionParams,
    CreateAcpSessionParams, CreateTerminalParams, IAcpSessionRepository,
    IAgentMetadataRepository, IAttachmentRepository, IChannelRepository,
    IClientPreferenceRepository, ICompanionTokenRepository, IConnectorCredentialRepository,
    IConversationRepository, ICronRepository, IIdmmInterventionRepository, IKnowledgeRepository,
    IMcpServerRepository, IModelProfileRepository, IOAuthTokenRepository, IProviderRepository,
    IRemoteAgentRepository, IRequirementRepository, ISettingsRepository, ISkillTagRepository,
    ITagSettingRepository, ITerminalRepository, IUserRepository, IWebhookRepository,
    ListRequirementsParams, PER_TARGET_CAP, PER_USER_ACTIVITY_CAP, PersistedSessionState,
    SaveRuntimeStateParams,
    SqliteAcpSessionRepository, SqliteAgentMetadataRepository, SqliteAttachmentRepository,
    SqliteAgentExecutionRepository,
    SqliteAgentExecutionTemplateRepository,
    SqliteChannelRepository, SqliteClientPreferenceRepository, SqliteCompanionTokenRepository,
    SqliteConnectorCredentialRepository, SqliteConversationRepository, SqliteCronRepository,
    SqliteIdmmInterventionRepository, SqliteKnowledgeRepository, SqliteMcpServerRepository,
    SqliteModelProfileRepository, SqliteOAuthTokenRepository, SqliteProviderRepository,
    SqliteRemoteAgentRepository, SqliteRequirementRepository, SqliteSettingsRepository,
    SqliteSkillTagRepository, SqliteTagSettingRepository, SqliteTerminalRepository,
    SqliteUserRepository, SqliteWebhookRepository, TTL_MS,
};
pub use repository::{
    IPresetRepository, IPresetStateRepository, IPresetTagRepository,
    SqlitePresetRepository, SqlitePresetStateRepository, SqlitePresetTagRepository,
};
// 创意工坊 (Creative Workshop) + 生成引擎 (creation) repository traits + sqlite impls + params.
pub use repository::{
    AssetSort, CreateCreationTaskParams, ICreationTaskRepository, IWorkshopRepository, ListAssetsParams,
    ListCreationTasksParams, SqliteCreationTaskRepository, SqliteWorkshopRepository,
    UpdateAssetParams, UpdateCreationTaskParams,
};

// Re-export sqlx (and its pool type) for downstream crates that run ad-hoc
// queries against the pool without declaring their own sqlx dependency
// (e.g. nomifun-app's bootstrap relocation path rewrite).
pub use sqlx;
pub use sqlx::SqlitePool;
