use std::io::{self, Stdout, Write};

use crate::events::ProtocolEvent;

/// Trait for emitting protocol events to a host.
///
/// The default implementation (`ProtocolWriter`) writes JSON Lines to stdout.
/// Backend integrations provide alternative implementations that bridge events
/// to their own event systems.
pub trait ProtocolEmitter: Send + Sync {
    fn emit(&self, event: &ProtocolEvent) -> io::Result<()>;
}

/// Thread-safe JSON Lines writer to stdout
pub struct ProtocolWriter {
    writer: Stdout,
}

impl Default for ProtocolWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolWriter {
    pub fn new() -> Self {
        Self {
            writer: io::stdout(),
        }
    }
}

impl ProtocolEmitter for ProtocolWriter {
    fn emit(&self, event: &ProtocolEvent) -> io::Result<()> {
        // Hold stdout's shared lock for the entire frame, even across different
        // ProtocolWriter instances. Stdout already owns a line buffer.
        let mut w = self.writer.lock();
        serde_json::to_writer(&mut w, event)
            .map_err(|e| io::Error::other(format!("failed to serialize protocol event: {}", e)))?;
        writeln!(&mut w)?;
        w.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Capabilities, ProtocolEvent};

    #[test]
    fn test_writer_construction() {
        let _writer = ProtocolWriter::new();
    }

    #[test]
    fn test_writer_emit_does_not_panic() {
        let writer = ProtocolWriter::new();
        let event = ProtocolEvent::Ready {
            version: "0.1.0".to_string(),
            session_id: None,
            capabilities: Capabilities {
                thinking: false,
                effort: false,
                effort_levels: vec![],
                mcp: false,
            },
        };
        writer.emit(&event).expect("protocol event should reach stdout");
    }
}
