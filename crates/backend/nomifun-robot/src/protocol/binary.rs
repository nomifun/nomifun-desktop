//! Binary audio frame codec.
//!
//! The firmware picks the framing from the OTA-delivered `websocket.version`.
//! We always advertise **version 1**, which is a bare Opus payload with no
//! header at all — v2/v3 add headers this gateway deliberately does not
//! implement (see spec §1 non-goals). The identity functions exist so the call
//! sites read as framing decisions and a future v2 has one obvious home.

use bytes::Bytes;

/// Wrap an Opus packet for the wire. v1 = the packet itself.
pub fn encode_binary_v1(opus_packet: &[u8]) -> Bytes {
    Bytes::copy_from_slice(opus_packet)
}

/// Extract the Opus packet from a wire frame. v1 = the frame itself.
pub fn decode_binary_v1(frame: &[u8]) -> &[u8] {
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_adds_no_header_bytes() {
        let packet = [0xfc_u8, 0x01, 0x02, 0x03];
        let framed = encode_binary_v1(&packet);
        assert_eq!(
            framed.as_ref(),
            &packet,
            "protocol v1 is a bare Opus payload — any header breaks the firmware decoder"
        );
    }

    #[test]
    fn v1_decode_is_identity() {
        let frame = [0x11_u8, 0x22, 0x33];
        assert_eq!(decode_binary_v1(&frame), &frame);
    }

    #[test]
    fn empty_frame_round_trips() {
        assert!(encode_binary_v1(&[]).is_empty());
        assert!(decode_binary_v1(&[]).is_empty());
    }
}
