use nomifun_common::AppError;

#[derive(Debug, thiserror::Error)]
pub enum OfficeError {
    #[error("officecli not found")]
    OfficecliNotFound,

    #[error("officecli install failed: {0}")]
    InstallFailed(String),

    #[error("preview start failed: {0}")]
    StartFailed(String),

    #[error("port readiness timeout for {0}")]
    PortTimeout(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("snapshot error: {0}")]
    Snapshot(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("conversion error: {0}")]
    Conversion(String),

    #[error("external tool not found: {0}")]
    ToolNotFound(String),
}

impl From<OfficeError> for AppError {
    fn from(err: OfficeError) -> Self {
        match err {
            OfficeError::OfficecliNotFound => AppError::BadRequest("officecli not found".into()),
            OfficeError::InstallFailed(msg) => AppError::Internal(format!("officecli install failed: {msg}")),
            OfficeError::StartFailed(msg) => AppError::Internal(format!("preview start failed: {msg}")),
            OfficeError::PortTimeout(path) => AppError::Timeout(format!("port readiness timeout for {path}")),
            OfficeError::Io(e) => AppError::Internal(format!("IO error: {e}")),
            OfficeError::Snapshot(msg) => AppError::Internal(format!("snapshot error: {msg}")),
            OfficeError::Json(e) => AppError::Internal(format!("JSON error: {e}")),
            OfficeError::Conversion(msg) => AppError::Internal(format!("conversion error: {msg}")),
            OfficeError::ToolNotFound(tool) => AppError::BadRequest(format!("{tool} is not installed")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn officecli_not_found_maps_to_bad_request() {
        let err: AppError = OfficeError::OfficecliNotFound.into();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn install_failed_maps_to_internal() {
        let err: AppError = OfficeError::InstallFailed("npm error".into()).into();
        assert!(matches!(err, AppError::Internal(msg) if msg.contains("npm error")));
    }

    #[test]
    fn start_failed_maps_to_internal() {
        let err: AppError = OfficeError::StartFailed("spawn error".into()).into();
        assert!(matches!(err, AppError::Internal(msg) if msg.contains("spawn error")));
    }

    #[test]
    fn port_timeout_maps_to_timeout() {
        let err: AppError = OfficeError::PortTimeout("/a.docx".into()).into();
        assert!(matches!(err, AppError::Timeout(msg) if msg.contains("/a.docx")));
    }

    #[test]
    fn io_error_maps_to_internal() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: AppError = OfficeError::Io(io_err).into();
        assert!(matches!(err, AppError::Internal(msg) if msg.contains("file missing")));
    }

    #[test]
    fn conversion_error_maps_to_internal() {
        let err: AppError = OfficeError::Conversion("bad format".into()).into();
        assert!(matches!(err, AppError::Internal(msg) if msg.contains("bad format")));
    }

    #[test]
    fn tool_not_found_maps_to_bad_request() {
        let err: AppError = OfficeError::ToolNotFound("pandoc".into()).into();
        assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("pandoc")));
    }

    #[test]
    fn display_messages() {
        assert_eq!(OfficeError::OfficecliNotFound.to_string(), "officecli not found");
        assert_eq!(
            OfficeError::InstallFailed("npm error".into()).to_string(),
            "officecli install failed: npm error"
        );
        assert_eq!(
            OfficeError::PortTimeout("/a.docx".into()).to_string(),
            "port readiness timeout for /a.docx"
        );
        assert_eq!(
            OfficeError::Conversion("bad data".into()).to_string(),
            "conversion error: bad data"
        );
        assert_eq!(
            OfficeError::ToolNotFound("pandoc".into()).to_string(),
            "external tool not found: pandoc"
        );
    }
}
