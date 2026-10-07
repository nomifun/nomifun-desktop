use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nomifun_agent_contracts::CanonicalErrorCode;
use nomifun_api_types::ErrorResponse;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("{message}")]
    Canonical {
        code: CanonicalErrorCode,
        status: StatusCode,
        message: String,
        details: Option<Value>,
    },
    #[error("control-plane wire conversion failed: {0}")]
    Wire(String),
}

impl ControlPlaneError {
    pub fn canonical(
        code: impl Into<CanonicalErrorCode>,
        status: StatusCode,
        message: impl Into<String>,
    ) -> Self {
        Self::Canonical {
            code: code.into(),
            status,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(
        code: impl Into<CanonicalErrorCode>,
        status: StatusCode,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self::Canonical {
            code: code.into(),
            status,
            message: message.into(),
            details: Some(details),
        }
    }

    pub fn code(&self) -> CanonicalErrorCode {
        match self {
            Self::Canonical { code, .. } => code.clone(),
            Self::Wire(_) => CanonicalErrorCode::from("PRESET_REVISION_SAVE_FAILED"),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Canonical { status, .. } => *status,
            Self::Wire(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn details(&self) -> Option<Value> {
        match self {
            Self::Canonical { details, .. } => details.clone(),
            Self::Wire(_) => None,
        }
    }
}

impl IntoResponse for ControlPlaneError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ErrorResponse::new_with_details(
            self.to_string(),
            self.code().as_ref(),
            self.details(),
        );
        (status, Json(body)).into_response()
    }
}

impl From<serde_json::Error> for ControlPlaneError {
    fn from(error: serde_json::Error) -> Self {
        Self::Wire(error.to_string())
    }
}
