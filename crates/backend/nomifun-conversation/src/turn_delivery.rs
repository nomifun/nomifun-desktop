//! Canonical Agent Turn delivery projections shared by product adapters.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotentMessageDelivery {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicTurnDeliveryState {
    Missing,
    Accepted { message_id: String },
    Completed(IdempotentMessageDelivery),
}

pub trait BackgroundTaskRegistrar: Send + Sync {
    fn spawn(
        &self,
        task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>,
    ) -> bool;
}
