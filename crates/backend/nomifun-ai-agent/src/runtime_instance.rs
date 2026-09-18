//! Host-facing contract for the one official Runtime instance.
//!
//! This is an internal implementation seam, not a registration SDK or a
//! permission boundary. Runtime operations consume only platform-admitted
//! capability/model/resource ports; Session facts, turn generations and
//! teardown quarantine stay with their host owners.

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use nomifun_api_types::{
    GetModelInfoResponse, SideQuestionRequest, SideQuestionResponse, SlashCommandItem,
};
use nomifun_common::{AgentKillReason, AppError};

use crate::{AgentRuntimeControl, SystemResourceNoticeDelivery};

pub type RuntimeTeardown = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send>>;

/// Internal delivery from the Conversation owner after receipt admission.
/// This is not a public request DTO or a substitute for host-side validation.
#[derive(Clone, Debug)]
pub struct RuntimeSteerDelivery {
    pub receipt_operation_id: String,
    pub wire_turn_id: String,
    pub turn_generation: u64,
    pub text: String,
    pub files: Vec<String>,
    pub inject_skills: Vec<String>,
}

/// A successful teardown is a proof of quiescence: all runtime tasks and owned
/// child processes are stopped. It is deliberately required, with no default
/// that silently treats a kill request as a completed cleanup.
#[async_trait]
pub trait OfficialAgentRuntime: AgentRuntimeControl {
    fn kill_and_wait(&self, reason: Option<AgentKillReason>) -> RuntimeTeardown;

    fn requires_turn_boundary_recycle(&self) -> bool {
        false
    }

    async fn clear_context(&self) -> Result<(), AppError> {
        Err(unsupported("clear context"))
    }

    fn steer(&self, _text: String) -> Result<bool, AppError> {
        Err(unsupported("steer"))
    }

    /// Supports receipt-backed attachments and hints for already-selected
    /// Skills. This declares transport support, never an additional grant.
    fn supports_steering_context(&self) -> bool { false }

    /// True means queued, not model-consumed or task-completed.
    async fn steer_with_receipt(&self, delivery: RuntimeSteerDelivery) -> Result<bool, AppError> {
        if !delivery.files.is_empty() || !delivery.inject_skills.is_empty() {
            return Err(unsupported("steering attachments or Skill hints"));
        }
        self.steer(delivery.text)
    }

    fn notify_system_resource(
        &self,
        _notice: String,
    ) -> Result<SystemResourceNoticeDelivery, AppError> {
        Err(unsupported("system resource notifications"))
    }

    async fn ensure_can_rewind_last_turn(&self, _source_message_id: &str) -> Result<(), AppError> {
        Err(unsupported("rewind"))
    }

    /// Automatic resend of an accepted source needs an engine/owner proof.
    /// A completed cleanup or empty assistant output is not a replay permit.
    async fn ensure_can_retry_turn(&self, _source_message_id: &str) -> Result<(), AppError> {
        Err(unsupported("automatic turn replay"))
    }

    async fn rewind_last_turn(&self, _source_message_id: &str) -> Result<(), AppError> {
        Err(unsupported("rewind"))
    }

    async fn get_model(&self) -> Result<GetModelInfoResponse, AppError> {
        Ok(GetModelInfoResponse { model_info: None })
    }

    async fn set_model(&self, _model_id: &str) -> Result<(), AppError> {
        Err(unsupported("model switching"))
    }

    async fn get_slash_commands(&self) -> Result<Vec<SlashCommandItem>, AppError> {
        Ok(Vec::new())
    }

    async fn handle_side_question(
        &self,
        _request: SideQuestionRequest,
    ) -> Result<SideQuestionResponse, AppError> {
        Ok(SideQuestionResponse {
            status: "unsupported".to_owned(),
            answer: None,
        })
    }
}

fn unsupported(operation: &str) -> AppError {
    AppError::BadRequest(format!("The official Runtime does not support {operation}"))
}
