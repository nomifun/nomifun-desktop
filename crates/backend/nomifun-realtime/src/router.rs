use crate::types::ConnectionId;

/// Routes upstream WebSocket messages to business logic handlers.
///
/// The `name` field of the incoming `WebSocketMessage` determines
/// which handler processes the message. Phase 4 provides only a
/// no-op implementation; concrete routing is added in later phases.
pub trait MessageRouter: Send + Sync {
    /// Route an upstream message to the appropriate handler.
    ///
    /// Called for any message whose `name` is not handled internally
    /// by the WebSocket layer (i.e. not `pong` or `subscribe-show-open`).
    fn route(&self, conn_id: ConnectionId, name: &str, data: serde_json::Value);
}

/// A no-op message router that silently discards all messages.
///
/// Used as a placeholder until business modules provide real routing.
pub struct NoopMessageRouter;

impl MessageRouter for NoopMessageRouter {
    fn route(&self, conn_id: ConnectionId, name: &str, _data: serde_json::Value) {
        tracing::debug!(
            %conn_id,
            message_name = name,
            "no router registered, message discarded"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn noop_router_does_not_panic() {
        let router = NoopMessageRouter;
        router.route(ConnectionId(1), "some-event", json!({"key": "val"}));
    }

    #[test]
    fn noop_router_is_trait_object_compatible() {
        let router: Box<dyn MessageRouter> = Box::new(NoopMessageRouter);
        router.route(ConnectionId(42), "test", json!(null));
    }
}
