use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{ActionId, CapabilityId, OperationId};
use nomifun_chat_model_broker::{ChatFinishReason, ChatToolCall, ChatUsage, ToolCallId};

use crate::engine::EngineBinding;
use crate::error::CodingEngineError;
use crate::tool::CodingToolResult;

#[derive(Clone, Debug, PartialEq)]
pub enum CodingEngineEvent {
    TurnStarted {
        binding: EngineBinding,
        turn_operation_id: OperationId,
    },
    ModelStepStarted {
        step: u16,
        operation_id: OperationId,
    },
    OutputTextDelta {
        step: u16,
        text: String,
    },
    ReasoningDelta {
        step: u16,
        text: String,
    },
    ToolCallDelta {
        step: u16,
        call_id: ToolCallId,
        name: String,
        arguments_delta: String,
    },
    ToolCallCompleted {
        step: u16,
        call: ChatToolCall,
    },
    ToolStarted {
        step: u16,
        call_id: ToolCallId,
        capability_id: CapabilityId,
        action_id: ActionId,
    },
    ToolCompleted {
        step: u16,
        result: CodingToolResult,
    },
    Usage {
        step: u16,
        usage: ChatUsage,
    },
    TurnCompleted {
        model_steps: u16,
        finish_reason: ChatFinishReason,
    },
    TurnCancelled {
        model_steps: u16,
    },
    TurnFailed {
        model_steps: u16,
        message: String,
    },
}

#[async_trait]
pub trait CodingEventSink: Send + Sync {
    async fn emit(&self, event: CodingEngineEvent) -> Result<(), CodingEngineError>;
}

#[derive(Clone, Default)]
pub struct NoopCodingEventSink;

#[async_trait]
impl CodingEventSink for NoopCodingEventSink {
    async fn emit(&self, _event: CodingEngineEvent) -> Result<(), CodingEngineError> {
        Ok(())
    }
}

pub(crate) type SharedCodingEventSink = Arc<dyn CodingEventSink>;
