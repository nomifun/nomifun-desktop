//! Bounded, record-aware data for the summary model, never live tool input.
use nomifun_chat_model_broker::{ChatMessage, ChatRole};

use crate::AgentEngineError;

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
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// The caller removes binary/private fields before this method. One JSON
    /// message per line makes complete records self-contained summary data.
    pub fn push(&mut self, message: &ChatMessage) -> Result<(), AgentEngineError> {
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
    ) -> Result<Vec<SummaryChunk<'_>>, AgentEngineError> {
        if max_bytes < 4 || self.records.is_empty() {
            return Err(invalid("summary source or chunk budget is empty"));
        }
        self.plan(max_bytes, max_calls, true)
            .or_else(|| self.plan(max_bytes, max_calls, false))
            .ok_or_else(|| invalid("source exceeds the remaining bounded compaction budget"))
    }

    /// Split a rejected summary fragment without losing or repeating bytes.
    /// Prefer a message boundary, then use a UTF-8 boundary when one message
    /// itself is too large. Both halves retain exact source coordinates.
    pub fn split_range(&self, start: usize, end: usize) -> Option<(SummaryChunk<'_>, SummaryChunk<'_>)> {
        if end <= start + 512 || end > self.text.len() { return None; }
        let halfway = start + (end - start) / 2;
        let boundary = self.records.iter()
            .map(|(record_end, _)| *record_end)
            .filter(|record_end| *record_end > start + 256 && *record_end < end - 256)
            .min_by_key(|record_end| record_end.abs_diff(halfway));
        let mut middle = boundary.unwrap_or(halfway);
        while middle < end && !self.text.is_char_boundary(middle) { middle += 1; }
        if middle <= start + 256 || middle >= end - 256 { return None; }
        Some((self.chunk(start, middle)?, self.chunk(middle, end)?))
    }

    fn chunk(&self, start: usize, end: usize) -> Option<SummaryChunk<'_>> {
        if start >= end || end > self.text.len()
            || !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) { return None; }
        let first = self.records.partition_point(|(record_end, _)| *record_end <= start);
        let last = self.records.partition_point(|(record_end, _)| *record_end < end);
        let first_role = self.records.get(first)?.1;
        let message_start = if first == 0 { 0 } else { self.records[first - 1].0 };
        Some(SummaryChunk {
            text: &self.text[start..end], start, end,
            first_message: first, last_message: last, first_role,
            starts_mid_message: start != message_start,
            ends_mid_message: end != self.records.get(last)?.0,
        })
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
            chunks.push(self.chunk(start, end)?);
            start = end;
        }
        Some(chunks)
    }
}

fn invalid(message: &str) -> AgentEngineError {
    AgentEngineError::Compaction(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_chat_model_broker::{ChatContentPart, ChatMessage};

    #[test]
    fn split_retry_fragments_cover_exact_utf8_source_once() {
        let mut source = SummarySource::default();
        source.push(&ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text { text: "é".repeat(600) }],
            provider_round_id: None,
        }).unwrap();
        let chunks = source.chunks(4096, 1).unwrap();
        let original = &chunks[0];
        let (first, second) = source.split_range(original.start, original.end).unwrap();
        assert_eq!(first.end, second.start);
        assert_eq!(format!("{}{}", first.text, second.text), original.text);
        assert!(first.ends_mid_message);
        assert!(second.starts_mid_message);
        assert_eq!(first.first_message, second.first_message);
    }
}
