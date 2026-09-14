//! The real [`CompanionTurnDispatcher`].
//!
//! Every endpoint resolves the Companion's existing Conversation. Utterances
//! carry device and connection identity through the host's keyed turn boundary.
//!
//! The concrete backend lives in `nomifun-app`, where `ConversationService`, the
//! runtime registry, the companion registry and the installation owner id are all
//! in scope at once. Keeping only the narrow trait here means `nomifun-robot`
//! never depends on the host crate, so the dependency direction stays one-way.
//!
//! Cancellation names one exact utterance. It cannot stop whichever unrelated
//! desktop or device turn happens to be active on the shared Conversation.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::services::{CompanionTurnDispatcher, RobotTurnRequest, TurnEvent};
use crate::vad::VadTuning;

/// The narrow view of the conversation stack this crate needs.
#[async_trait::async_trait]
pub trait RobotConversationBackend: Send + Sync {
    /// Obtain the Companion's single Conversation.
    async fn ensure_companion_session(&self, companion_id: &str) -> anyhow::Result<String>;
    /// Send one user turn and stream reduced events.
    async fn dispatch(
        &self,
        request: RobotTurnRequest,
    ) -> anyhow::Result<mpsc::Receiver<TurnEvent>>;
    async fn cancel(&self, request: &RobotTurnRequest) -> anyhow::Result<()>;
    /// `voice.vad` of the companion profile.
    async fn vad_tuning(&self, companion_id: &str) -> VadTuning;
    /// `voice.vad.engine` of the companion profile.
    async fn vad_engine(&self, companion_id: &str) -> String;
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
    async fn ensure_companion_session(&self, companion_id: &str) -> anyhow::Result<String> {
        self.inner.ensure_companion_session(companion_id).await
    }

    async fn dispatch(
        &self,
        request: RobotTurnRequest,
    ) -> anyhow::Result<mpsc::Receiver<TurnEvent>> {
        self.inner
            .dispatch(request)
            .await
    }

    async fn cancel(&self, request: &RobotTurnRequest) -> anyhow::Result<()> {
        self.inner.cancel(request).await
    }

    async fn vad_tuning(&self, companion_id: &str) -> VadTuning {
        self.inner.vad_tuning(companion_id).await
    }

    async fn vad_engine(&self, companion_id: &str) -> String {
        self.inner.vad_engine(companion_id).await
    }

}
