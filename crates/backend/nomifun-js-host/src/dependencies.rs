//! Callbacks belong to one pending Tool/Context request, not to a Mount or SDK.
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CanonicalErrorCode, PluginDependencyCall, PluginHostResponseBody, PluginHostWireError,
};
use tokio::sync::watch;

/// The Host owns and drops each call future on parent closure or generation
/// failure. Implementations must not detach effects outside that lifetime.
#[async_trait]
pub trait ExtensionHostDependencyCaller: Send + Sync {
    async fn invoke(&self, call: PluginDependencyCall) -> PluginHostResponseBody;
}

pub(crate) struct PendingDependencies {
    pub caller: Arc<dyn ExtensionHostDependencyCaller>,
    open: watch::Sender<bool>,
}

impl PendingDependencies {
    pub fn new(caller: Arc<dyn ExtensionHostDependencyCaller>) -> Self {
        let (open, _) = watch::channel(true);
        Self { caller, open }
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.open.subscribe()
    }

    pub fn close(&self) {
        self.open.send_replace(false);
    }
}

impl Drop for PendingDependencies {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) fn parent_closed() -> PluginHostResponseBody {
    PluginHostResponseBody::Failure(PluginHostWireError {
        code: CanonicalErrorCode::from("DEPENDENCY_PARENT_CLOSED"),
        message: "dependency call requires its live, exactly bound parent".into(),
        retryable: false,
    })
}
