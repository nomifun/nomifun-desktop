//! WebSocket connection manager, event broadcasting, and token-validated upgrade handler.
pub mod broadcaster;
pub mod handler;
pub mod manager;
pub mod types;

pub use broadcaster::{BroadcastEventBus, EventBroadcaster, UserEventEnvelope, UserEventSink};
pub use handler::{TokenExtractor, WsHandlerState, parse_allowed_origins, ws_upgrade_handler};
pub use manager::{TokenAuthenticator, WebSocketManager};
pub use types::{
    ClientInfo, ConnectionId, HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, PER_CONNECTION_BUFFER, WebSocketCloseCode,
    WsOutbound,
};
