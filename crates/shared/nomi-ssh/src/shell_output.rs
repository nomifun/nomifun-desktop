//! Decode the byte stream, not individual SSH packets. Keep at most three
//! incomplete UTF-8 bytes until the next packet, or replace them on completion.
use std::ops::Deref;

use crate::limits::validate_output_size;

#[derive(Default)]
pub(super) struct ShellOutput {
    text: String,
    pending: Vec<u8>,
}

impl Deref for ShellOutput {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl ShellOutput {
    pub fn append(&mut self, data: &[u8]) -> Result<(), usize> {
        if self.pending.is_empty() {
            self.append_complete(data)
        } else {
            let mut joined = std::mem::take(&mut self.pending);
            joined.extend_from_slice(data);
            self.append_complete(&joined)
        }
    }

    fn append_complete(&mut self, data: &[u8]) -> Result<(), usize> {
        // utf8_chunks already knows UTF-8 boundaries, including malformed
        // sequences. Only its final incomplete sequence belongs to a later
        // packet; complete invalid sequences retain ordinary lossy semantics.
        let pending_len = data.utf8_chunks().last().map_or(0, |chunk| {
            let invalid = chunk.invalid();
            if std::str::from_utf8(invalid).is_err_and(|e| e.error_len().is_none()) {
                invalid.len()
            } else {
                0
            }
        });
        let complete_len = data.len() - pending_len;
        let decoded = String::from_utf8_lossy(&data[..complete_len]);
        let actual = self
            .text
            .len()
            .saturating_add(decoded.len())
            .saturating_add(pending_len);
        validate_output_size(actual).map_err(|_| actual)?;
        self.text.push_str(&decoded);
        self.pending.extend_from_slice(&data[complete_len..]);
        Ok(())
    }

    /// Flush an incomplete final character once, including on timeout/EOF.
    /// Its replacement can expand, so completion also checks the output cap.
    pub fn finish(&mut self) -> Result<(), usize> {
        let tail = String::from_utf8_lossy(&self.pending);
        let actual = self.text.len().saturating_add(tail.len());
        validate_output_size(actual).map_err(|_| actual)?;
        self.text.push_str(&tail);
        self.pending.clear();
        Ok(())
    }
}
