//! SQLite database layer: init, migrations, repository traits, and implementations.
pub mod backup_bundle;
mod database;
mod error;
mod id_schema_contract;
pub mod models;
mod repository;

pub use database::{
    Database, MigrationLineageStatus, init_database, init_database_memory,
    init_database_memory_with_owner, inspect_supported_migration_lineage,
    open_database_for_backup, validate_current_migration_lineage,
};
pub use error::DbError;
pub use id_schema_contract::{validate_id_data_contract, validate_id_schema_contract};
pub use models::{
    AgentExecutionAttemptDetailRow, AgentExecutionAttemptRow, AgentExecutionDetailRows,
    AgentExecutionEventRow, AgentExecutionParticipantRow, AgentExecutionRow,
    AgentExecutionStepDependencyRow, AgentExecutionStepDetailRow, AgentExecutionStepRow,
    AgentExecutionTemplateDetailRows, AgentExecutionTemplateParticipantRow,
    AgentExecutionTemplateRow,
    AgentMetadataRow,
    ConversationArtifactRow, IdmmActionReservationRow,
    CreateKnowledgeTagParams, CreationTaskRow, CronJobRunRow, CronRunReservationRow,
    KnowledgeBaseRow, KnowledgeBindingRow,
    KnowledgeTagRow, SkillTagRow, TagSettingRow, TerminalSessionRow, TerminalTurnAdmissionRow,
    UpdateAgentHandshakeParams,
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
pub use models::{
    CsAgentRow, CsAuditEventRow, CsChannelBindingRow, CsDialogueRow, CsMessageRow, CsNoteRow,
    NewCsAgentRow,
};
pub use models::{
    NewProviderModel, NewProviderModelCapability, ProviderConnectionRow,
    ProviderModelCapabilityRow, ProviderModelRow,
    UpsertProviderConnectionParams,
};
pub use repository::channel::UpdatePluginStatusParams;
pub use repository::customer_service::{
    CsDialogueKey, ICustomerServiceRepository, UpdateCsAgentParams,
};
pub use repository::customer_service_search::{
    CsNoteSearchHit, NoteMatchChannel, backfill_note_search_text, fts_rebuild, note_search_text,
};
pub use repository::SqliteCustomerServiceRepository;
pub use repository::conversation::{
    ConversationDeliveryReceiptClaim, ConversationFilters, ConversationMessageProjection,
    ConversationTurnAdmissionState,
    ConversationRowUpdate, MessageDayBucket, MessageRowUpdate, MessageSearchRow, SortOrder,
    MAX_UNSETTLED_TURN_ADMISSION_PAGE_SIZE,
    RequirementConversationTurnAuthority,
    TurnArtifactMessageCommit, TurnLifecycleTransition, TurnReceiptCompletion,
    UnsettledConversationTurnAdmission,
};
pub use repository::cron::{
    AdvanceCronOccurrenceParams, CRON_RUN_HISTORY_LIMIT, FinalizeCronRunOutcome,
    FinalizeCronRunParams, ReserveCronRunParams,
    UpdateCronJobParams,
};
pub use repository::mcp_server::{CreateMcpServerParams, UpdateMcpServerParams};
pub use repository::miniapp::{CreateMiniAppParams, IMiniAppRepository, UpdateMiniAppParams};
pub use repository::oauth_token::UpsertOAuthTokenParams;
pub use repository::provider::{CreateProviderParams, UpdateProviderParams};
pub use repository::ssh_host::{
    CreateSshHostParams, ISshHostRepository, UpdateSshHostParams,
};
pub use repository::SqliteMiniAppRepository;
pub use models::{MiniAppDocumentRow, MiniAppRow};
pub use repository::SqliteSshHostRepository;
pub use models::SshHostRow;
pub use repository::{
    AdoptAgentExecutionStepOutputParams, AgentExecutionAttemptRecoveryDisposition,
    AgentExecutionAttemptRecoveryResult, AgentExecutionLeaseToken, AgentExecutionTurnAuthority,
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
    CreateTerminalParams,
    IAgentMetadataRepository, IAttachmentRepository, ChannelInboundClaim,
    IChannelRepository, PENDING_PROMPT_EXPIRY_MS, PENDING_PROMPT_QUEUE_LIMIT,
    PairingApprovalOutcome, PendingPromptEnqueue, SettleChannelInboundReceiptParams,
    IClientPreferenceRepository, ICompanionTokenRepository, KNOWLEDGE_RETRIEVAL_KEY,
    IConversationRepository, ICronRepository, IIdmmInterventionRepository,
    IdmmActionReservationKey, IdmmActionReserveResult, IdmmActionSettleResult,
    IdmmActionSettlement, IdmmActionTurnIdentity, IKnowledgeRepository,
    IMcpServerRepository, IOAuthTokenRepository,
    IProviderConnectionRepository, IProviderModelCapabilityRepository, IProviderModelRepository,
    IProviderRepository,
    IRequirementRepository, ISettingsRepository, ISkillTagRepository,
    ITagSettingRepository, ITerminalRepository, IUserRepository, IWebhookRepository,
    ListRequirementsParams, RequirementClaim, RequirementClaimResolution,
    MAX_IDMM_ACTION_FAILURE_REASON_CHARS, PER_TARGET_CAP, PER_USER_ACTIVITY_CAP,
    ReserveIdmmActionParams,
    SqliteAgentMetadataRepository, SqliteAttachmentRepository,
    SqliteAgentExecutionRepository,
    SqliteAgentExecutionTemplateRepository,
    SqliteChannelRepository, SqliteClientPreferenceRepository, SqliteCompanionTokenRepository,
    SqliteConversationRepository, SqliteCronRepository,
    SqliteIdmmInterventionRepository, SqliteKnowledgeRepository, SqliteMcpServerRepository,
    SqliteOAuthTokenRepository,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository,
    SqliteRequirementRepository, SqliteSettingsRepository,
    SqliteSkillTagRepository, SqliteTagSettingRepository, SqliteTerminalRepository,
    SqliteUserRepository, SqliteWebhookRepository, TerminalTurnAdmissionClaim,
    TerminalTurnAdmissionKey, TerminalTurnAdmissionScope, TerminalTurnEffectsStart,
    TerminalTurnOutcome, TerminalTurnSettlement, TTL_MS,
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

/// Resolve the canonical owner user ID for this dataset.
///
/// The identity is stored in the database rather than reconstructed from a
/// global constant. A missing, duplicated, non-canonical, or dangling owner is
/// a database invariant violation and fails closed.
pub async fn installation_owner_id(pool: &SqlitePool) -> Result<String, DbError> {
    let identities: Vec<(String, String)> =
        sqlx::query_as("SELECT singleton_key, owner_user_id FROM installation_identity")
            .fetch_all(pool)
            .await
            .map_err(DbError::Query)?;
    let [(key, owner_user_id)] = identities.as_slice() else {
        return Err(DbError::Init(format!(
            "installation identity must contain exactly one owner, found {}",
            identities.len()
        )));
    };
    if key != "installation" {
        return Err(DbError::Init(format!(
            "installation identity contains invalid singleton key {key:?}"
        )));
    }
    nomifun_common::UserId::parse(owner_user_id.clone()).map_err(|error| {
        DbError::Init(format!(
            "installation owner ID is not canonical: {owner_user_id}: {error}"
        ))
    })?;
    let owner_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE user_id = ?")
        .bind(owner_user_id)
        .fetch_one(pool)
        .await
        .map_err(DbError::Query)?;
    if owner_exists != 1 {
        return Err(DbError::Init(format!(
            "installation identity references missing owner user {owner_user_id}"
        )));
    }
    Ok(owner_user_id.clone())
}
