//! Bound an attempt's complete decoded wire data before accumulating fields.
//! Serialization counts without allocating a second copy of media or text.
use crate::ChatModelError;
use serde_json::Value;
use std::io::{self, Write};

#[derive(Default)]
pub(crate) struct WireBudget {
    bytes: usize,
    frames: usize,
}

impl WireBudget {
    pub(crate) fn admit(&mut self, value: &Value) -> Result<(), ChatModelError> {
        self.frames += 1;
        if self.frames > 65_536 {
            return Err(ChatModelError::protocol_violation(
                "provider wire frame limit exceeded",
            ));
        }
        self.bytes += encoded_size(value, (16 * 1024 * 1024_usize).saturating_sub(self.bytes))?;
        Ok(())
    }
}

pub(crate) fn encoded_size(value: &Value, limit: usize) -> Result<usize, ChatModelError> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes) {
                return Err(io::Error::other("provider wire byte limit"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value)
        .map_err(|_| ChatModelError::protocol_violation("provider wire byte limit exceeded"))?;
    Ok(counter.bytes)
}
