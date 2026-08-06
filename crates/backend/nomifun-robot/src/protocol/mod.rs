//! xiaozhi wire protocol: JSON message vocabulary and binary frame codec.

pub mod binary;
pub mod messages;

pub use binary::{decode_binary_v1, encode_binary_v1};
pub use messages::{
    DOWNLINK_SAMPLE_RATE, DeviceHello, DeviceMessage, FRAME_DURATION_MS, ListenState,
    ListeningMode, ProtocolError, ServerMessage, UPLINK_SAMPLE_RATE, parse_device_message,
    serialize_server_message,
};
