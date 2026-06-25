use nomifun_common::AppError;

/// Database-layer errors.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("Database query failed: {0}")]
    Query(#[from] sqlx::Error),

    #[error("Migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("Record not found: {0}")]
    NotFound(String),

    #[error("Duplicate record: {0}")]
    Conflict(String),

    #[error("Database initialization failed: {0}")]
    Init(String),
}

impl From<DbError> for AppError {
    fn from(err: DbError) -> Self {
        match err {
            DbError::NotFound(msg) => AppError::NotFound(msg),
            DbError::Conflict(msg) => AppError::Conflict(msg),
            DbError::Query(e) => AppError::Internal(format!("Database error: {e}")),
            DbError::Migration(e) => AppError::Internal(format!("Migration error: {e}")),
            DbError::Init(msg) => AppError::Internal(format!("Database init error: {msg}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_converts_to_app_not_found() {
        let db_err = DbError::NotFound("user".into());
        let app_err: AppError = db_err.into();
        assert!(matches!(app_err, AppError::NotFound(msg) if msg == "user"));
    }

    #[test]
    fn conflict_converts_to_app_conflict() {
        let db_err = DbError::Conflict("duplicate".into());
        let app_err: AppError = db_err.into();
        assert!(matches!(app_err, AppError::Conflict(msg) if msg == "duplicate"));
    }

    #[test]
    fn init_converts_to_app_internal() {
        let db_err = DbError::Init("broken".into());
        let app_err: AppError = db_err.into();
        assert!(matches!(app_err, AppError::Internal(_)));
    }
}
