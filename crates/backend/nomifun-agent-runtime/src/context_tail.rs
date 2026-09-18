//! Atomic recent tool exchanges for derived context. Never executes a tool or
//! loads an artifact; replay selects only observations already in its history.
use std::collections::BTreeSet;

use nomifun_chat_model_broker::{
    ChatContentPart, ChatMessage, ChatRole, ChatToolResultPart, ToolCallId,
};

use crate::AgentEngineError;

const MAX_EXCHANGE_CALLS: usize = 64;
const MAX_TEXT_EXCHANGE_BYTES: usize = 32 * 1024;
const MAX_RETAINED_EXCHANGES: usize = 3;

pub(crate) struct RecentToolExchange<'a> {
    source: &'a [ChatMessage],
    start: usize,
    pub call_ids: Vec<ToolCallId>,
    has_images: bool,
    has_assistant_followup: bool,
    exchanges: usize,
}

fn invalid(message: &str) -> AgentEngineError {
    AgentEngineError::Compaction(message.into())
}

pub(crate) fn latest(
    messages: &[ChatMessage],
) -> Result<Option<RecentToolExchange<'_>>, AgentEngineError> {
    let Some(start) = messages.iter().rposition(|message| {
        message.role == ChatRole::Assistant
            && message
                .content
                .iter()
                .any(|part| matches!(part, ChatContentPart::ToolCall { .. }))
    }) else {
        return Ok(None);
    };
    let mut call_ids = Vec::new();
    let mut calls = BTreeSet::new();
    for part in &messages[start].content {
        if let ChatContentPart::ToolCall { call_id, .. } = part {
            if call_ids.len() >= MAX_EXCHANGE_CALLS
                || call_id.as_ref().is_empty()
                || call_id.as_ref().len() > 256
                || !calls.insert(call_id.clone())
            {
                return Err(invalid(
                    "Recent tool exchange has invalid or duplicate call identities",
                ));
            }
            call_ids.push(call_id.clone());
        }
    }
    if calls.is_empty() {
        return Ok(None);
    }
    let mut results = BTreeSet::new();
    let mut has_images = false;
    let mut has_assistant_followup = false;
    for message in &messages[start + 1..] {
        match message.role {
            ChatRole::User => {
                if message.content.iter().any(|part| {
                    matches!(
                        part,
                        ChatContentPart::ToolCall { .. } | ChatContentPart::ToolResult { .. }
                    )
                }) {
                    return Err(invalid("User input cannot supply recent tool identities"));
                }
            }
            ChatRole::Tool => {
                if message.content.is_empty() || has_assistant_followup {
                    return Err(invalid(
                        "Recent tool exchange has an empty or late result message",
                    ));
                }
                for part in &message.content {
                    let ChatContentPart::ToolResult {
                        call_id, output, ..
                    } = part
                    else {
                        return Err(invalid(
                            "Recent tool result message contains non-result content",
                        ));
                    };
                    if output.is_empty()
                        || !calls.contains(call_id)
                        || !results.insert(call_id.clone())
                    {
                        return Err(invalid(
                            "Recent tool exchange has inconsistent call/result identities",
                        ));
                    }
                    has_images |= output
                        .iter()
                        .any(|part| matches!(part, ChatToolResultPart::Image { .. }));
                }
            }
            ChatRole::Assistant => {
                if results != calls
                    || message.content.is_empty()
                    || message.content.iter().any(|part| {
                        matches!(
                            part,
                            ChatContentPart::ToolCall { .. } | ChatContentPart::ToolResult { .. }
                        )
                    })
                {
                    return Err(invalid(
                        "Assistant follow-up precedes complete tool results or contains unmatched tool content",
                    ));
                }
                has_assistant_followup = true;
            }
            _ => return Err(invalid("Unexpected role after the recent tool batch")),
        }
    }
    if calls != results {
        return Err(invalid(
            "Recent tool exchange is incomplete; compaction cannot repair missing results",
        ));
    }
    Ok(Some(RecentToolExchange {
        source: messages,
        start,
        call_ids,
        has_images,
        has_assistant_followup,
        exchanges: 1,
    }))
}

/// Resolve an exact contiguous suffix from a durable compaction record. The
/// IDs select observations already present here, never an archive lookup or
/// permission to reconstruct missing results. Single-batch records still work.
pub(crate) fn selected<'a>(
    messages: &'a [ChatMessage],
    call_ids: &[ToolCallId],
) -> Result<Option<RecentToolExchange<'a>>, AgentEngineError> {
    let Some(mut exchange) = latest(messages)? else {
        return Ok(None);
    };
    loop {
        if exchange.call_ids == call_ids {
            return Ok(Some(exchange));
        }
        if exchange.call_ids.len() >= call_ids.len() {
            return Ok(None);
        }
        let Some(earlier) = exchange.earlier()? else {
            return Ok(None);
        };
        exchange = earlier;
    }
}

