//! The real [`CompanionTurnDispatcher`].
//!
//! Every endpoint resolves the Companion's existing Conversation. Utterances
//! carry device and connection identity through the host's keyed turn boundary.
//!
//! The concrete backend lives in `nomifun-app`, where the canonical AgentSession owner, the
//! runtime registry, the companion registry and the installation owner id are all
//! in scope at once. Keeping only the narrow trait here means `nomifun-robot`
//! never depends on the host crate, so the dependency direction stays one-way.
//!
//! Cancellation names one exact utterance. It cannot stop whichever unrelated
//! desktop or device turn happens to be active on the shared Conversation.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::services::{CompanionTurnDispatcher, RobotOutputLease, RobotQuestionPlaybackAuthority, RobotTurnRequest, TurnEvent};
use crate::vad::VadTuning;

/// The narrow view of the conversation stack this crate needs.
#[async_trait::async_trait]
pub trait RobotConversationBackend: Send + Sync {
    async fn capture_input_context(&self,companion:&str,robot:&str,session:&str,connection:&str,question:Option<String>)->anyhow::Result<Option<crate::services::RobotInputContext>>;
    async fn input_context_matches(&self,request:&RobotTurnRequest)->anyhow::Result<bool>;
    async fn acquire_output_context(&self,request:&RobotTurnRequest,_expected_proactive_revision:Option<u64>,question:Option<&RobotQuestionPlaybackAuthority>)->anyhow::Result<Option<RobotOutputLease>> {
        if question.is_some() {return Ok(None);}
        Ok(self.input_context_matches(request).await?.then(||RobotOutputLease::new(())))
    }
    async fn question_presented(&self,request:&RobotTurnRequest,source:&nomifun_voice_contracts::VoiceSpeechSourceRef,presentation_id:&str)->anyhow::Result<()>;
    async fn supplement(&self,request:&RobotTurnRequest,text:&str,correction:bool,input_id:&str)->anyhow::Result<()>;
    async fn cancel_background_work(&self,request:&RobotTurnRequest)->anyhow::Result<()>;
    async fn supplement_background_work(&self,request:&RobotTurnRequest,text:&str,correction:bool,input_id:&str)->anyhow::Result<()>;
    /// Obtain this exact device's independent companion ingress Session.
    async fn ensure_companion_session(&self, companion_id: &str, robot_id: &str) -> anyhow::Result<String>;
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
    async fn capture_input_context(&self,companion:&str,robot:&str,session:&str,connection:&str,question:Option<String>)->anyhow::Result<Option<crate::services::RobotInputContext>> {
        self.inner.capture_input_context(companion,robot,session,connection,question).await
    }
    async fn input_context_matches(&self,request:&RobotTurnRequest)->anyhow::Result<bool> {self.inner.input_context_matches(request).await}
    async fn acquire_output_context(&self,request:&RobotTurnRequest,expected_proactive_revision:Option<u64>,question:Option<&RobotQuestionPlaybackAuthority>)->anyhow::Result<Option<RobotOutputLease>> {
        self.inner.acquire_output_context(request,expected_proactive_revision,question).await
    }
    async fn question_presented(&self,request:&RobotTurnRequest,source:&nomifun_voice_contracts::VoiceSpeechSourceRef,presentation_id:&str)->anyhow::Result<()> {
        self.inner.question_presented(request,source,presentation_id).await
    }
    async fn supplement(&self,request:&RobotTurnRequest,text:&str,correction:bool,input_id:&str)->anyhow::Result<()> {
        self.inner.supplement(request,text,correction,input_id).await
    }
    async fn cancel_background_work(&self,request:&RobotTurnRequest)->anyhow::Result<()> {self.inner.cancel_background_work(request).await}
    async fn supplement_background_work(&self,request:&RobotTurnRequest,text:&str,correction:bool,input_id:&str)->anyhow::Result<()> {self.inner.supplement_background_work(request,text,correction,input_id).await}
    async fn ensure_companion_session(&self, companion_id: &str, robot_id: &str) -> anyhow::Result<String> {
        self.inner.ensure_companion_session(companion_id, robot_id).await
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
