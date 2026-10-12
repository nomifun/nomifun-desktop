//! Broker-owned exact-route binding of canonical continuation data.
use crate::{ChatContentPart, ChatModelError, ChatModelErrorCode, ChatModelInput, ChatProtocol,
    ChatRetryDirective, ChatToolCall, ResolvedChatRoute};
use nomifun_agent_contracts::{StrictJsonValue, digest_payload};
pub use nomifun_agent_contracts::chat_provider_reasoning::*;

pub(crate) fn validate_reasoning_signature_origins(input: &ChatModelInput) -> Result<(), ChatModelError> {
    if input.messages.iter().flat_map(|message| &message.content).any(|part|
        matches!(part, ChatContentPart::Reasoning { signature: Some(_), .. })) {
        // Generic reasoning has no producing-route evidence. Preserve the
        // explicit typed continuation path instead of guessing its origin.
        return Err(ChatModelError::new(
            ChatModelErrorCode::UnsupportedFeature,
            "generic reasoning signature has no verifiable producing route",
            ChatRetryDirective::Never,
        ));
    }
    Ok(())
}

pub(crate) const GEMINI_TOOL_ROUTE_ORIGIN: &str = "_nomifun_producing_route_digest";

pub(crate) fn gemini_tool_signature(
    metadata: &Option<StrictJsonValue>,
) -> Result<Option<&str>, ChatModelError> {
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let metadata = metadata.0.as_object().ok_or_else(|| {
        ChatModelError::invalid_request("Gemini tool provider metadata must be an object")
    })?;
    let mut signature = None;
    for field in ["thoughtSignature", "thought_signature"] {
        if let Some(value) = metadata.get(field) {
            let value = value.as_str().filter(|value| !value.is_empty()).ok_or_else(|| {
                ChatModelError::invalid_request(format!(
                    "Gemini tool metadata field {field} must be a non-empty string"
                ))
            })?;
            if signature.is_some_and(|existing| existing != value) {
                return Err(ChatModelError::invalid_request(
                    "Gemini tool metadata contains conflicting thought signatures",
                ));
            }
            signature = Some(value);
        }
    }
    Ok(signature)
}

pub(crate) fn bind_gemini_tool_route(
    call: &mut ChatToolCall,
    route: &ResolvedChatRoute,
) -> Result<(), ChatModelError> {
    let object = call.provider_metadata.as_ref().and_then(|value| value.0.as_object());
    if object.is_some_and(|object| object.contains_key(GEMINI_TOOL_ROUTE_ORIGIN)) {
        return Err(ChatModelError::protocol_violation(
            "provider decoder supplied an unexpected tool signature origin",
        ));
    }
    if route.protocol != ChatProtocol::Gemini {
        if object.is_some_and(|object| {
            object.contains_key("thoughtSignature") || object.contains_key("thought_signature")
        }) {
            return Err(ChatModelError::protocol_violation(
                "Gemini tool signature is incompatible with the producing protocol",
            ));
        }
        return Ok(());
    }
    if gemini_tool_signature(&call.provider_metadata)?.is_none() {
        return Ok(());
    }
    let metadata = call.provider_metadata.as_mut().and_then(|value| value.0.as_object_mut())
        .ok_or_else(|| ChatModelError::protocol_violation("Gemini signed tool metadata is not an object"))?;
    let digest = digest_payload(route).map_err(|_| {
        ChatModelError::protocol_violation("cannot bind Gemini tool signature route")
    })?;
    metadata.insert(GEMINI_TOOL_ROUTE_ORIGIN.into(), serde_json::json!(digest));
    Ok(())
}

pub(crate) fn gemini_tool_matches_route(
    metadata: &Option<StrictJsonValue>,
    route: &ResolvedChatRoute,
) -> Result<bool, ChatModelError> {
    let object = metadata.as_ref().and_then(|value| value.0.as_object());
    let has_signature = object.is_some_and(|object| {
        object.contains_key("thoughtSignature") || object.contains_key("thought_signature")
    });
    if !has_signature {
        // Other protocols can carry their own opaque metadata. Gemini retains
        // its object-shape validation without changing unsigned calls.
        if route.protocol == ChatProtocol::Gemini {
            gemini_tool_signature(metadata)?;
        }
        return Ok(true);
    }
    gemini_tool_signature(metadata)?;
    Ok(route.protocol == ChatProtocol::Gemini && object
        .and_then(|object| object.get(GEMINI_TOOL_ROUTE_ORIGIN))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|origin| digest_payload(route).is_ok_and(|digest| digest.as_ref() == origin)))
}

pub(crate) trait ProviderReasoningRoute {
    fn bind_route(&mut self, route: &ResolvedChatRoute) -> Result<(), &'static str>;
    fn matches_route(&self, route: &ResolvedChatRoute) -> bool;
}

impl ProviderReasoningRoute for ChatProviderReasoning {
    /// Bind only at the Broker boundary, using the actual producing attempt,
    /// including failover identity, model, protocol and configuration revision.
    /// Decoder or remote-supplied origins are never accepted as authority.
    fn bind_route(&mut self, route: &ResolvedChatRoute) -> Result<(), &'static str> {
        self.validate()?;
        if !route.protocol.uses_anthropic_messages() || route.validate().is_err() {
            return Err("provider reasoning is incompatible with the attempted route");
        }
        let digest = digest_payload(route).map_err(|_| "cannot bind provider reasoning route")?;
        let origin = match self {
            Self::AnthropicThinking { route_digest, .. }
            | Self::AnthropicRedactedThinking { route_digest, .. } => route_digest,
        };
        if origin.is_some() {
            return Err("provider decoder supplied an unexpected reasoning origin");
        }
        *origin = Some(digest);
        Ok(())
    }

    /// Unbound historical state must not be guessed or silently stripped. The
    /// Engine may explicitly rebuild context without it; no transparent failover
    /// can send a signature to another route or turn it into ordinary text.
    fn matches_route(&self, route: &ResolvedChatRoute) -> bool {
        let origin = match self {
            Self::AnthropicThinking { route_digest, .. }
            | Self::AnthropicRedactedThinking { route_digest, .. } => route_digest,
        };
        route.protocol.uses_anthropic_messages()
            && origin
                .as_ref()
                .is_some_and(|origin| digest_payload(route).is_ok_and(|digest| digest == *origin))
    }
}
