use async_trait::async_trait;
use nomifun_db::models::RequirementRow;

/// Notified after a requirement reaches a terminal state (done|failed).
/// Implementations MUST be cheap / non-blocking (the caller spawns this).
#[async_trait]
pub trait CompletionNotifier: Send + Sync {
    async fn notify_completion(&self, requirement: &RequirementRow);
}
