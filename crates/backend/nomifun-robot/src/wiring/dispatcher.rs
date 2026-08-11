//! The real [`CompanionTurnDispatcher`].
//!
//! Thread creation mirrors the channel domain's own dispatch path
//! (`nomifun-channel/src/message_service.rs`): build a `SendMessageRequest`, send
//! it with an idempotency key, then attach to the runtime's stream. The device
//! inherits the installation owner — robots do not log in.
//!
//! The concrete backend lives in `nomifun-app`, where `ConversationService`, the
//! runtime registry, the companion registry and the installation owner id are all
//! in scope at once. Keeping only the narrow trait here means `nomifun-robot`
//! never depends on the host crate, so the dependency direction stays one-way.
//!
//! Cancellation uses the **public** `cancel` (`CancelOrigin` is crate-private),
//! and never `runtime_registry.terminate` — that would kill the runtime rather
//! than stop one turn.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::services::{CompanionTurnDispatcher, TurnEvent};
use crate::vad::VadTuning;

/// The narrow view of the conversation stack this crate needs.
#[async_trait::async_trait]
pub trait RobotConversationBackend: Send + Sync {
    /// Find or create the `(robot, companion)` thread, refreshing its
    /// `session_mcp_servers` entry so the robot MCP proxy URL is never stale
    /// across restarts (the port is per-boot).
    async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String>;
    /// Send one user turn and stream reduced events.
    async fn dispatch(
        &self,
        conversation_id: &str,
        text: &str,
        use_fallback_model: bool,
    ) -> anyhow::Result<mpsc::Receiver<TurnEvent>>;
    /// Public cancel.
    async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()>;
    /// `voice.vad` of the companion profile.
    async fn vad_tuning(&self, companion_id: &str) -> VadTuning;
    /// `voice.vad.engine` of the companion profile.
    async fn vad_engine(&self, companion_id: &str) -> String;
    /// Whether `fallback_model` is set.
    async fn has_fallback_model(&self, companion_id: &str) -> bool;
}

/// What the host must supply for real conversation access.
pub struct RobotDispatcher {
    inner: Arc<dyn RobotConversationBackend>,
}

impl RobotDispatcher {
    pub fn new(inner: Arc<dyn RobotConversationBackend>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl CompanionTurnDispatcher for RobotDispatcher {
    async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String> {
        self.inner.ensure_thread(robot_id, companion_id).await
    }

    async fn dispatch(
        &self,
        conversation_id: &str,
        text: &str,
        use_fallback_model: bool,
    ) -> anyhow::Result<mpsc::Receiver<TurnEvent>> {
        self.inner
            .dispatch(conversation_id, text, use_fallback_model)
            .await
    }

    async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()> {
        self.inner.cancel(conversation_id).await
    }

    async fn vad_tuning(&self, companion_id: &str) -> VadTuning {
        self.inner.vad_tuning(companion_id).await
    }

    async fn vad_engine(&self, companion_id: &str) -> String {
        self.inner.vad_engine(companion_id).await
    }

    async fn has_fallback_model(&self, companion_id: &str) -> bool {
        self.inner.has_fallback_model(companion_id).await
    }
}
