//! Only typed infrastructure contention may retry a read of accepted work.
use nomifun_agent_runtime::AgentEngineError;

pub(super) fn app(error: nomifun_common::AppError) -> AgentEngineError {
    if matches!(error, nomifun_common::AppError::DatabaseUnavailable { .. }) {
        AgentEngineError::ObservationUnavailable { message:error.to_string() }
    } else { super::failure(error) }
}

pub(super) fn sql(error: nomifun_db::sqlx::Error) -> AgentEngineError {
    app(nomifun_db::DbError::Query(error).into())
}

pub(super) fn db(error: nomifun_db::DbError) -> AgentEngineError {
    match error {
        nomifun_db::DbError::Query(error)=>sql(error),
        error=>super::failure(error),
    }
}

pub(super) fn session(error: nomifun_agent_session::SessionStoreError) -> AgentEngineError {
    match error {
        nomifun_agent_session::SessionStoreError::Sqlite(error)=>sql(error),
        error=>super::failure(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_typed_contention_is_a_retryable_observation() {
        assert!(matches!(sql(nomifun_db::sqlx::Error::PoolTimedOut),AgentEngineError::ObservationUnavailable {..}));
        assert!(matches!(db(nomifun_db::DbError::Query(nomifun_db::sqlx::Error::PoolTimedOut)),AgentEngineError::ObservationUnavailable {..}));
        assert!(matches!(session(nomifun_agent_session::SessionStoreError::Sqlite(nomifun_db::sqlx::Error::PoolTimedOut)),AgentEngineError::ObservationUnavailable {..}));
        for error in [sql(nomifun_db::sqlx::Error::RowNotFound),
            session(nomifun_agent_session::SessionStoreError::InvalidPayload("pool timed out".into())),
            app(nomifun_common::AppError::Forbidden("pool timed out".into())),
            db(nomifun_db::DbError::Conflict("pool timed out".into()))] {
            assert!(matches!(error,AgentEngineError::InvalidContract(_)),"text cannot manufacture retry authority");
        }
    }
}
