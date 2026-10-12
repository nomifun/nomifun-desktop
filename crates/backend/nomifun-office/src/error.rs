use nomifun_common::AppError;

#[derive(Debug, thiserror::Error)]
pub enum OfficeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot error: {0}")]
    Snapshot(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<OfficeError> for AppError {
    fn from(error: OfficeError) -> Self {
        AppError::Internal(error.to_string())
    }
}
