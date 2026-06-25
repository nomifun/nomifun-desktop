//! WebSocket connection manager, event broadcasting, and token-validated upgrade handler.
pub mod broadcaster;
pub mod handler;
pub mod manager;
pub mod router;
pub mod types;

pub use broadcaster::{BroadcastEventBus, EventBroadcaster};
pub use handler::{TokenExtractor, WsHandlerState, ws_upgrade_handler};
pub use manager::{TokenValidator, WebSocketManager};
pub use router::{MessageRouter, NoopMessageRouter};
pub use types::{
    ClientInfo, ConnectionId, HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, PER_CONNECTION_BUFFER, WebSocketCloseCode,
    WsOutbound,
};
