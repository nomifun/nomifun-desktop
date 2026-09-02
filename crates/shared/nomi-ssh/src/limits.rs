//! Bounds shared by the SSH transport and its backend owner.
//!
//! These are admission limits, not truncation hints. A request that would
//! exceed one of them is rejected before it reaches the remote host.

use std::fmt;
use std::time::Duration;

/// Maximum size of credential material accepted by the transport.
pub const MAX_SSH_CREDENTIAL_BYTES: usize = 16 * 1024 * 1024;
/// Practical upper bound for a DNS/IP host coordinate.
pub const MAX_SSH_HOST_BYTES: usize = 255;
/// Practical upper bound for an SSH login name.
pub const MAX_SSH_USERNAME_BYTES: usize = 255;
/// Practical upper bound for an agent socket path.
pub const MAX_SSH_AGENT_SOCKET_BYTES: usize = 4 * 1024;
/// Maximum length of a remote POSIX path accepted by SFTP operations.
pub const MAX_SSH_PATH_BYTES: usize = 4 * 1024;
/// Maximum length of one shell submission before the sentinel is appended.
pub const MAX_SSH_COMMAND_BYTES: usize = 64 * 1024;
/// Maximum bytes accepted for one remote file write.
pub const MAX_SSH_WRITE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum bytes retained from one remote file or shell command result.
pub const MAX_SSH_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// Bound for operations whose API has no caller-supplied timeout.
pub const SSH_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound for the complete SSH connect/authentication handshake.
pub const SSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitError {
    Empty { field: &'static str },
    Nul { field: &'static str },
    Control { field: &'static str },
    TooLong {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { field } => write!(f, "{field} must not be empty"),
            Self::Nul { field } => write!(f, "{field} contains NUL"),
            Self::Control { field } => write!(f, "{field} contains control characters"),
            Self::TooLong {
                field,
                limit,
                actual,
            } => write!(f, "{field} is {actual} bytes; maximum is {limit}"),
        }
    }
}

impl std::error::Error for LimitError {}

pub fn validate_credential(field: &'static str, value: &str) -> Result<(), LimitError> {
    validate_text(field, value, MAX_SSH_CREDENTIAL_BYTES, false)
}

pub fn validate_endpoint_component(
    field: &'static str,
    value: &str,
    limit: usize,
) -> Result<(), LimitError> {
    validate_text(field, value, limit, true)
}

pub fn validate_path(value: &str) -> Result<(), LimitError> {
    validate_text("SSH path", value, MAX_SSH_PATH_BYTES, true)
}

pub fn validate_command(value: &str) -> Result<(), LimitError> {
    validate_text("SSH command", value, MAX_SSH_COMMAND_BYTES, false)
}

pub fn validate_write_payload(value: &[u8]) -> Result<(), LimitError> {
    validate_size("SSH write payload", value.len(), MAX_SSH_WRITE_BYTES)
}

pub fn validate_output_size(actual: usize) -> Result<(), LimitError> {
    validate_size("SSH output", actual, MAX_SSH_OUTPUT_BYTES)
}

fn validate_text(
    field: &'static str,
    value: &str,
    limit: usize,
    reject_controls: bool,
) -> Result<(), LimitError> {
    if value.is_empty() {
        return Err(LimitError::Empty { field });
    }
    validate_size(field, value.len(), limit)?;
    if value.as_bytes().contains(&0) {
        return Err(LimitError::Nul { field });
    }
    if reject_controls && value.chars().any(char::is_control) {
        return Err(LimitError::Control { field });
    }
    Ok(())
}

pub fn validate_size(
    field: &'static str,
    actual: usize,
    limit: usize,
) -> Result<(), LimitError> {
    if actual > limit {
        return Err(LimitError::TooLong {
            field,
            limit,
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_and_credential_limits_reject_unsafe_or_oversized_inputs() {
        assert!(validate_endpoint_component("SSH host", "example.test", 255).is_ok());
        assert!(matches!(
            validate_endpoint_component("SSH host", "", 255),
            Err(LimitError::Empty { .. })
        ));
        assert!(matches!(
            validate_endpoint_component("SSH host", "line\nbreak", 255),
            Err(LimitError::Control { .. })
        ));
        assert!(matches!(
            validate_credential("SSH key", &"x".repeat(MAX_SSH_CREDENTIAL_BYTES + 1)),
            Err(LimitError::TooLong { .. })
        ));
        assert!(matches!(
            validate_credential("SSH key", "echo\0bad"),
            Err(LimitError::Nul { .. })
        ));
        assert!(validate_path("/tmp/file.txt").is_ok());
        assert!(matches!(
            validate_path(&"x".repeat(MAX_SSH_PATH_BYTES + 1)),
            Err(LimitError::TooLong { .. })
        ));
        assert!(matches!(
            validate_command(&"x".repeat(MAX_SSH_COMMAND_BYTES + 1)),
            Err(LimitError::TooLong { .. })
        ));
        assert!(validate_write_payload(&[]).is_ok());
        assert!(matches!(
            validate_write_payload(&vec![0; MAX_SSH_WRITE_BYTES + 1]),
            Err(LimitError::TooLong { .. })
        ));
        assert!(matches!(
            validate_output_size(MAX_SSH_OUTPUT_BYTES + 1),
            Err(LimitError::TooLong { .. })
        ));
    }
}
