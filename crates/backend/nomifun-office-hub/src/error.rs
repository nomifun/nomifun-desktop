use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct HubError {
    pub code: String,
    pub message: String,
    /// True only with evidence that no mutation request was dispatched.
    pub not_executed: bool,
    #[serde(default)] pub missing_scopes: Vec<String>,
    pub console_url: Option<String>,
    pub related_operation_id: Option<String>,
    pub related_flow_id: Option<String>,
}
impl HubError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self { code: "OFFICE_INVALID_INPUT".into(), message: message.into(), not_executed: true,
            missing_scopes: Vec::new(), console_url: None, related_operation_id: None, related_flow_id: None }
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self { code: "OFFICE_UNAVAILABLE".into(), message: message.into(), not_executed: true,
            missing_scopes: Vec::new(), console_url: None, related_operation_id: None, related_flow_id: None }
    }
    pub fn unknown(message: impl Into<String>) -> Self {
        Self { code: "OFFICE_RESULT_UNKNOWN".into(), message: message.into(), not_executed: false,
            missing_scopes: Vec::new(), console_url: None, related_operation_id: None, related_flow_id: None }
    }
    pub fn storage(error: impl std::fmt::Display) -> Self {
        Self { code: "OFFICE_STORAGE".into(), message: error.to_string(), not_executed: false,
            missing_scopes: Vec::new(), console_url: None, related_operation_id: None, related_flow_id: None }
    }
    pub fn conflict(message: impl Into<String>, operation_id: Option<String>) -> Self {
        Self { code: "OFFICE_CONFLICT".into(), message: message.into(), not_executed: true,
            missing_scopes: Vec::new(), console_url: None, related_operation_id: operation_id, related_flow_id: None }
    }
}
impl axum::response::IntoResponse for HubError {
    fn into_response(self) -> axum::response::Response {
        use axum::{Json, http::StatusCode};
        let status = match self.code.as_str() {
            "OFFICE_INVALID_INPUT" => StatusCode::BAD_REQUEST,
            "OFFICE_NOT_FOUND" => StatusCode::NOT_FOUND,
            "OFFICE_CONFLICT" | "OFFICE_RESULT_UNKNOWN" => StatusCode::CONFLICT,
            "OFFICE_PERMISSION" => StatusCode::FORBIDDEN,
            "OFFICE_STORAGE" => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_GATEWAY,
        };
        let details = serde_json::json!({
            "not_executed": self.not_executed,
            "missing_scopes": self.missing_scopes,
            "console_url": self.console_url,
            "related_operation_id": self.related_operation_id,
            "related_flow_id": self.related_flow_id,
        });
        (status, Json(nomifun_api_types::ErrorResponse::new_with_details(self.message, self.code, Some(details)))).into_response()
    }
}
impl From<HubError> for AppError {
    fn from(error: HubError) -> Self {
        match error.code.as_str() {
            "OFFICE_INVALID_INPUT" => AppError::BadRequest(error.message),
            "OFFICE_NOT_FOUND" => AppError::NotFound(error.message),
            "OFFICE_CONFLICT" | "OFFICE_RESULT_UNKNOWN" => AppError::Conflict(error.message),
            "OFFICE_PERMISSION" => AppError::Forbidden(error.message),
            "OFFICE_STORAGE" => AppError::Internal(error.message),
            _ => AppError::BadGateway(error.message),
        }
    }
}
