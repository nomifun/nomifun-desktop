//! xiaozhi wire protocol: JSON message vocabulary and binary frame codec.

pub mod messages;

pub use messages::{
    DOWNLINK_SAMPLE_RATE, DeviceHello, DeviceMessage, FRAME_DURATION_MS, ListenState,
    ListeningMode, ProtocolError, ServerMessage, UPLINK_SAMPLE_RATE, parse_device_message,
    serialize_server_message,
};
