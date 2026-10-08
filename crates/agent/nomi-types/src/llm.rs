use serde_json::Value;

use crate::message::{StopReason, TokenUsage, ToolUseId};
use crate::tool::ToolDef;

/// A request to the LLM provider
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<crate::message::Message>,
    pub tools: Vec<ToolDef>,
    /// Output-token ceiling to put on the provider request. `None` means the
    /// serializer must omit the field and let the provider apply its default.
    pub max_tokens: Option<u32>,
    /// Optional: thinking config (Anthropic extended thinking)
    pub thinking: Option<ThinkingConfig>,
    /// Optional: reasoning effort for OpenAI reasoning models (low/medium/high)
    pub reasoning_effort: Option<String>,
    /// Whether this request belongs to a durable agent round that may retain
    /// provider-side state and consume the resulting round cursor. Providers
    /// must still require their own explicit compatibility opt-in; this bit is
    /// the per-request lifecycle half of that two-key decision.
    pub retain_provider_round: bool,
}

#[derive(Debug, Clone)]
pub enum ThinkingConfig {
    Enabled { budget_tokens: u32 },
    Disabled,
}

/// Streaming events from the LLM
#[derive(Debug, Clone)]
pub enum LlmEvent {
    /// Incremental text output
    TextDelta(String),
    /// Complete tool call (after accumulating streaming deltas)
    ToolUse {
        id: ToolUseId,
        name: String,
        input: Value,
        /// Opaque provider metadata (e.g. Gemini thought_signature) to round-trip.
        extra: Option<Value>,
    },
    /// Partial tool call progress while the provider is still streaming tool arguments.
    ToolUseDelta {
        id: ToolUseId,
        name: String,
        /// Small structured preview of arguments that are already known.
        input: Option<Value>,
    },
    /// A tool call the provider began streaming but truncated at its output
    /// ceiling. NEVER executable and never enters the engine's `tool_calls`
    /// vector. Emitted so a resumable round can tell the next attempt which
    /// call was cut off and how far it got, instead of discarding the
    /// accumulator in silence. `id` matches the `ToolUseDelta` already emitted
    /// for the same call, so a sink can settle the tool card it opened.
    ToolUseTruncated {
        id: ToolUseId,
        name: String,
        /// Size of the argument payload the ceiling cut off, for the recovery
        /// prompt's "how big was the thing you were writing" hint.
        ///
        /// Not a single wire quantity: for providers that stream arguments as
        /// text fragments this is the bytes actually streamed, while for
        /// providers that deliver a complete argument object and only then
        /// report the ceiling it is the serialized length of that object. Both
        /// answer the question the next attempt needs — the order of magnitude
        /// of the payload — so neither is normalized into the other.
        argument_bytes: usize,
    },
    /// Thinking content (Anthropic only)
    ThinkingDelta(String),
    /// Opaque provider signature for the current thinking block.
    ThinkingSignature(String),
    /// Opaque identity for this provider round. Emitted immediately before
    /// `Done`, and only when the provider has proved that the completed round
    /// is a legal parent for a later request.
    ProviderRoundId(String),
    /// Response complete
    Done {
        stop_reason: StopReason,
        usage: TokenUsage,
    },
    /// Error from the API
    Error(String),
}
