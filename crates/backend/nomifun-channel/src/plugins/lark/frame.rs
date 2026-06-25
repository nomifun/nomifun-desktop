use prost::Message;

pub const METHOD_CONTROL: i32 = 0;
pub const METHOD_DATA: i32 = 1;

#[derive(Clone, PartialEq, Message)]
pub struct PbHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct PbFrame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<PbHeader>,
    #[prost(string, tag = "6")]
    pub payload_encoding: String,
    #[prost(string, tag = "7")]
    pub payload_type: String,
    #[prost(bytes = "vec", tag = "8")]
    pub payload: Vec<u8>,
    #[prost(string, tag = "9")]
    pub log_id_new: String,
}

pub fn decode_frame(data: &[u8]) -> Result<PbFrame, prost::DecodeError> {
    PbFrame::decode(data)
}

pub fn encode_frame(frame: &PbFrame) -> Vec<u8> {
    frame.encode_to_vec()
}

pub fn get_header<'a>(headers: &'a [PbHeader], key: &str) -> Option<&'a str> {
    headers.iter().find(|h| h.key == key).map(|h| h.value.as_str())
}

pub fn build_ping_frame(service_id: i32) -> PbFrame {
    PbFrame {
        seq_id: 0,
        log_id: 0,
        service: service_id,
        method: METHOD_CONTROL,
        headers: vec![PbHeader {
            key: "type".into(),
            value: "ping".into(),
        }],
        payload_encoding: String::new(),
        payload_type: String::new(),
        payload: Vec::new(),
        log_id_new: String::new(),
    }
}

pub fn build_ack_frame(original: &PbFrame) -> PbFrame {
    let mut ack_headers = Vec::new();
    for h in &original.headers {
        if h.key == "type" || h.key == "message_id" || h.key == "trace_id" {
            ack_headers.push(h.clone());
        }
    }
    ack_headers.push(PbHeader {
        key: "biz_rt".into(),
        value: "0".into(),
    });

    PbFrame {
        seq_id: original.seq_id,
        log_id: original.log_id,
        service: original.service,
        method: METHOD_DATA,
        headers: ack_headers,
        payload_encoding: String::new(),
        payload_type: String::new(),
        payload: br#"{"code":200}"#.to_vec(),
        log_id_new: original.log_id_new.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_ping_frame() {
        let frame = PbFrame {
            seq_id: 0,
            log_id: 0,
            service: 1,
            method: METHOD_CONTROL,
            headers: vec![PbHeader {
                key: "type".into(),
                value: "ping".into(),
            }],
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload: Vec::new(),
            log_id_new: String::new(),
        };
        let bytes = encode_frame(&frame);
        let decoded = decode_frame(&bytes).unwrap();
        assert_eq!(decoded.method, METHOD_CONTROL);
        assert_eq!(decoded.headers[0].key, "type");
        assert_eq!(decoded.headers[0].value, "ping");
    }

    #[test]
    fn round_trip_data_frame_with_payload() {
        let payload = br#"{"sender":{},"message":{}}"#;
        let frame = PbFrame {
            seq_id: 42,
            log_id: 100,
            service: 1,
            method: METHOD_DATA,
            headers: vec![
                PbHeader {
                    key: "type".into(),
                    value: "event".into(),
                },
                PbHeader {
                    key: "message_id".into(),
                    value: "msg_001".into(),
                },
                PbHeader {
                    key: "sum".into(),
                    value: "1".into(),
                },
                PbHeader {
                    key: "seq".into(),
                    value: "0".into(),
                },
            ],
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload: payload.to_vec(),
            log_id_new: String::new(),
        };
        let bytes = encode_frame(&frame);
        let decoded = decode_frame(&bytes).unwrap();
        assert_eq!(decoded.seq_id, 42);
        assert_eq!(decoded.method, METHOD_DATA);
        assert_eq!(decoded.payload, payload);
        assert_eq!(decoded.headers.len(), 4);
    }

    #[test]
    fn decode_invalid_bytes_returns_error() {
        let result = decode_frame(&[0xFF, 0xFF, 0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn build_ping_frame_has_correct_structure() {
        let frame = build_ping_frame(7);
        assert_eq!(frame.method, METHOD_CONTROL);
        assert_eq!(frame.service, 7);
        let type_header = frame.headers.iter().find(|h| h.key == "type").unwrap();
        assert_eq!(type_header.value, "ping");
    }

    #[test]
    fn build_ack_frame_has_code_200_payload() {
        let original = PbFrame {
            seq_id: 10,
            log_id: 20,
            service: 1,
            method: METHOD_DATA,
            headers: vec![
                PbHeader {
                    key: "type".into(),
                    value: "event".into(),
                },
                PbHeader {
                    key: "message_id".into(),
                    value: "msg_x".into(),
                },
            ],
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload: Vec::new(),
            log_id_new: String::new(),
        };
        let ack = build_ack_frame(&original);
        assert_eq!(ack.method, METHOD_DATA);
        let payload_str = String::from_utf8(ack.payload.clone()).unwrap();
        assert!(payload_str.contains("200"));
    }

    #[test]
    fn get_header_finds_existing_key() {
        let headers = vec![
            PbHeader {
                key: "type".into(),
                value: "event".into(),
            },
            PbHeader {
                key: "sum".into(),
                value: "3".into(),
            },
        ];
        assert_eq!(get_header(&headers, "type"), Some("event"));
        assert_eq!(get_header(&headers, "sum"), Some("3"));
        assert_eq!(get_header(&headers, "missing"), None);
    }

    #[test]
    fn build_ack_frame_retains_only_allowed_headers() {
        let original = PbFrame {
            seq_id: 5,
            log_id: 10,
            service: 1,
            method: METHOD_DATA,
            headers: vec![
                PbHeader {
                    key: "type".into(),
                    value: "event".into(),
                },
                PbHeader {
                    key: "message_id".into(),
                    value: "msg_99".into(),
                },
                PbHeader {
                    key: "trace_id".into(),
                    value: "trace_abc".into(),
                },
                PbHeader {
                    key: "sum".into(),
                    value: "3".into(),
                },
                PbHeader {
                    key: "seq".into(),
                    value: "1".into(),
                },
            ],
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload: Vec::new(),
            log_id_new: String::new(),
        };
        let ack = build_ack_frame(&original);
        let keys: Vec<&str> = ack.headers.iter().map(|h| h.key.as_str()).collect();
        assert!(keys.contains(&"type"));
        assert!(keys.contains(&"message_id"));
        assert!(keys.contains(&"trace_id"));
        assert!(keys.contains(&"biz_rt"));
        assert!(!keys.contains(&"sum"));
        assert!(!keys.contains(&"seq"));
    }

    #[test]
    fn build_ack_frame_echoes_seq_and_log_ids() {
        let original = PbFrame {
            seq_id: 77,
            log_id: 88,
            service: 3,
            method: METHOD_DATA,
            headers: vec![],
            payload_encoding: String::new(),
            payload_type: String::new(),
            payload: Vec::new(),
            log_id_new: "new_log_xyz".into(),
        };
        let ack = build_ack_frame(&original);
        assert_eq!(ack.seq_id, 77);
        assert_eq!(ack.log_id, 88);
        assert_eq!(ack.service, 3);
        assert_eq!(ack.log_id_new, "new_log_xyz");
    }
}
