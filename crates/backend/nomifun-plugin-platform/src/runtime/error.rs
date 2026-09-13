use nomifun_agent_contracts::MiniAppM1ContractError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginRuntimePlatformError {
    #[error("Plugin {0} was not found")]
    NotFound(String),
    #[error("Plugin {0} already exists")]
    AlreadyExists(String),
    #[error("Plugin repository compare-and-swap conflict")]
    CompareAndSwapConflict,
    #[error("Plugin state is invalid: {0}")]
    InvalidState(String),
    #[error("Plugin operation is not allowed while lifecycle is {0}")]
    LifecycleConflict(String),
    #[error("Plugin release {0} is unknown")]
    UnknownRelease(String),
    #[error("Plugin storage handle is unknown or belongs to another product")]
    UnknownStorageHandle,
    #[error("Plugin credential slot {0} has no bound credential")]
    MissingCredential(String),
    #[error("Plugin runtime port failed: {0}")]
    Runtime(String),
    #[error("Plugin MessageChannel port is closed, stale, or foreign")]
    StaleBridgePort,
    #[error("Plugin Bridge call {0} is already in flight")]
    DuplicateBridgeCall(String),
    #[error("Plugin call was canceled")]
    Canceled,
    #[error("Plugin Service is unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Plugin Service callback belongs to a stale Host generation")]
    StaleServiceGeneration,
    #[error("Plugin Service Host crashed: {0}")]
    ServiceCrashed(String),
    #[error(
        "Plugin Service Host capacity exhausted: max_active={max_active}, active={active_miniapps:?}"
    )]
    ServiceCapacityExhausted {
        max_active: usize,
        active_miniapps: Vec<String>,
    },
    #[error("Plugin KV revision overflow")]
    KvRevisionOverflow,
    #[error("Plugin managed storage compare-and-swap conflict")]
    StorageConflict,
    #[error("Plugin managed database request is invalid: {0}")]
    InvalidDatabaseRequest(String),
    #[error("Plugin managed database failed: {0}")]
    Database(String),
    #[error("Plugin repository failed: {0}")]
    Repository(String),
    #[error("Plugin commit is authoritative but runtime reconciliation is required: {0}")]
    ReconcileRequired(String),
    #[error("Plugin runtime recovery failed: {0}")]
    RuntimeRecovery(String),
    #[error("Plugin operation {0} is unknown")]
    UnknownOperation(String),
    #[error("Plugin operation compare-and-swap conflict")]
    OperationConflict,
    #[error("Plugin operation {0} cannot be canceled")]
    OperationNotCancelable(String),
    #[error("Plugin owner is busy: {0}")]
    OwnerBusy(String),
    #[error(transparent)]
    Contract(#[from] MiniAppM1ContractError),
}

pub type PluginRuntimePlatformResult<T> = Result<T, PluginRuntimePlatformError>;
