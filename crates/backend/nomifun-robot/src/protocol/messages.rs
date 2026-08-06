//! The xiaozhi JSON message vocabulary.
//!
//! Inbound parsing is deliberately tolerant: the firmware may add message types
//! at any time, and an unknown `type` must not kill the session — it becomes
//! [`DeviceMessage::Unknown`]. Outbound messages always carry `session_id`
//! first, matching the firmware's own hand-built strings.

use serde::Deserialize;

/// Uplink audio is hardcoded in firmware and cannot be negotiated.
pub const UPLINK_SAMPLE_RATE: u32 = 16_000;
/// Downlink rate we declare in the server hello.
pub const DOWNLINK_SAMPLE_RATE: u32 = 24_000;
/// Opus frame duration used in both directions, in milliseconds.
pub const FRAME_DURATION_MS: u32 = 60;

/// Failure to understand an inbound text frame.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("malformed JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("message has no string `type` field")]
    MissingType,
    #[error("`listen` message has no valid `state`")]
    MissingListenState,
}

/// `listen.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenState {
    Start,
    Stop,
    Detect,
}

/// `listen.mode` — how the turn ends. `Auto`/`Realtime` mean the **server**
/// must decide when the user stopped talking; `Manual` means the device will
/// send `listen stop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListeningMode {
    Auto,
    Manual,
    Realtime,
}

/// Device hello payload (the parts we act on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceHello {
    pub version: u32,
    pub transport: String,
    pub mcp: bool,
    pub aec: bool,
}

/// A parsed inbound text frame.
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceMessage {
    Hello(DeviceHello),
    Listen {
        state: ListenState,
        mode: Option<ListeningMode>,
        text: Option<String>,
    },
    Abort {
        reason: Option<String>,
    },
    Mcp {
        payload: serde_json::Value,
    },
    Goodbye,
    Unknown {
        raw_type: String,
    },
}

#[derive(Deserialize)]
struct RawHello {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    transport: String,
    #[serde(default)]
    features: RawFeatures,
}

fn default_version() -> u32 {
    1
}

#[derive(Deserialize, Default)]
struct RawFeatures {
    #[serde(default)]
    mcp: bool,
    #[serde(default)]
    aec: bool,
}

/// Parse an inbound text frame. Unknown `type` values are surfaced as
/// [`DeviceMessage::Unknown`] rather than an error.
pub fn parse_device_message(raw: &str) -> Result<DeviceMessage, ProtocolError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let msg_type = value
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or(ProtocolError::MissingType)?;
    match msg_type {
        "hello" => {
            let raw_hello: RawHello = serde_json::from_value(value)?;
            Ok(DeviceMessage::Hello(DeviceHello {
                version: raw_hello.version,
                transport: raw_hello.transport,
                mcp: raw_hello.features.mcp,
                aec: raw_hello.features.aec,
            }))
        }
        "listen" => {
            let state = match value.get("state").and_then(|s| s.as_str()) {
                Some("start") => ListenState::Start,
                Some("stop") => ListenState::Stop,
                Some("detect") => ListenState::Detect,
                _ => return Err(ProtocolError::MissingListenState),
            };
            let mode = match value.get("mode").and_then(|m| m.as_str()) {
                Some("auto") => Some(ListeningMode::Auto),
                Some("manual") => Some(ListeningMode::Manual),
                Some("realtime") => Some(ListeningMode::Realtime),
                _ => None,
            };
            let text = value.get("text").and_then(|t| t.as_str()).map(str::to_owned);
            Ok(DeviceMessage::Listen { state, mode, text })
        }
        "abort" => Ok(DeviceMessage::Abort {
            reason: value
                .get("reason")
                .and_then(|r| r.as_str())
                .map(str::to_owned),
        }),
        "mcp" => Ok(DeviceMessage::Mcp {
            payload: value
                .get("payload")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        }),
        "goodbye" => Ok(DeviceMessage::Goodbye),
        other => Ok(DeviceMessage::Unknown {
            raw_type: other.to_owned(),
        }),
    }
}

/// An outbound text frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    Hello {
        session_id: String,
    },
    Stt {
        session_id: String,
        text: String,
    },
    Llm {
        session_id: String,
        emotion: String,
    },
    TtsStart {
        session_id: String,
    },
    TtsStop {
        session_id: String,
    },
    TtsSentence {
        session_id: String,
        text: String,
    },
    Mcp {
        session_id: String,
        payload: serde_json::Value,
    },
    Ping {
        session_id: String,
    },
}

