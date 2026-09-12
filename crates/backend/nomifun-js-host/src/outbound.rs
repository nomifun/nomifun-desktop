use std::collections::VecDeque;
use std::io::{self, Write};

use nomifun_agent_contracts::CorrelationId;
use serde::Serialize;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

use crate::JavaScriptHostError;

struct Frame {
    bytes: Vec<u8>,
    written: usize,
    deadline: Instant,
    service_request_id: Option<CorrelationId>,
}

/// Owned by the generation Actor. No detached writer or unbounded output task.
pub(crate) struct OutboundQueue {
    frames: VecDeque<Frame>,
    capacity: usize,
    max_frame_bytes: usize,
}

impl OutboundQueue {
    pub(crate) fn new(capacity: usize, max_frame_bytes: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            capacity,
            max_frame_bytes,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub(crate) fn has_expired(&self, now: Instant) -> bool {
        self.frames.iter().any(|frame| frame.deadline <= now)
    }

    pub(crate) fn enqueue(
        &mut self,
        value: &impl Serialize,
        deadline: Instant,
        service_request_id: Option<CorrelationId>,
    ) -> Result<(), JavaScriptHostError> {
        if self.frames.len() >= self.capacity {
            return Err(JavaScriptHostError::QueueFull);
        }
        // Bound serialization itself, including JSON escapes and the newline;
        // checking to_vec().len() would first allocate the oversized copy.
        let mut buffer = FrameBuffer {
            bytes: Vec::new(),
            limit: self.max_frame_bytes,
        };
        serde_json::to_writer(&mut buffer, value)
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        buffer
            .write_all(b"\n")
            .map_err(|error| JavaScriptHostError::Contract(error.to_string()))?;
        self.frames.push_back(Frame {
            bytes: buffer.bytes,
            written: 0,
            deadline,
            service_request_id,
        });
        Ok(())
    }

    /// Each pollable step performs one cancellation-safe write, then flushes a
    /// completed frame before retiring it. The offset lives in the queue, so
    /// selecting a command cannot replay a prefix.
    pub(crate) async fn write_next(
        &mut self,
        stdin: &mut (impl AsyncWrite + Unpin),
    ) -> Result<Option<CorrelationId>, String> {
        let Some(frame) = self.frames.front_mut() else {
            return Ok(None);
        };
        if frame.deadline <= Instant::now() {
            return Err("Host IPC write timed out".into());
        }
        if frame.written < frame.bytes.len() {
            let written = stdin
                .write(&frame.bytes[frame.written..])
                .await
                .map_err(|error| format!("Host IPC write failed: {error}"))?;
            if written == 0 {
                return Err("Host IPC write returned zero bytes".into());
            }
            frame.written += written;
        }
        if frame.written == frame.bytes.len() {
            stdin
                .flush()
                .await
                .map_err(|error| format!("Host IPC flush failed: {error}"))?;
            return Ok(self
                .frames
                .pop_front()
                .and_then(|frame| frame.service_request_id));
        }
        Ok(None)
    }
}

struct FrameBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for FrameBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other(format!(
                "Host IPC frame exceeds {} bytes",
                self.limit
            )));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "outbound_tests.rs"]
mod tests;
