//! Atomic recent tool exchanges for derived context. Never executes a tool or
//! loads an artifact; replay selects only observations already in its history.
use std::collections::BTreeSet;

use nomifun_chat_model_broker::{
    ChatContentPart, ChatMessage, ChatRole, ChatToolResultPart, ToolCallId,
};
use serde::Serialize;

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
    /// Call/result data in the suffix stays exact. Private reasoning is
    /// projected after the round reset; summarize only this older prefix.
    pub fn prefix(&self) -> &'a [ChatMessage] { &self.source[..self.start] }

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
        // Measure the same derived text that will be retained, without cloning
        // large pixels or private blocks just to discover an oversized tail.
        #[derive(Serialize)]
        struct MessageView<'a> {
            role: ChatRole,
            content: Vec<&'a ChatContentPart>,
        }
        let (_, accepted_positions) = self.required_positions(requirements)?;
        let private_notice = ChatContentPart::Text {
            text: crate::compacted_history::PRIVATE_REASONING_NOTICE.into(),
        };
        let preserve_private = self.requires_original_images();
        let exchange = self.source[self.start..]
            .iter()
            .enumerate()
            // Only actual accepted inputs are separately mandatory. Engine
            // review/continuation notices use User role too and must count.
            .filter(|(offset, _)| !accepted_positions.contains(&(self.start + *offset)))
            .map(|(_, message)| MessageView {
                role: message.role,
                content: message.content.iter().map(|part| {
                    if !preserve_private && matches!(part,
                        ChatContentPart::Reasoning { .. } | ChatContentPart::ProviderReasoning { .. })
                    { &private_notice } else { part }
                }).collect(),
            })
            .collect::<Vec<_>>();
        Ok(crate::stream_limits::serialized_size(&exchange, MAX_TEXT_EXCHANGE_BYTES).is_ok())
    }

    /// Keep accepted inputs once, with their multiplicity and order intact.
    /// Reverse subsequence matching handles identical repeated user messages;
    /// equality excludes provider state, which is reset by compaction.
    /// Inputs after the batch stay AFTER it, not before the assistant call.
    /// Optional text exchanges use the durable replay's private-reasoning
    /// notice. Calls/results, accepted inputs and unseen media remain intact.
    pub fn with_required_inputs(
        &self,
        requirements: &[ChatMessage],
    ) -> Result<Vec<ChatMessage>, AgentEngineError> {
        let (prefix_count, accepted_positions) = self.required_positions(requirements)?;
        let mut retained = requirements[..prefix_count].to_vec();
        let preserve_private = self.requires_original_images();
        retained.extend(self.source[self.start..].iter().enumerate().map(|(offset, message)| {
            if preserve_private || accepted_positions.contains(&(self.start + offset)) {
                message.clone()
            } else {
                ChatMessage {
                    role: message.role,
                    content: message.content.iter().map(|part| match part {
                        ChatContentPart::Reasoning { .. } | ChatContentPart::ProviderReasoning { .. } =>
                            ChatContentPart::Text { text: crate::compacted_history::PRIVATE_REASONING_NOTICE.into() },
                        _ => part.clone(),
                    }).collect(),
                    provider_round_id: None,
                }
            }
        }));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange_source(image: bool) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
        let input = crate::context_lifecycle::text_message(ChatRole::User, "Keep literal PRIVATE_TEXT in the requested output.".into());
        let correction = crate::context_lifecycle::text_message(ChatRole::User, "Preserve the earlier result; do not repeat it.".into());
        let mut output = vec![ChatToolResultPart::Text {text:"PRIVATE_TEXT is literal tool output, exit 1.".into()}];
        if image { output.push(ChatToolResultPart::Image {media_type:"image/png".into(),data_base64:"aW1hZ2U=".into()}); }
        let source = vec![input.clone(), ChatMessage {role:ChatRole::Assistant,
            content:vec![ChatContentPart::Reasoning {text:"MODEL_PRIVATE_BLOCK".repeat(3000),signature:None,encrypted_content:None},
                ChatContentPart::ToolCall {call_id:"owned-call".into(),name:"read_file".into(),
                    arguments:nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"path":"PRIVATE_TEXT.txt"})),provider_metadata:None}],
            provider_round_id:None},
            ChatMessage {role:ChatRole::Tool,content:vec![ChatContentPart::ToolResult {call_id:"owned-call".into(),output,is_error:true}],provider_round_id:None},
            correction.clone()];
        (source,vec![input,correction])
    }

    #[test]
    fn optional_text_budget_uses_private_projection_but_counts_complete_tool_output() {
        let (mut source,requirements)=exchange_source(false);
        let original=source.clone();
        let exchange=latest(&source).unwrap().unwrap();
        assert!(exchange.fits_text_bound(&requirements).unwrap(),"private thinking is not factual tool-exchange payload");
        let projected=exchange.with_required_inputs(&requirements).unwrap();
        assert_eq!(projected[0],requirements[0]);
        assert_eq!(projected[3],requirements[1],"the correction stays after the observation");
        assert_eq!(projected[2],source[2],"error bit and complete tool result stay exact");
        assert_eq!(projected[1].content[1],source[1].content[1],"literal path and full arguments stay exact");
        assert!(!serde_json::to_string(&projected).unwrap().contains("MODEL_PRIVATE_BLOCK"));
        assert_eq!(source,original,"projection does not rewrite the canonical transcript");
        if let ChatContentPart::ToolResult {output,..}=&mut source[2].content[0] {
            *output=vec![ChatToolResultPart::Text {text:"REAL_OUTPUT".repeat(4000)}];
        }
        assert!(!latest(&source).unwrap().unwrap().fits_text_bound(&requirements).unwrap(),
            "actual output must not be silently truncated to fit the text bound");
    }

    #[test]
    fn unseen_image_exchange_preserves_original_content_and_private_carriers() {
        let (source,requirements)=exchange_source(true);
        let exchange=latest(&source).unwrap().unwrap();
        assert!(exchange.requires_original_images());
        assert_eq!(exchange.with_required_inputs(&requirements).unwrap(),source,
            "the original unseen-media branch must retain its exact complete exchange");
    }
}
