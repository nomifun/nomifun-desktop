use std::sync::Arc;

use nomi_config::compat::ProviderCompat;
use nomi_protocol::events::{Capabilities, ErrorInfo, ProtocolEvent, Usage};
use nomi_protocol::writer::{ProtocolEmitter, ProtocolWriter};

use super::OutputSink;

/// JSON stream protocol output sink
pub struct ProtocolSink {
    writer: Arc<ProtocolWriter>,
}

impl ProtocolSink {
    pub fn new(writer: Arc<ProtocolWriter>) -> Self {
        Self { writer }
    }

    /// Emit the ready event at session start
    pub fn emit_ready(
        &self,
        compat: &ProviderCompat,
        has_mcp: bool,
        session_id: Option<String>,
    ) {
        let _ = self.writer.emit(&ProtocolEvent::Ready {
            version: env!("CARGO_PKG_VERSION").to_string(),
            session_id,
            capabilities: Self::build_capabilities(compat, has_mcp),
        });
    }

    /// Emit a config_changed event after a runtime configuration update.
    pub fn emit_config_changed(&self, compat: &ProviderCompat, has_mcp: bool) {
        let _ = self.writer.emit(&ProtocolEvent::ConfigChanged {
            capabilities: Self::build_capabilities(compat, has_mcp),
        });
    }

    /// Access the underlying writer for custom events
    pub fn writer(&self) -> &Arc<ProtocolWriter> {
        &self.writer
    }

    fn build_capabilities(compat: &ProviderCompat, has_mcp: bool) -> Capabilities {
        Capabilities {
            thinking: compat.supports_thinking(),
            effort: compat.supports_effort(),
            effort_levels: compat.effort_levels().to_vec(),
            mcp: has_mcp,
        }
    }
}

impl OutputSink for ProtocolSink {
    fn emit_text_delta(&self, text: &str, msg_id: &str) {
        let _ = self.writer.emit(&ProtocolEvent::TextDelta {
            text: text.to_string(),
            msg_id: msg_id.to_string(),
        });
    }

    fn emit_thinking(&self, text: &str, msg_id: &str) {
        let _ = self.writer.emit(&ProtocolEvent::Thinking {
            text: text.to_string(),
            msg_id: msg_id.to_string(),
        });
    }

    fn emit_tool_call(&self, _tool_use_id: &str, name: &str, _input: &str) {
        // In protocol mode, tool_call is handled by tool_running events.
        // This is a fallback for compatibility.
        let _ = self.writer.emit(&ProtocolEvent::Info {
            msg_id: String::new(),
            message: format!("Tool call: {name}"),
        });
    }

    fn emit_tool_result(&self, _tool_use_id: &str, name: &str, is_error: bool, content: &str) {
        // In protocol mode, tool results are emitted via explicit ToolResult events
        // with call_id. This fallback emits an info event.
        let status = if is_error { "error" } else { "success" };
        let _ = self.writer.emit(&ProtocolEvent::Info {
            msg_id: String::new(),
            message: format!("[{name} {status}] {content}"),
        });
    }

    fn emit_stream_start(&self, msg_id: &str) {
        let _ = self.writer.emit(&ProtocolEvent::StreamStart {
            msg_id: msg_id.to_string(),
        });
    }

    fn emit_output_discarded(&self, msg_id: &str, restart_attempt: u32) {
        let _ = self.writer.emit(&ProtocolEvent::OutputDiscarded {
            msg_id: msg_id.to_string(),
            restart_attempt,
        });
    }

    fn emit_stream_end(
        &self,
        msg_id: &str,
        _turns: usize,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
    ) {
        let _ = self.writer.emit(&ProtocolEvent::StreamEnd {
            msg_id: msg_id.to_string(),
            usage: Some(Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens: if cache_read_tokens > 0 {
                    Some(cache_read_tokens)
                } else {
                    None
                },
                cache_write_tokens: if cache_creation_tokens > 0 {
                    Some(cache_creation_tokens)
                } else {
                    None
                },
            }),
        });
    }

    fn emit_error(&self, msg: &str) {
        let _ = self.writer.emit(&ProtocolEvent::Error {
            msg_id: None,
            error: ErrorInfo {
                code: "engine_error".to_string(),
                message: msg.to_string(),
                retryable: false,
            },
        });
    }

    fn emit_info(&self, msg: &str) {
        let _ = self.writer.emit(&ProtocolEvent::Info {
            msg_id: String::new(),
            message: msg.to_string(),
        });
    }
}