impl<'a> RecentToolExchange<'a> {
    /// Expand by one whole preceding batch, retaining intervening messages.
    /// Bound both batch count and total IDs under the existing event envelope.
    /// Repeated IDs across historical turns make expansion ambiguous; retain
    /// the newer suffix instead. Never skip an oversized/intervening batch.
    pub fn earlier(&self) -> Result<Option<Self>, AgentEngineError> {
        if self.exchanges >= MAX_RETAINED_EXCHANGES {
            return Ok(None);
        }
        // System/developer messages are not optional transcript tail data.
        // Do not widen across such a boundary or reinterpret their authority.
        let boundary = self.source[..self.start]
            .iter()
            .rposition(|message| {
                !matches!(
                    message.role,
                    ChatRole::User | ChatRole::Assistant | ChatRole::Tool
                )
            })
            .map_or(0, |index| index + 1);
        let Some(previous) = latest(&self.source[boundary..self.start])? else {
            return Ok(None);
        };
        if previous.call_ids.len() + self.call_ids.len() > MAX_EXCHANGE_CALLS
            || previous
                .call_ids
                .iter()
                .any(|id| self.call_ids.contains(id))
        {
            return Ok(None);
        }
        let mut call_ids = previous.call_ids;
        call_ids.extend(self.call_ids.iter().cloned());
        Ok(Some(Self {
            source: self.source,
            start: boundary + previous.start,
            call_ids,
            // Only the newest batch can still have unseen pixels; every
            // earlier batch already preceded another complete model response.
            has_images: self.has_images,
            has_assistant_followup: self.has_assistant_followup,
            exchanges: self.exchanges + 1,
        }))
    }

    /// A response after the complete batch shows a model boundary already
    /// passed with these results in context, not that the model understood or
    /// verified them. Without such a boundary, unseen images stay mandatory.
    pub fn requires_original_images(&self) -> bool {
        self.has_images && !self.has_assistant_followup
    }

    /// User attachments already belong to mandatory inputs, not to the
    /// optional tail budget. Count calls/results, assistant follow-ups and
    /// engine notices without a second unbounded serialized allocation.
    pub fn fits_text_bound(&self, requirements: &[ChatMessage]) -> Result<bool, AgentEngineError> {
        let (_, accepted_positions) = self.required_positions(requirements)?;
        let exchange = self.source[self.start..]
            .iter()
            .enumerate()
            // Only actual accepted inputs are separately mandatory. Engine
            // review/continuation notices use User role too and must count.
            .filter(|(offset, _)| !accepted_positions.contains(&(self.start + *offset)))
            .map(|(_, message)| message)
            .collect::<Vec<_>>();
        Ok(crate::stream_limits::serialized_size(&exchange, MAX_TEXT_EXCHANGE_BYTES).is_ok())
    }

    /// Keep accepted inputs once, with their multiplicity and order intact.
    /// Reverse subsequence matching handles identical repeated user messages;
    /// equality excludes provider state, which is reset by compaction.
    /// Inputs after the batch stay AFTER it, not before the assistant call.
    pub fn with_required_inputs(
        &self,
        requirements: &[ChatMessage],
    ) -> Result<Vec<ChatMessage>, AgentEngineError> {
        let (prefix_count, _) = self.required_positions(requirements)?;
        let mut retained = requirements[..prefix_count].to_vec();
        retained.extend_from_slice(&self.source[self.start..]);
        for message in &mut retained {
            message.provider_round_id = None;
        }
        Ok(retained)
    }

    fn required_positions(
        &self,
        requirements: &[ChatMessage],
    ) -> Result<(usize, BTreeSet<usize>), AgentEngineError> {
        let mut cursor = self.source.len();
        let mut prefix_count = requirements.len();
        let mut positions = BTreeSet::new();
        for requirement in requirements.iter().rev() {
            let Some(index) = self.source[..cursor].iter().rposition(|message| {
                message.role == requirement.role && message.content == requirement.content
            }) else {
                return Err(invalid(
                    "Accepted input is missing from the context being compacted",
                ));
            };
            if index >= self.start {
                prefix_count -= 1;
            }
            positions.insert(index);
            cursor = index;
        }
        Ok((prefix_count, positions))
    }
}
