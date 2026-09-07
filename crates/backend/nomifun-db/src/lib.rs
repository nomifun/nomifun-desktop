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
    CreateKnowledgeTagParams, CreationTaskRow, CreativeStudioAgentProposalReceiptRow,
    CreativeStudioProjectRow, CreativeStudioTemplateRow, CreativeStudioTemplateRunRow, CronJobRunRow,
    CronRunReservationRow,
    KNOWLEDGE_ENTRY_KIND_DIRECTORY, KNOWLEDGE_ENTRY_KIND_FILE,
    KNOWLEDGE_ENTRY_ORIGIN_GENERATED, KNOWLEDGE_ENTRY_ORIGIN_URL_SNAPSHOT,
    KNOWLEDGE_ENTRY_ORIGIN_USER, KnowledgeBaseRow, KnowledgeBindingRow, KnowledgeEntryRow,
    InvalidKnowledgeSourceValue, KnowledgeEntryProvenanceRelationship,
    JavaScriptRuntimeSelectionRecord, JavaScriptRuntimeSelectionRow,
    MiniAppM1Kind, MiniAppM1ProjectSourceState, MiniAppM1ReleaseOrigin,
    MiniAppM1ReleaseSourceKind, MiniAppM1LibrarySnapshot, MiniAppM1Snapshot,
    MiniAppCredentialBindingRow, MiniAppLibraryStateRow, MiniAppProductRow,
    MiniAppProjectRow, MiniAppReleaseArtifactRow, MiniAppReleaseRow,
    KnowledgeEntryProvenanceRow,
    KnowledgeSourceItemRow, KnowledgeSourceItemSyncStatus, KnowledgeSourceKind,
    KnowledgeSourceMode, KnowledgeSourceRow, KnowledgeSourceState, KnowledgeTagRow,
    KnowledgeTreeEventStatus, KnowledgeTreeOperationRow,
    KnowledgeTreeOperationState, SkillTagRow, TagSettingRow, TerminalSessionRow,
    TerminalTurnAdmissionRow,
    PluginArtifactRow, PluginCandidateTestReceiptRow, PluginCredentialBindingInput,
    PluginCredentialBindingRow, PluginCredentialBindingSnapshot, PluginKvRow,
    PluginMountRevisionRow, PluginMountRow, PluginMountRuntimeState, PluginProjectRow,
    PluginReadyCandidateRow,
    PluginCandidateOrigin, ProductOperationKind, ProductOperationRow, ProductOperationState,
    UpdateAgentHandshakeParams,
    UpdateKnowledgeTagParams,
    UpsertAgentMetadataParams, UpsertSkillTagParams, WebhookRow,
    WorkshopAssetRow, ConversationExecutionLinkRow,
    NomiRemoteEventPage, NomiRemoteEventRow, NomiRemoteSessionRow, RemoteBindingRow,
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
    CreativeStudioConversationTurnAuthority,
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
    IConversationRepository, ICronRepository, IIdmmInterventionRepository,
    IJavaScriptRuntimeSelectionRepository,
    CommitMiniAppM1PointerStateParams, CreateMiniAppM1Params,
    IMiniAppM1Repository, RecordMiniAppM1ReadyReleaseParams,
    SqliteMiniAppM1Repository, UpdateMiniAppM1ProjectSourceParams,
    IdmmActionReservationKey, IdmmActionReserveResult, IdmmActionSettleResult,
    IdmmActionSettlement, IdmmActionTurnIdentity, IKnowledgeRepository,
    IKnowledgeEntryRepository, IKnowledgeSourceRepository, IKnowledgeTreeOperationRepository,
    KnowledgeEntryMutation,
    KnowledgeProjectionReplacement,
    RelocateKnowledgeEntryProjectionParams, UpsertKnowledgeEntryParams,
    IMcpServerRepository, IOAuthTokenRepository,
    ApplyPluginCandidateParams, CreatePluginArtifactParams, CreatePluginProjectParams,
    DeletePluginKvParams, DeletePluginProjectParams, FinishProductOperationParams,
    GetPluginKvParams,
    IPluginN1Repository, ListPluginCredentialBindingsParams, PutPluginKvParams,
    RecordPluginCandidateTestReceiptParams, RecordPluginReadyCandidateParams,
    ReplacePluginCredentialBindingsParams, RestorePluginMountParams, StartProductOperationParams,
    UninstallPluginMountParams, UpdatePluginMountConfigParams, UpdatePluginProjectSourceParams,
    MAX_PRODUCT_OPERATION_LOG_LINES,
    MAX_PRODUCT_OPERATION_LOG_LINE_CHARS,
    CoordinatedProviderModelDelete, IProviderConnectionRepository,
    IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
    IRequirementRepository, ISettingsRepository, ISkillTagRepository,
    ITagSettingRepository, ITerminalRepository, IUserRepository, IWebhookRepository,
    ListRequirementsParams, RequirementClaim, RequirementClaimResolution,
    MAX_IDMM_ACTION_FAILURE_REASON_CHARS, PER_TARGET_CAP, PER_USER_ACTIVITY_CAP,
    ReserveIdmmActionParams, ResolveCreativeStudioAgentSessionParams,
    ResolvedCreativeStudioAgentSession,
    SqliteAgentMetadataRepository, SqliteAttachmentRepository,
    SqliteAgentExecutionRepository,
    SqliteAgentExecutionTemplateRepository,
    SaveJavaScriptRuntimeSelectionParams,
    SqliteChannelRepository, SqliteClientPreferenceRepository, SqliteInstanceTokenRepository,
    SqliteConversationRepository, SqliteCronRepository,
    SqliteIdmmInterventionRepository, SqliteKnowledgeRepository,
    SqliteJavaScriptRuntimeSelectionRepository,
    SqliteKnowledgeTreeOperationRepository, SqliteMcpServerRepository,
    SqliteOAuthTokenRepository,
    SqlitePluginN1Repository,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    ProviderModelCleanupPlan, ProviderModelProjectCleanup, ProviderModelTemplateCleanup,
    SqliteProviderModelRepository, SqliteProviderRepository,
    SqliteRequirementRepository, SqliteSettingsRepository,
    SqliteSkillTagRepository, SqliteTagSettingRepository, SqliteTerminalRepository,
    SqliteUserRepository, SqliteWebhookRepository, TerminalTurnAdmissionClaim,
    TerminalTurnAdmissionKey, TerminalTurnAdmissionScope, TerminalTurnEffectsStart,
    TerminalTurnOutcome, TerminalTurnSettlement, TTL_MS,
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
    CreationTaskPageCursorRef, CreativeAgentProposalCommit, CreativeTaskOwnerRef,
    ICreationTaskRepository, IWorkshopRepository, IdempotentCreationTask, ListAssetsParams,
    ListStandaloneWorkbenchTasksParams, PromptLibraryAssetIdentity,
    RetireStandaloneWorkbenchTasksParams,
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