/// Render an outbound frame. Never fails: every variant is plain JSON.
pub fn serialize_server_message(msg: &ServerMessage) -> String {
    use serde_json::json;
    let value = match msg {
        ServerMessage::Hello { session_id } => json!({
            "type": "hello",
            "transport": "websocket",
            "session_id": session_id,
            "audio_params": {
                "format": "opus",
                "sample_rate": DOWNLINK_SAMPLE_RATE,
                "channels": 1,
                "frame_duration": FRAME_DURATION_MS,
            },
        }),
        ServerMessage::Stt { session_id, text } => {
            json!({ "session_id": session_id, "type": "stt", "text": text })
        }
        ServerMessage::Llm {
            session_id,
            emotion,
        } => {
            json!({ "session_id": session_id, "type": "llm", "emotion": emotion })
        }
        ServerMessage::TtsStart { session_id } => {
            json!({ "session_id": session_id, "type": "tts", "state": "start" })
        }
        ServerMessage::TtsStop { session_id } => {
            json!({ "session_id": session_id, "type": "tts", "state": "stop" })
        }
        ServerMessage::TtsSentence { session_id, text } => {
            json!({ "session_id": session_id, "type": "tts", "state": "sentence_start", "text": text })
        }
        ServerMessage::Mcp {
            session_id,
            payload,
        } => {
            json!({ "session_id": session_id, "type": "mcp", "payload": payload })
        }
        ServerMessage::Ping { session_id } => {
            json!({ "session_id": session_id, "type": "ping" })
        }
    };
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_hello() {
        let raw = r#"{"type":"hello","version":1,"features":{"mcp":true},"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}"#;
        let DeviceMessage::Hello(hello) = parse_device_message(raw).unwrap() else {
            panic!("expected hello");
        };
        assert_eq!(hello.version, 1);
        assert_eq!(hello.transport, "websocket");
        assert!(hello.mcp);
        assert!(!hello.aec);
    }

    #[test]
    fn parses_listen_variants() {
        let start = parse_device_message(
            r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#,
        )
        .unwrap();
        assert!(matches!(
            start,
            DeviceMessage::Listen {
                state: ListenState::Start,
                mode: Some(ListeningMode::Auto),
                ..
            }
        ));
        let stop =
            parse_device_message(r#"{"session_id":"s","type":"listen","state":"stop"}"#).unwrap();
        assert!(matches!(
            stop,
            DeviceMessage::Listen {
                state: ListenState::Stop,
                mode: None,
                ..
            }
        ));
        let detect = parse_device_message(
            r#"{"session_id":"s","type":"listen","state":"detect","text":"你好小智"}"#,
        )
        .unwrap();
        let DeviceMessage::Listen {
            state: ListenState::Detect,
            text,
            ..
        } = detect
        else {
            panic!("expected detect");
        };
        assert_eq!(text.as_deref(), Some("你好小智"));
    }

    #[test]
    fn parses_abort_with_and_without_reason() {
        let with = parse_device_message(
            r#"{"session_id":"s","type":"abort","reason":"wake_word_detected"}"#,
        )
        .unwrap();
        assert!(matches!(with, DeviceMessage::Abort { reason: Some(ref r) } if r == "wake_word_detected"));
        let without = parse_device_message(r#"{"session_id":"s","type":"abort"}"#).unwrap();
        assert!(matches!(without, DeviceMessage::Abort { reason: None }));
    }

    #[test]
    fn unknown_type_is_tolerated_not_an_error() {
        let msg = parse_device_message(r#"{"type":"something_new","x":1}"#).unwrap();
        assert!(matches!(msg, DeviceMessage::Unknown { ref raw_type } if raw_type == "something_new"));
    }

    #[test]
    fn missing_type_is_an_error() {
        assert!(parse_device_message(r#"{"state":"start"}"#).is_err());
    }

    #[test]
    fn server_hello_declares_downlink_audio_params() {
        let json = serialize_server_message(&ServerMessage::Hello {
            session_id: "abc".into(),
        });
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "hello");
        assert_eq!(value["transport"], "websocket");
        assert_eq!(value["session_id"], "abc");
        assert_eq!(value["audio_params"]["format"], "opus");
        assert_eq!(value["audio_params"]["sample_rate"], 24000);
        assert_eq!(value["audio_params"]["channels"], 1);
        assert_eq!(value["audio_params"]["frame_duration"], 60);
    }

    #[test]
    fn tts_messages_carry_state_and_session() {
        let start: serde_json::Value = serde_json::from_str(&serialize_server_message(
            &ServerMessage::TtsStart {
                session_id: "s".into(),
            },
        ))
        .unwrap();
        assert_eq!(start["type"], "tts");
        assert_eq!(start["state"], "start");
        assert_eq!(start["session_id"], "s");

        let sentence: serde_json::Value = serde_json::from_str(&serialize_server_message(
            &ServerMessage::TtsSentence {
                session_id: "s".into(),
                text: "你好".into(),
            },
        ))
        .unwrap();
        assert_eq!(sentence["state"], "sentence_start");
        assert_eq!(sentence["text"], "你好");

        let stop: serde_json::Value = serde_json::from_str(&serialize_server_message(
            &ServerMessage::TtsStop {
                session_id: "s".into(),
            },
        ))
        .unwrap();
        assert_eq!(stop["state"], "stop");
    }

    #[test]
    fn llm_message_carries_emotion_only() {
        let value: serde_json::Value = serde_json::from_str(&serialize_server_message(
            &ServerMessage::Llm {
                session_id: "s".into(),
                emotion: "happy".into(),
            },
        ))
        .unwrap();
        assert_eq!(value["type"], "llm");
        assert_eq!(value["emotion"], "happy");
    }
}
