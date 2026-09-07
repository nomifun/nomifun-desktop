use nomifun_agent_contracts::MiniAppM1ContractError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MiniAppPlatformError {
    #[error("MiniApp {0} was not found")]
    NotFound(String),
    #[error("MiniApp {0} already exists")]
    AlreadyExists(String),
    #[error("MiniApp repository compare-and-swap conflict")]
    CompareAndSwapConflict,
    #[error("MiniApp state is invalid: {0}")]
    InvalidState(String),
    #[error("MiniApp operation is not allowed while lifecycle is {0}")]
    LifecycleConflict(String),
    #[error("MiniApp release {0} is unknown")]
    UnknownRelease(String),
    #[error("MiniApp storage handle is unknown or belongs to another product")]
    UnknownStorageHandle,
    #[error("MiniApp credential slot {0} has no bound credential")]
    MissingCredential(String),
    #[error("MiniApp runtime port failed: {0}")]
    Runtime(String),
    #[error("MiniApp MessageChannel port is closed, stale, or foreign")]
    StaleBridgePort,
    #[error("MiniApp Bridge call {0} is already in flight")]
    DuplicateBridgeCall(String),
    #[error("MiniApp call was canceled")]
    Canceled,
    #[error("MiniApp Service is unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("MiniApp Service callback belongs to a stale Host generation")]
    StaleServiceGeneration,
    #[error("MiniApp Service Host crashed: {0}")]
    ServiceCrashed(String),
    #[error(
        "MiniApp Service Host capacity exhausted: max_active={max_active}, active={active_miniapps:?}"
    )]
    ServiceCapacityExhausted {
        max_active: usize,
        active_miniapps: Vec<String>,
    },
    #[error("MiniApp KV revision overflow")]
    KvRevisionOverflow,
    #[error("MiniApp managed storage compare-and-swap conflict")]
    StorageConflict,
    #[error("MiniApp managed database request is invalid: {0}")]
    InvalidDatabaseRequest(String),
    #[error("MiniApp repository failed: {0}")]
    Repository(String),
    #[error("MiniApp commit is authoritative but runtime reconciliation is required: {0}")]
    ReconcileRequired(String),
    #[error("MiniApp runtime recovery failed: {0}")]
    RuntimeRecovery(String),
    #[error("MiniApp operation {0} is unknown")]
    UnknownOperation(String),
    #[error("MiniApp operation compare-and-swap conflict")]
    OperationConflict,
    #[error("MiniApp operation {0} cannot be canceled")]
    OperationNotCancelable(String),
    #[error("MiniApp owner is busy: {0}")]
    OwnerBusy(String),
    #[error(transparent)]
    Contract(#[from] MiniAppM1ContractError),
}

pub type MiniAppPlatformResult<T> = Result<T, MiniAppPlatformError>;
