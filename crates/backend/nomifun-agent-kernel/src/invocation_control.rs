//! Process-local invocation lifetime, independent of frozen authorization.

use tokio_util::sync::CancellationToken;

/// Cooperative control carried from the owning execution task to a handler.
/// Cancellation requests cleanup; it never grants authority or proves that an
/// external effect did not happen. The owner must keep polling dispatched work
/// until its result and cleanup have been observed.
#[derive(Clone, Debug, Default)]
pub struct InvocationControl {
    cancellation: CancellationToken,
}

impl InvocationControl {
    pub fn new(cancellation: CancellationToken) -> Self {
        Self { cancellation }
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }
}
