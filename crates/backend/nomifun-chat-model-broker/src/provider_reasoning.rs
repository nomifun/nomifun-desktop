//! Broker-owned exact-route binding of canonical continuation data.
use crate::ResolvedChatRoute;
use nomifun_agent_contracts::digest_payload;
pub use nomifun_agent_contracts::chat_provider_reasoning::*;

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
