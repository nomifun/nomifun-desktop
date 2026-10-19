//! Safe, typed failures for the MCP authorization boundary.

use axum::{Json, http::StatusCode, response::{IntoResponse, Response}};
use nomifun_api_types::{ErrorResponse, McpAuthDiagnostic, McpAuthErrorCode, McpAuthStage, McpAuthUserAction};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct McpAuthFailure {
    pub diagnostic: McpAuthDiagnostic,
}

impl McpAuthFailure {
    /// Messages and details supplied here must be authored, safe diagnostics,
    /// never upstream response bodies, callback URLs, or credential material.
    pub fn new(code: McpAuthErrorCode, stage: McpAuthStage, message: impl Into<String>) -> Self {
        use McpAuthErrorCode::*;
        let retryable = matches!(&code, UpstreamUnavailable | CallbackTimeout);
        let user_action = match &code {
            ClientRegistrationRequired | ClientRegistrationRejected | ClientAuthenticationUnsupported
                | InvalidConfiguration | RedirectUnavailable => McpAuthUserAction::ConfigureClient,
            ReauthorizationRequired | UserDenied => McpAuthUserAction::Reauthorize,
            InsufficientScope => McpAuthUserAction::GrantScope,
            UpstreamUnavailable | CallbackTimeout | ConnectionChanged => McpAuthUserAction::Retry,
            _ => McpAuthUserAction::None,
        };
        Self { diagnostic: McpAuthDiagnostic {
            code, stage, retryable, user_action,
            trace_id: nomifun_common::generate_id(), message: message.into(), details: None,
        } }
    }

    pub fn with_details(mut self, details: Value) -> Self { self.diagnostic.details = Some(details); self }
    pub fn retryable(mut self, retryable: bool) -> Self { self.diagnostic.retryable = retryable; self }
    pub fn action(mut self, action: McpAuthUserAction) -> Self { self.diagnostic.user_action = action; self }
    pub fn at_stage(mut self, stage: McpAuthStage) -> Self { self.diagnostic.stage = stage; self }

    pub fn status(&self) -> StatusCode {
        use McpAuthErrorCode::*;
        match &self.diagnostic.code {
            InvalidConfiguration | CallbackInvalid | StateMismatch => StatusCode::BAD_REQUEST,
            ConnectionChanged | Cancelled => StatusCode::CONFLICT,
            CallbackTimeout => StatusCode::GATEWAY_TIMEOUT,
            UpstreamUnavailable | ResourceMetadataUnavailable => StatusCode::BAD_GATEWAY,
            Internal | SecretStorageFailed => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }
}

impl std::fmt::Display for McpAuthFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.diagnostic.message) }
}
impl std::error::Error for McpAuthFailure {}

impl IntoResponse for McpAuthFailure {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = serde_json::to_value(&self.diagnostic.code).ok()
            .and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_else(|| "internal".into());
        let details = serde_json::to_value(&self.diagnostic).ok();
        (status, Json(ErrorResponse::new_with_details(self.diagnostic.message, code, details))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_failures_preserve_their_stage_and_status() {
        let failure = McpAuthFailure::new(McpAuthErrorCode::IssuerMismatch, McpAuthStage::Discovery,
            "Authorization server identity did not match");
        assert_eq!(failure.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!failure.diagnostic.retryable);
        assert_eq!(failure.to_string(), "Authorization server identity did not match");
        assert!(!failure.diagnostic.trace_id.is_empty());
        assert_eq!(McpAuthFailure::new(McpAuthErrorCode::CallbackTimeout, McpAuthStage::Callback, "Timed out").status(),
            StatusCode::GATEWAY_TIMEOUT);
    }
}
