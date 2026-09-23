//! SQLite database layer: init, migrations, repository traits, and implementations.
pub mod backup_bundle;
mod agent_store_reset;
mod database;
mod error;
mod id_schema_contract;
pub mod models;
mod repository;
mod session_projection;
mod installation_role_bindings;
pub use installation_role_bindings::{load_installation_role_bindings, put_installation_role_binding};

pub use database::{
    Database, init_database, init_database_memory, init_database_memory_with_owner,
    open_database_for_backup, requires_unified_plugin_clean_start,
    validate_current_migration_lineage,
};
pub use agent_store_reset::{AgentDataResetReport, reset_agent_data};
pub use error::DbError;
pub use id_schema_contract::{validate_id_data_contract, validate_id_schema_contract};
pub use session_projection::MessageDayBucket;
pub use models::{
    AgentExecutionAttemptDetailRow, AgentExecutionAttemptRow, AgentExecutionDetailRows,
    AgentExecutionEventRow, AgentExecutionParticipantRow, AgentExecutionRow,
    AgentExecutionStepDependencyRow, AgentExecutionStepDetailRow, AgentExecutionStepRow,
    AgentExecutionTemplateDetailRows, AgentExecutionTemplateParticipantRow,
    AgentExecutionTemplateRow,
    AgentMetadataRow,
    CreateKnowledgeTagParams, CreationTaskRow, CreativeStudioAgentProposalReceiptRow,
    CreativeStudioProjectRow, CreativeStudioTemplateRow, CreativeStudioTemplateRunRow, CronJobRunRow,
    CronRunReservationRow,
    KNOWLEDGE_ENTRY_KIND_DIRECTORY, KNOWLEDGE_ENTRY_KIND_FILE,
    KNOWLEDGE_ENTRY_ORIGIN_GENERATED, KNOWLEDGE_ENTRY_ORIGIN_URL_SNAPSHOT,
    KNOWLEDGE_ENTRY_ORIGIN_USER, KnowledgeBaseRow, KnowledgeBindingRow, KnowledgeEntryRow,
    InvalidKnowledgeSourceValue, KnowledgeEntryProvenanceRelationship,
    KnowledgeEntryProvenanceRow,
    KnowledgeSourceItemRow, KnowledgeSourceItemSyncStatus, KnowledgeSourceKind,
    KnowledgeSourceMode, KnowledgeSourceRow, KnowledgeSourceState, KnowledgeTagRow,
    KnowledgeTreeEventStatus, KnowledgeTreeOperationRow,
    KnowledgeTreeOperationState, SkillTagRow, TagSettingRow, TerminalSessionRow,
    TerminalTurnAdmissionRow,
    UpdateAgentHandshakeParams,
    UpdateKnowledgeTagParams,
    UpsertAgentMetadataParams, UpsertSkillTagParams, WebhookRow,
    WorkshopAssetRow, ConversationExecutionLinkRow,
    NomiRemoteEventPage, NomiRemoteEventRow, NomiRemoteSessionRow, RemoteBindingRow,
};
pub use models::{
    CS_HANDOFF_STATUS_CANCELLED, CS_HANDOFF_STATUS_CLAIMED, CS_HANDOFF_STATUS_PENDING,
    CS_HANDOFF_STATUS_RESOLVED, CsAgentCapabilityReceiptRow, CsAgentRow, CsAuditEventRow,
    CsChannelBindingRow, CsDialogueRow, CsHandoffRow, CsMessageRow, CsNoteRow, NewCsAgentRow,
};
pub use models::{
    NewProviderModel, NewProviderModelCapability, ProviderConnectionRow,
    ProviderModelCapabilityRow, ProviderModelRow,
    UpsertProviderConnectionParams,
};
pub use repository::channel::UpdatePluginStatusParams;
pub use repository::customer_service::{
    CsDialogueKey, CsHandoffRequestResult, CsNoteWriteMutation, CsNoteWriteReceipt,
    ICustomerServiceRepository, UpdateCsAgentParams,
};
pub use repository::customer_service_search::{
    CsNoteSearchHit, NoteMatchChannel, backfill_note_search_text, fts_rebuild, note_search_text,
};
pub use repository::SqliteCustomerServiceRepository;
pub use repository::cron::{
    AdvanceCronOccurrenceParams, CRON_RUN_HISTORY_LIMIT, FinalizeCronRunOutcome,
    FinalizeCronRunParams, ReserveCronRunParams,
    UpdateCronJobParams,
};
pub use repository::mcp_server::{CreateMcpServerParams, UpdateMcpServerParams};
pub use repository::oauth_token::UpsertOAuthTokenParams;
pub use repository::provider::{CreateProviderParams, UpdateProviderParams};
pub use repository::ssh_host::{
    CreateSshHostParams, ISshHostRepository, UpdateSshHostParams,
};
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
    IClientPreferenceRepository, IInstanceTokenRepository, KNOWLEDGE_RETRIEVAL_KEY,
    ICronRepository,
    IKnowledgeRepository,
    IKnowledgeEntryRepository, IKnowledgeSourceRepository, IKnowledgeTreeOperationRepository,
    KnowledgeEntryMutation,
    KnowledgeProjectionReplacement,
    RelocateKnowledgeEntryProjectionParams, UpsertKnowledgeEntryParams,
    IMcpServerRepository, IOAuthTokenRepository,
    CoordinatedProviderModelDelete, IProviderConnectionRepository,
    IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
    IRequirementRepository, ISettingsRepository, ISkillTagRepository,
    ITagSettingRepository, ITerminalRepository, IUserRepository, IWebhookRepository,
    ListRequirementsParams, RequirementClaim, RequirementClaimResolution,
    SqliteAgentMetadataRepository, SqliteAttachmentRepository,
    SqliteAgentExecutionRepository,
    SqliteAgentExecutionTemplateRepository,
    SqliteChannelRepository, SqliteClientPreferenceRepository, SqliteInstanceTokenRepository,
    SqliteCronRepository,
    SqliteKnowledgeRepository,
    SqliteKnowledgeTreeOperationRepository, SqliteMcpServerRepository,
    SqliteOAuthTokenRepository,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    ProviderModelCleanupPlan, ProviderModelProjectCleanup, ProviderModelTemplateCleanup,
    SqliteProviderModelRepository, SqliteProviderRepository,
    SqliteRequirementRepository, SqliteSettingsRepository,
    SqliteSkillTagRepository, SqliteTagSettingRepository, SqliteTerminalRepository,
    SqliteUserRepository, SqliteWebhookRepository, TerminalTurnAdmissionClaim,
    TerminalTurnAdmissionKey, TerminalTurnAdmissionScope, TerminalTurnEffectsStart,
    TerminalTurnOutcome, TerminalTurnSettlement,
};
pub use repository::{
    AppendNomiRemoteEventParams, AppendNomiRemoteEventResult, CreateRemoteBindingParams,
    GetOrCreateRemoteSessionParams, IRemoteBindingRepository, NomiRemoteStateTransitionResult,
    RemoteOpenResult, SqliteRemoteBindingRepository, TransitionNomiRemoteSessionParams,
    UpdateRemoteBindingParams,
};
pub use repository::{
    BindManagedKnowledgeEntryParams, CreateKnowledgeSourceItemParams,
    EnsureKnowledgeSourceParams, EnsuredKnowledgeSource, RecordKnowledgeEntryCopyParams,
    RecordKnowledgeSourceSyncFailureParams, RecordKnowledgeSourceSyncSuccessParams,
    StageKnowledgeSourcePublicationParams, UpdateKnowledgeSourceItemParams,
    UpdateKnowledgeSourceParams,
};
pub use repository::{
    CommitKnowledgeTreeOperationParams, KnowledgeTreeOperationPageCursor,
    MAX_KNOWLEDGE_TREE_OPERATION_PAGE_SIZE, PrepareKnowledgeTreeOperationParams,
    PreparedKnowledgeTreeOperation,
};
// 创意工坊 (Creative Workshop) + 生成引擎 (creation) repository traits + sqlite impls + params.
pub use repository::{
    ApplyCreativeAgentProposalParams, AssetSort, CreateCreativeTaskParams,
    CreativeAgentProposalCommit, CreativeTaskOwnerRef,
    ICreationTaskRepository, IWorkshopRepository, IdempotentCreationTask, ListAssetsParams,
    PromptLibraryAssetIdentity,
    SqliteCreationTaskRepository, SqliteWorkshopRepository, UpdateAssetParams,
    UpdateCreationTaskParams,
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
