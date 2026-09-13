use thiserror::Error;

pub const ERR_CONFLICT: &str = "PLUGIN_CONFLICT";
pub const ERR_NOT_FOUND: &str = "PLUGIN_NOT_FOUND";
pub const ERR_FORBIDDEN: &str = "PLUGIN_FORBIDDEN";
pub const ERR_INVALID_INPUT: &str = "PLUGIN_INVALID_INPUT";
pub const ERR_STALE: &str = "PLUGIN_STALE";
pub const ERR_OPERATION: &str = "PLUGIN_OPERATION_FAILED";
pub const ERR_OPERATION_CANCELED: &str = "PLUGIN_OPERATION_CANCELED";
pub const ERR_INTEGRATION: &str = "PLUGIN_INTEGRATION_REQUIRED";
pub const ERR_SECRET_LEAK: &str = "PLUGIN_SECRET_LEAK";
pub const ERR_ARTIFACT: &str = "PLUGIN_ARTIFACT_INVALID";
pub const ERR_RUNTIME: &str = "PLUGIN_RUNTIME_UNAVAILABLE";
pub const ERR_RECONCILE_REQUIRED: &str = "PLUGIN_RECONCILE_REQUIRED";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PluginServiceError {
    #[error("{code}: {message}")]
    Coded { code: &'static str, message: String },
    #[error("{ERR_INTEGRATION}: {0}")]
    Integration(String),
    #[error("{ERR_RECONCILE_REQUIRED}: {0}")]
    ReconcileRequired(String),
}

impl PluginServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Coded { code, .. } => code,
            Self::Integration(_) => ERR_INTEGRATION,
            Self::ReconcileRequired(_) => ERR_RECONCILE_REQUIRED,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Coded {
            code: ERR_INVALID_INPUT,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Coded {
            code: ERR_CONFLICT,
            message: message.into(),
        }
    }

    pub fn stale(message: impl Into<String>) -> Self {
        Self::Coded {
            code: ERR_STALE,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::Coded {
            code: ERR_NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn operation_canceled(message: impl Into<String>) -> Self {
        Self::Coded {
            code: ERR_OPERATION_CANCELED,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Coded {
            code: ERR_FORBIDDEN,
            message: message.into(),
        }
    }

    pub fn integration(message: impl Into<String>) -> Self {
        Self::Integration(message.into())
    }

    pub fn reconcile_required(message: impl Into<String>) -> Self {
        Self::ReconcileRequired(message.into())
    }
}

impl From<nomifun_db::DbError> for PluginServiceError {
    fn from(value: nomifun_db::DbError) -> Self {
        match value {
            nomifun_db::DbError::NotFound(message) => Self::not_found(message),
            nomifun_db::DbError::Conflict(message) => Self::conflict(message),
            other => Self::integration(other.to_string()),
        }
    }
}

impl From<crate::PluginArtifactStoreError> for PluginServiceError {
    fn from(value: crate::PluginArtifactStoreError) -> Self {
        Self::Coded {
            code: ERR_ARTIFACT,
            message: value.to_string(),
        }
    }
}

impl From<nomifun_js_host::JavaScriptHostError> for PluginServiceError {
    fn from(value: nomifun_js_host::JavaScriptHostError) -> Self {
        Self::Coded {
            code: ERR_RUNTIME,
            message: value.to_string(),
        }
    }
}

impl From<crate::OwnerMutationError> for PluginServiceError {
    fn from(value: crate::OwnerMutationError) -> Self {
        Self::integration(value.to_string())
    }
}
