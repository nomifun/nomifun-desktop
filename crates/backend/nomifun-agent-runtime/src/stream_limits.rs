//! Count the complete wire envelope without allocating a second serialized copy.
use std::io::{self, Write};

use nomifun_chat_model_broker::{ChatModelEvent, ChatToolCall, ToolCallId};
use serde::Serialize;

use crate::AgentEngineError;

pub(crate) const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENTS: usize = 65_536;

#[derive(Default)]
pub(crate) struct StreamBudget {
    bytes: usize,
    events: usize,
}

impl StreamBudget {
    pub(crate) fn admit(&mut self, event: &ChatModelEvent) -> Result<(), AgentEngineError> {
        self.events = self.events.saturating_add(1);
        if self.events > MAX_EVENTS {
            return Err(invalid("model stream event count exceeded"));
        }
        match event {
            ChatModelEvent::ToolCallDelta { call_id, name, .. } => identity(call_id, name)?,
            ChatModelEvent::ToolCallCompleted { call } => completed(call)?,
            ChatModelEvent::ProviderRoundId { round_id } if round_id.as_ref().len() > 4096 => {
                return Err(invalid("provider round identity exceeds 4096 bytes"));
            }
            ChatModelEvent::Usage { usage } => {
                serialized_size(usage, 16 * 1024)?;
            }
            _ => {}
        }
        self.bytes += serialized_size(event, MAX_STREAM_BYTES.saturating_sub(self.bytes))?;
        Ok(())
    }
}

pub(crate) fn identity(id: &ToolCallId, name: &str) -> Result<(), AgentEngineError> {
    let id = id.as_ref();
    if id.trim().is_empty()
        || id.len() > 256
        || id.chars().any(char::is_control)
        || id.starts_with("agent-instructions:")
        || name.len() > 128
        || name.chars().any(char::is_control)
    {
        return Err(invalid("invalid or oversized model tool identity"));
    }
    Ok(())
}

pub(crate) fn completed(call: &ChatToolCall) -> Result<(), AgentEngineError> {
    identity(&call.call_id, &call.name)?;
    if let Some(metadata) = &call.provider_metadata {
        serialized_size(metadata, 16 * 1024)?;
    }
    Ok(())
}

fn invalid(message: &str) -> AgentEngineError {
    AgentEngineError::InvalidModelEvent(message.into())
}

pub(crate) fn serialized_size(
    value: &impl Serialize,
    limit: usize,
) -> Result<usize, AgentEngineError> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let next = self.bytes.saturating_add(bytes.len());
            if next > self.limit {
                return Err(io::Error::other(
                    "model stream envelope byte budget exceeded",
                ));
            }
            self.bytes = next;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(|error| invalid(&error.to_string()))?;
    Ok(counter.bytes)
}
