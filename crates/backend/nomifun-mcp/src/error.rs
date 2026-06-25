use nomifun_common::AppError;

/// MCP crate-level errors.
///
/// Uses `thiserror` (library crate convention).
/// Converts to `AppError` for HTTP response mapping.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("MCP server not found: {0}")]
    NotFound(String),

    #[error("MCP server name conflict: {0}")]
    Conflict(String),

    #[error("Invalid MCP server edit: {0}")]
    InvalidEdit(String),

    #[error("Invalid transport configuration: {0}")]
    InvalidTransport(String),

    #[error("Agent CLI not installed: {0}")]
    AgentNotInstalled(String),

    #[error("Agent operation failed: {0}")]
    AgentOperationFailed(String),

    #[error("Connection test failed: {0}")]
    ConnectionFailed(String),

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("{0}")]
    Database(#[from] nomifun_db::DbError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<McpError> for AppError {
    fn from(err: McpError) -> Self {
        match err {
            McpError::NotFound(msg) => AppError::NotFound(msg),
            McpError::Conflict(msg) => AppError::Conflict(msg),
            McpError::InvalidEdit(msg) => AppError::BadRequest(msg),
            McpError::InvalidTransport(msg) => AppError::BadRequest(msg),
            McpError::AgentNotInstalled(msg) => AppError::BadRequest(msg),
            McpError::AgentOperationFailed(msg) => AppError::Internal(msg),
            McpError::ConnectionFailed(msg) => AppError::BadGateway(msg),
            McpError::OAuth(msg) => AppError::Internal(format!("OAuth error: {msg}")),
            McpError::Database(db_err) => AppError::from(db_err),
            McpError::Json(e) => AppError::Internal(format!("JSON error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_maps_to_app_not_found() {
        let err: AppError = McpError::NotFound("mcp_123".into()).into();
        assert!(matches!(err, AppError::NotFound(msg) if msg == "mcp_123"));
    }

    #[test]
    fn conflict_maps_to_app_conflict() {
        let err: AppError = McpError::Conflict("test-server".into()).into();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[test]
    fn invalid_transport_maps_to_bad_request() {
        let err: AppError = McpError::InvalidTransport("missing command".into()).into();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn invalid_edit_maps_to_bad_request() {
        let err: AppError = McpError::InvalidEdit("rename forbidden".into()).into();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn agent_not_installed_maps_to_bad_request() {
        let err: AppError = McpError::AgentNotInstalled("claude".into()).into();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn agent_operation_failed_maps_to_internal() {
        let err: AppError = McpError::AgentOperationFailed("exit code 1".into()).into();
        assert!(matches!(err, AppError::Internal(_)));
    }

    #[test]
    fn connection_failed_maps_to_bad_gateway() {
        let err: AppError = McpError::ConnectionFailed("timeout".into()).into();
        assert!(matches!(err, AppError::BadGateway(_)));
    }

    #[test]
    fn oauth_maps_to_internal() {
        let err: AppError = McpError::OAuth("discovery failed".into()).into();
        assert!(matches!(err, AppError::Internal(_)));
    }

    #[test]
    fn json_error_maps_to_internal() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid").unwrap_err();
        let err: AppError = McpError::Json(json_err).into();
        assert!(matches!(err, AppError::Internal(_)));
    }

    #[test]
    fn display_messages() {
        assert_eq!(
            McpError::NotFound("mcp_1".into()).to_string(),
            "MCP server not found: mcp_1"
        );
        assert_eq!(
            McpError::InvalidTransport("bad".into()).to_string(),
            "Invalid transport configuration: bad"
        );
        assert_eq!(
            McpError::InvalidEdit("rename forbidden".into()).to_string(),
            "Invalid MCP server edit: rename forbidden"
        );
    }
}
