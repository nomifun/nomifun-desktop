use nomifun_common::AppError;

/// Extension system domain errors.
#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("Manifest validation failed: {0}")]
    ManifestValidation(String),

    #[error("Extension name '{name}' uses reserved prefix '{prefix}'")]
    ReservedNamePrefix { name: String, prefix: String },

    #[error("Invalid version '{version}': {reason}")]
    InvalidVersion { version: String, reason: String },

    #[error("Undefined environment variable: {0}")]
    UndefinedEnvVariable(String),

    #[error("File reference not found: {0}")]
    FileReferenceNotFound(String),

    #[error("Path traversal detected: {0}")]
    PathTraversal(String),

    #[error("Engine incompatible: extension '{name}' requires nomifun {required}, got {actual}")]
    EngineIncompatible {
        name: String,
        required: String,
        actual: String,
    },

    #[error("API version incompatible: extension '{name}' requires API {required}, supported {supported}")]
    ApiVersionIncompatible {
        name: String,
        required: String,
        supported: String,
    },

    #[error("WebUI route '{route}' must be under '/{extension_name}/' namespace")]
    InvalidWebuiRouteNamespace { extension_name: String, route: String },

    #[error("WebUI route '{route}' uses reserved prefix '{prefix}'")]
    ReservedWebuiRoute { route: String, prefix: String },

    #[error("Theme CSS file not found: {0}")]
    ThemeCssNotFound(String),

    #[error("Contribution resolution failed for '{extension_name}': {reason}")]
    ResolutionFailed { extension_name: String, reason: String },

    #[error("Lifecycle hook '{hook}' timed out after {timeout_secs}s for extension '{extension_name}'")]
    HookTimeout {
        extension_name: String,
        hook: String,
        timeout_secs: u64,
    },

    #[error("Lifecycle hook '{hook}' failed for extension '{extension_name}': {reason}")]
    HookFailed {
        extension_name: String,
        hook: String,
        reason: String,
    },

    #[error("Lifecycle hook script not found: {0}")]
    HookNotFound(String),

    #[error("Extension not found: {0}")]
    NotFound(String),

    #[error("State persistence failed: {0}")]
    StatePersistence(String),

    #[error("Cannot delete built-in skill: {0}")]
    BuiltinSkillDeletion(String),

    #[error("Skill not found: {0}")]
    SkillNotFound(String),

    #[error("Invalid skill path: {0}")]
    InvalidSkillPath(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    JsonParse(#[from] serde_json::Error),
}

impl From<ExtensionError> for AppError {
    fn from(err: ExtensionError) -> Self {
        match err {
            ExtensionError::ManifestValidation(msg) => AppError::BadRequest(msg),
            ExtensionError::ReservedNamePrefix { .. } => AppError::BadRequest(err.to_string()),
            ExtensionError::InvalidVersion { .. } => AppError::BadRequest(err.to_string()),
            ExtensionError::UndefinedEnvVariable(var) => {
                AppError::BadRequest(format!("Undefined environment variable: {var}"))
            }
            ExtensionError::FileReferenceNotFound(path) => {
                AppError::NotFound(format!("File reference not found: {path}"))
            }
            ExtensionError::PathTraversal(path) => AppError::BadRequest(format!("Path traversal detected: {path}")),
            ExtensionError::EngineIncompatible { .. } => AppError::BadRequest(err.to_string()),
            ExtensionError::ApiVersionIncompatible { .. } => AppError::BadRequest(err.to_string()),
            ExtensionError::InvalidWebuiRouteNamespace { .. } => AppError::BadRequest(err.to_string()),
            ExtensionError::ReservedWebuiRoute { .. } => AppError::BadRequest(err.to_string()),
            ExtensionError::ThemeCssNotFound(path) => AppError::NotFound(format!("Theme CSS not found: {path}")),
            ExtensionError::HookTimeout { .. } => AppError::Internal(err.to_string()),
            ExtensionError::HookFailed { .. } => AppError::Internal(err.to_string()),
            ExtensionError::HookNotFound(path) => AppError::NotFound(format!("Hook script not found: {path}")),
            ExtensionError::ResolutionFailed { .. } => AppError::Internal(err.to_string()),
            ExtensionError::NotFound(name) => AppError::NotFound(format!("Extension not found: {name}")),
            ExtensionError::StatePersistence(msg) => AppError::Internal(msg),
            ExtensionError::BuiltinSkillDeletion(name) => {
                AppError::BadRequest(format!("Cannot delete built-in skill: {name}"))
            }
            ExtensionError::SkillNotFound(name) => AppError::NotFound(format!("Skill not found: {name}")),
            ExtensionError::InvalidSkillPath(path) => AppError::BadRequest(format!("Invalid skill path: {path}")),
            ExtensionError::Io(e) => AppError::Internal(e.to_string()),
            ExtensionError::JsonParse(e) => AppError::BadRequest(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_validation_error_display() {
        let err = ExtensionError::ManifestValidation("name is required".into());
        assert_eq!(err.to_string(), "Manifest validation failed: name is required");
    }

    #[test]
    fn test_reserved_name_prefix_error_display() {
        let err = ExtensionError::ReservedNamePrefix {
            name: "nomi-test".into(),
            prefix: "nomi-".into(),
        };
        assert_eq!(
            err.to_string(),
            "Extension name 'nomi-test' uses reserved prefix 'nomi-'"
        );
    }

    #[test]
    fn test_invalid_version_error_display() {
        let err = ExtensionError::InvalidVersion {
            version: "not-semver".into(),
            reason: "unexpected character".into(),
        };
        assert_eq!(err.to_string(), "Invalid version 'not-semver': unexpected character");
    }

    #[test]
    fn test_undefined_env_variable_error_display() {
        let err = ExtensionError::UndefinedEnvVariable("MY_SECRET".into());
        assert_eq!(err.to_string(), "Undefined environment variable: MY_SECRET");
    }

    #[test]
    fn test_file_reference_not_found_error_display() {
        let err = ExtensionError::FileReferenceNotFound("prompts/system.md".into());
        assert_eq!(err.to_string(), "File reference not found: prompts/system.md");
    }

    #[test]
    fn test_path_traversal_error_display() {
        let err = ExtensionError::PathTraversal("../../etc/passwd".into());
        assert_eq!(err.to_string(), "Path traversal detected: ../../etc/passwd");
    }

    #[test]
    fn test_into_app_error_path_traversal() {
        let err = ExtensionError::PathTraversal("../secret".into());
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::BadRequest(_)));
    }

    #[test]
    fn test_into_app_error_bad_request() {
        let err = ExtensionError::ManifestValidation("test".into());
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::BadRequest(_)));
    }

    #[test]
    fn test_into_app_error_not_found() {
        let err = ExtensionError::FileReferenceNotFound("missing.md".into());
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::NotFound(_)));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err = ExtensionError::from(io_err);
        assert!(matches!(err, ExtensionError::Io(_)));
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::Internal(_)));
    }
}
