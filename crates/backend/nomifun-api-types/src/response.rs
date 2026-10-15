use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

/// Standard API success response envelope.
///
/// Endpoints that return data wrap it in this structure. For custom
/// response shapes (login, auth status, etc.), use dedicated types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl<T> ApiResponse<T> {
    /// Create a success response with data.
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: None,
        }
    }

    /// Create a success response with data and a message.
    pub fn with_message(data: T, message: impl Into<String>) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: Some(message.into()),
        }
    }
}

impl ApiResponse<()> {
    /// Create a success response with only a message (no data payload).
    pub fn message(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            data: None,
            message: Some(msg.into()),
        }
    }

    /// Create a minimal success response (no data, no message).
    pub fn success() -> Self {
        Self {
            success: true,
            data: None,
            message: None,
        }
    }
}

/// Standard API error response.
///
/// Matches the JSON format produced by `AppError::IntoResponse`:
/// `{ "success": false, "error": "...", "code": "...", "details": ... }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>, code: impl Into<String>) -> Self {
        Self::new_with_details(error, code, None)
    }

    pub fn new_with_details(
        error: impl Into<String>,
        code: impl Into<String>,
        details: impl Into<Option<serde_json::Value>>,
    ) -> Self {
        Self {
            success: false,
            error: error.into(),
            code: code.into(),
            details: details.into(),
        }
    }
}

impl From<AppError> for ErrorResponse {
    fn from(err: AppError) -> Self {
        Self {
            success: false,
            error: err.to_string(),
            code: err.error_code().to_owned(),
            details: err.error_details(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_response_ok() {
        let resp = ApiResponse::ok(42);
        assert!(resp.success);
        assert_eq!(resp.data, Some(42));
        assert!(resp.message.is_none());
    }

    #[test]
    fn test_api_response_with_message() {
        let resp = ApiResponse::with_message("data", "Created");
        assert!(resp.success);
        assert_eq!(resp.data, Some("data"));
        assert_eq!(resp.message.as_deref(), Some("Created"));
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"success": true, "data": "data", "message": "Created"})
        );
    }

    #[test]
    fn test_api_response_message_only() {
        let resp = ApiResponse::message("Done");
        assert!(resp.success);
        assert!(resp.data.is_none());
        assert_eq!(resp.message.as_deref(), Some("Done"));
    }

    #[test]
    fn test_api_response_success_minimal() {
        let resp = ApiResponse::success();
        assert!(resp.success);
        assert!(resp.data.is_none());
        assert!(resp.message.is_none());
    }

