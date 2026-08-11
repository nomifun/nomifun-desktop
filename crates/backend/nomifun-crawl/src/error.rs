use nomifun_common::AppError;

#[derive(Debug, thiserror::Error)]
pub enum CrawlError {
    #[error("crawl job not found: {0}")]
    JobNotFound(String),
    #[error("crawl task not found: {0}")]
    TaskNotFound(String),
    /// The submitted claim token no longer matches the row. The result is
    /// discarded rather than overwriting whoever legitimately owns the task now.
    #[error("stale claim for task {0}; result discarded")]
    StaleClaim(String),
    #[error("url rejected: {0}")]
    UrlRejected(String),
    #[error("job {0} reached its URL budget")]
    BudgetExhausted(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    App(#[from] AppError),
}

impl From<CrawlError> for AppError {
    fn from(err: CrawlError) -> Self {
        match err {
            CrawlError::App(inner) => inner,
            CrawlError::JobNotFound(_) | CrawlError::TaskNotFound(_) => {
                AppError::NotFound(err.to_string())
            }
            CrawlError::UrlRejected(_) | CrawlError::BudgetExhausted(_) => {
                AppError::BadRequest(err.to_string())
            }
            CrawlError::StaleClaim(_) => AppError::Conflict(err.to_string()),
            CrawlError::Db(inner) => AppError::Internal(format!("crawl database error: {inner}")),
        }
    }
}
