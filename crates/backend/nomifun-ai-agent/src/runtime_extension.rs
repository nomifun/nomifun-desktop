//! Host-facing extension contract for user-developed execution runtimes.
//!
//! This is an implementation extension point, not a permission boundary for
//! untrusted code. Hosts register trusted factories; runtime tools must still
//! use the platform's admitted capability/model/resource ports. Session facts,
//! turn generations and teardown quarantine stay with the existing registry.

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use nomifun_api_types::{
    GetModelInfoResponse, SideQuestionRequest, SideQuestionResponse, SlashCommandItem,
};
use nomifun_common::{AgentKillReason, AppError};

use crate::{AgentRuntimeControl, SystemResourceNoticeDelivery};

pub type RuntimeTeardown = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send>>;

/// Object-safe production extension surface. Adding an engine does not require
/// adding an enum variant or changing Conversation/Remote/Automation callers.
///
/// A successful teardown is a proof of quiescence: all runtime tasks and owned
/// child processes are stopped. It is deliberately required, with no default
/// that silently treats a kill request as a completed cleanup.
#[async_trait]
pub trait RegisteredAgentRuntime: AgentRuntimeControl {
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

    fn notify_system_resource(
        &self,
        _notice: String,
    ) -> Result<SystemResourceNoticeDelivery, AppError> {
        Err(unsupported("system resource notifications"))
    }

    async fn ensure_can_rewind_last_turn(&self, _source_message_id: &str) -> Result<(), AppError> {
        Err(unsupported("rewind"))
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
    AppError::BadRequest(format!("The selected runtime does not support {operation}"))
}

// Nomi uses exactly the same extension seam as a user-developed runtime. The
// implementation stays here so product consumers never downcast the handle.
#[async_trait]
impl RegisteredAgentRuntime for crate::manager::nomi::NomiAgentManager {
    fn kill_and_wait(&self, reason: Option<AgentKillReason>) -> RuntimeTeardown {
        Self::kill_and_wait(self, reason)
    }

    async fn clear_context(&self) -> Result<(), AppError> {
        Self::clear_context(self).await
    }

    fn steer(&self, text: String) -> Result<bool, AppError> {
        Self::steer(self, text)
    }

    fn notify_system_resource(
        &self,
        notice: String,
    ) -> Result<SystemResourceNoticeDelivery, AppError> {
        Self::notify_system_resource(self, notice)
    }

    async fn ensure_can_rewind_last_turn(&self, source_message_id: &str) -> Result<(), AppError> {
        Self::ensure_can_rewind_last_turn(self, source_message_id).await
    }

    async fn rewind_last_turn(&self, source_message_id: &str) -> Result<(), AppError> {
        Self::rewind_last_turn(self, source_message_id).await
    }

    async fn get_slash_commands(&self) -> Result<Vec<SlashCommandItem>, AppError> {
        Self::get_slash_commands(self).await
    }
}
