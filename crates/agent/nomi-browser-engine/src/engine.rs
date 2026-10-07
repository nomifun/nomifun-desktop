//! Errors shared by explicitly owned browser launch/input primitives.
//! Native Workspace and System Browser keep their own public runtime contracts.
use crate::transport::TransportError;
#[derive(thiserror::Error, Debug, Clone)]
pub enum BrowserError {
    #[error("unsupported capability {capability}: {hint}")]
    Unsupported { capability: String, hint: String },
    #[error("browser session lost (recoverable={recoverable})")]
    SessionLost { recoverable: bool },
    #[error("blocked: {reason}")]
    Blocked { reason: String },
    #[error("navigation failed: {kind}")]
    NavFailed { kind: String },
    #[error("target crashed")]
    TargetCrashed,
    #[error("target closed")]
    TargetClosed,
    #[error("{0}")]
    Other(String),
}

pub fn map_transport_err(e: TransportError) -> BrowserError {
    match e {
        TransportError::Timeout => BrowserError::NavFailed {
            kind: "cdp command timed out".into(),
        },
        TransportError::Closed => BrowserError::SessionLost { recoverable: false },
        TransportError::SessionClosed => BrowserError::TargetClosed,
        TransportError::SessionCrashed => BrowserError::TargetCrashed,
        TransportError::Cdp { code, message } => {
            BrowserError::Other(format!("cdp error {code}: {message}"))
        }
        TransportError::Protocol(msg) => BrowserError::Other(format!("cdp protocol error: {msg}")),
    }
}
