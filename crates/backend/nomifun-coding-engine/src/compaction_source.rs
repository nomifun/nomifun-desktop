//! Bounded, record-aware data for the summary model, never live tool input.
use nomifun_chat_model_broker::{ChatMessage, ChatRole};

use crate::CodingEngineError;

const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct SummarySource {
    text: String,
    records: Vec<(usize, ChatRole)>,
}

pub(crate) struct SummaryChunk<'a> {
    pub text: &'a str,
    pub start: usize,
    pub end: usize,
    pub first_message: usize,
    pub last_message: usize,
    pub first_role: ChatRole,
    pub starts_mid_message: bool,
    pub ends_mid_message: bool,
}

impl SummarySource {
    /// The caller removes binary/private fields before this method. One JSON
    /// message per line makes complete records self-contained summary data.
    pub fn push(&mut self, message: &ChatMessage) -> Result<(), CodingEngineError> {
        crate::stream_limits::serialized_size(
            message,
            MAX_SOURCE_BYTES
                .saturating_sub(self.text.len())
                .saturating_sub(1),
        )
        .map_err(|_| invalid("sanitized summary source exceeds 64 MiB"))?;
        let encoded = serde_json::to_string(message)
            .map_err(|_| invalid("summary message could not be serialized"))?;
        self.text.push_str(&encoded);
        self.text.push('\n');
        self.records.push((self.text.len(), message.role));
        Ok(())
    }

    /// Plan every request before spending any model operation. Prefer the
    /// last complete message in the window. Oversized messages are split on
    /// UTF-8 boundaries with explicit fragment metadata. If record packing
    /// exceeds the call budget, retain the previous dense-fragment capacity.
    /// Coverage is contiguous: never drop, overlap, reorder or skip a record.
    pub fn chunks(
        &self,
        max_bytes: usize,
        max_calls: usize,
    ) -> Result<Vec<SummaryChunk<'_>>, CodingEngineError> {
        if max_bytes < 4 || self.records.is_empty() {
            return Err(invalid("summary source or chunk budget is empty"));
        }
        self.plan(max_bytes, max_calls, true)
            .or_else(|| self.plan(max_bytes, max_calls, false))
            .ok_or_else(|| invalid("source exceeds the remaining bounded compaction budget"))
    }

    fn plan(
        &self,
        max_bytes: usize,
        max_calls: usize,
        prefer_records: bool,
    ) -> Option<Vec<SummaryChunk<'_>>> {
        let mut chunks = Vec::new();
        let mut start = 0usize;
        while start < self.text.len() {
            if chunks.len() >= max_calls {
                return None;
            }
            let first = self.records.partition_point(|(end, _)| *end <= start);
            let hard_end = start.saturating_add(max_bytes).min(self.text.len());
            let complete = self.records.partition_point(|(end, _)| *end <= hard_end);
            let mut end = if prefer_records && complete > first {
                self.records[complete - 1].0
            } else {
                hard_end
            };
            while !self.text.is_char_boundary(end) {
                end -= 1;
            }
            if end <= start {
                return None;
            }
            let last = self
                .records
                .partition_point(|(record_end, _)| *record_end < end);
            let message_start = if first == 0 {
                0
            } else {
                self.records[first - 1].0
            };
            chunks.push(SummaryChunk {
                text: &self.text[start..end],
                start,
                end,
                first_message: first,
                last_message: last,
                first_role: self.records[first].1,
                starts_mid_message: start != message_start,
                ends_mid_message: end != self.records[last].0,
            });
            start = end;
        }
        Some(chunks)
    }
}

fn invalid(message: &str) -> CodingEngineError {
    CodingEngineError::Compaction(message.into())
}
