use serde::Serialize;

/// Temporary storage admission failures. This classifies an observation; it
/// never authorizes replaying a command or an uncertain write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseUnavailableReason {
    #[error("connection acquisition timed out")]
    PoolTimedOut,
    #[error("database is busy")]
    Busy,
    #[error("database is locked")]
    Locked,
}