    #[test]
    fn test_api_response_serialization_with_data() {
        #[derive(serde::Serialize)]
        struct UserData {
            id: String,
            name: String,
        }
        let resp = ApiResponse::ok(UserData {
            id: "u1".into(),
            name: "Alice".into(),
        });
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["data"], serde_json::json!({"id": "u1", "name": "Alice"}));
        assert!(json.get("message").is_none());
    }

    #[test]
    fn test_api_response_serialization_message_only() {
        let resp = ApiResponse::message("Logged out successfully");
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["success"], true);
        assert!(json.get("data").is_none());
        assert_eq!(json["message"], "Logged out successfully");
    }

    #[test]
    fn test_api_response_serialization_minimal() {
        let resp = ApiResponse::success();
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["success"], true);
        assert!(json.get("data").is_none());
        assert!(json.get("message").is_none());
    }

    #[test]
    fn test_error_response_new() {
        let resp = ErrorResponse::new("Not found", "NOT_FOUND");
        assert!(!resp.success);
        assert_eq!(resp.error, "Not found");
        assert_eq!(resp.code, "NOT_FOUND");
        assert!(resp.details.is_none());
    }

    #[test]
    fn test_error_response_serialization() {
        let resp = ErrorResponse::new("Bad request: missing field", "BAD_REQUEST");
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["success"], false);
        assert_eq!(json["error"], "Bad request: missing field");
        assert_eq!(json["code"], "BAD_REQUEST");
        assert!(json.get("details").is_none());
    }

    #[test]
    fn test_error_response_from_app_errors() {
        for (err, code, message) in [
            (AppError::NotFound("user 42".into()), "NOT_FOUND", "Not found: user 42"),
            (
                AppError::BadRequest("missing field: username".into()),
                "BAD_REQUEST", "Bad request: missing field: username",
            ),
            (AppError::Unauthorized("invalid token".into()), "UNAUTHORIZED", "Unauthorized: invalid token"),
            (AppError::Forbidden("access denied".into()), "FORBIDDEN", "Forbidden: access denied"),
            (AppError::Conflict("username taken".into()), "CONFLICT", "Conflict: username taken"),
            (AppError::RateLimited, "RATE_LIMITED", "Rate limited"),
            (
                AppError::Internal("db connection lost".into()),
                "INTERNAL_ERROR", "Internal error: db connection lost",
            ),
            (AppError::BadGateway("upstream timeout".into()), "BAD_GATEWAY", "Bad gateway: upstream timeout"),
            (AppError::Timeout("request timed out".into()), "TIMEOUT", "Request timeout: request timed out"),
        ] {
            let resp = ErrorResponse::from(err);
            assert!(!resp.success);
            assert_eq!(resp.error, message);
            assert_eq!(resp.code, code);
            assert!(resp.details.is_none());
        }
    }

    #[test]
    fn test_error_response_new_with_details() {
        let resp = ErrorResponse::new_with_details(
            "Bad request: invalid workspace",
            "WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED",
            serde_json::json!({ "workspace_path": "/tmp/Archive " }),
        );
        assert_eq!(
            resp.details,
            Some(serde_json::json!({ "workspace_path": "/tmp/Archive " }))
        );
    }

    #[test]
    fn test_error_response_from_workspace_error_includes_details() {
        let resp = ErrorResponse::from(AppError::WorkspacePathEdgeWhitespace(
            "/tmp/Archive ".into(),
        ));
        assert_eq!(resp.code, "WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED");
        assert_eq!(
            resp.details
                .as_ref()
                .and_then(|details| details.get("workspace_path")),
            Some(&serde_json::json!("/tmp/Archive "))
        );
        assert_eq!(
            resp.details
                .as_ref()
                .and_then(|details| details.get("operation")),
            Some(&serde_json::json!("create"))
        );
        assert_eq!(resp.details.as_ref().unwrap()["offending_segments"], serde_json::json!(["Archive "]));
    }

    #[test]
    fn test_error_response_from_runtime_workspace_error_includes_details() {
        let resp = ErrorResponse::from(AppError::WorkspacePathEdgeWhitespaceRuntimeUnsupported(
            "/tmp/Archive ".into(),
        ));
        assert_eq!(
            resp.code,
            "WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED"
        );
        assert_eq!(
            resp.details
                .as_ref()
                .and_then(|details| details.get("workspace_path")),
            Some(&serde_json::json!("/tmp/Archive "))
        );
        assert_eq!(
            resp.details
                .as_ref()
                .and_then(|details| details.get("operation")),
            Some(&serde_json::json!("runtime"))
        );
        assert_eq!(resp.details.as_ref().unwrap()["offending_segments"], serde_json::json!(["Archive "]));
    }

    #[test]
    fn test_api_response_deserialization() {
        let json = r#"{"success":true,"data":"test","message":"ok"}"#;
        let resp: ApiResponse<String> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert_eq!(resp.data.as_deref(), Some("test"));
        assert_eq!(resp.message.as_deref(), Some("ok"));
    }

    #[test]
    fn test_error_response_deserialization() {
        let json = r#"{"success":false,"error":"Not found","code":"NOT_FOUND"}"#;
        let resp: ErrorResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.error, "Not found");
        assert_eq!(resp.code, "NOT_FOUND");
        assert!(resp.details.is_none());
    }

    #[test]
    fn test_error_response_with_details() {
        let resp = ErrorResponse::new_with_details(
            "Command not found: npx",
            "MCP_COMMAND_NOT_FOUND",
            Some(serde_json::json!({ "command": "npx" })),
        );
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["details"]["command"], "npx");
    }
}
