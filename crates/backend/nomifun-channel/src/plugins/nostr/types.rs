//! Nostr relay message types and event structures.
//!
//! Relay communication is JSON-array based:
//! - Client → Relay: `["REQ", <sub_id>, <filter>]`, `["EVENT", <event>]`, `["CLOSE", <sub_id>]`
//! - Relay → Client: `["EVENT", <sub_id>, <event>]`, `["EOSE", <sub_id>]`, `["OK", ...]`, `["NOTICE", ...]`

use serde::{Deserialize, Serialize};

/// A raw event as received from a relay.
///
/// We deserialize the full event JSON for crypto verification (id + sig)
/// via the `nostr` crate, but also keep a lightweight struct for field access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

impl RawEvent {
    /// Check if the event has a `["p", <pubkey>]` tag matching the target.
    pub fn has_p_tag(&self, target_pk_hex: &str) -> bool {
        self.tags
            .iter()
            .any(|tag| tag.len() >= 2 && tag[0] == "p" && tag[1] == target_pk_hex)
    }

    /// Whether this event is a kind-4 (NIP-04 encrypted DM).
    pub fn is_dm(&self) -> bool {
        self.kind == 4
    }
}

/// Parsed relay message (relay → client).
#[derive(Debug)]
pub enum RelayMessage {
    /// `["EVENT", <subscription_id>, <event>]`
    Event {
        subscription_id: String,
        event: RawEvent,
    },
    /// `["EOSE", <subscription_id>]`
    EndOfStoredEvents {
        subscription_id: String,
    },
    /// `["OK", <event_id>, <success>, <message>]`
    Ok {
        event_id: String,
        success: bool,
        message: String,
    },
    /// `["NOTICE", <message>]`
    Notice {
        message: String,
    },
    /// Unrecognized message type.
    Unknown(String),
}

impl RelayMessage {
    /// Parse a raw JSON string from a relay into a typed message.
    pub fn parse(raw: &str) -> Option<Self> {
        let arr: Vec<serde_json::Value> = serde_json::from_str(raw).ok()?;
        if arr.is_empty() {
            return None;
        }

        let msg_type = arr[0].as_str()?;
        match msg_type {
            "EVENT" if arr.len() >= 3 => {
                let subscription_id = arr[1].as_str()?.to_owned();
                let event: RawEvent = serde_json::from_value(arr[2].clone()).ok()?;
                Some(Self::Event {
                    subscription_id,
                    event,
                })
            }
            "EOSE" if arr.len() >= 2 => {
                let subscription_id = arr[1].as_str()?.to_owned();
                Some(Self::EndOfStoredEvents { subscription_id })
            }
            "OK" if arr.len() >= 4 => {
                let event_id = arr[1].as_str()?.to_owned();
                let success = arr[2].as_bool()?;
                let message = arr[3].as_str().unwrap_or("").to_owned();
                Some(Self::Ok {
                    event_id,
                    success,
                    message,
                })
            }
            "NOTICE" if arr.len() >= 2 => {
                let message = arr[1].as_str()?.to_owned();
                Some(Self::Notice { message })
            }
            _ => Some(Self::Unknown(raw.to_owned())),
        }
    }
}

/// Build a REQ subscription message for kind-4 DMs addressed to the bot.
pub fn build_req_message(subscription_id: &str, bot_pubkey_hex: &str, since_unix: i64) -> String {
    serde_json::json!([
        "REQ",
        subscription_id,
        {
            "kinds": [4],
            "#p": [bot_pubkey_hex],
            "since": since_unix
        }
    ])
    .to_string()
}

/// Build an EVENT publish message wrapping a signed event JSON.
pub fn build_event_message(event_json: &str) -> Result<String, serde_json::Error> {
    let event_value: serde_json::Value = serde_json::from_str(event_json)?;
    Ok(serde_json::json!(["EVENT", event_value]).to_string())
}

/// Build a CLOSE message to unsubscribe.
pub fn build_close_message(subscription_id: &str) -> String {
    serde_json::json!(["CLOSE", subscription_id]).to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_event_message() {
        let raw = r#"["EVENT","sub1",{"id":"abc","pubkey":"def","created_at":1700000000,"kind":4,"tags":[["p","ghi"]],"content":"encrypted","sig":"jkl"}]"#;
        match RelayMessage::parse(raw) {
            Some(RelayMessage::Event {
                subscription_id,
                event,
            }) => {
                assert_eq!(subscription_id, "sub1");
                assert_eq!(event.id, "abc");
                assert_eq!(event.pubkey, "def");
                assert_eq!(event.kind, 4);
                assert!(event.is_dm());
                assert!(event.has_p_tag("ghi"));
                assert!(!event.has_p_tag("xyz"));
            }
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn parse_eose_message() {
        let raw = r#"["EOSE","sub1"]"#;
        match RelayMessage::parse(raw) {
            Some(RelayMessage::EndOfStoredEvents { subscription_id }) => {
                assert_eq!(subscription_id, "sub1");
            }
            other => panic!("expected EOSE, got {other:?}"),
        }
    }

    #[test]
    fn parse_ok_message() {
        let raw = r#"["OK","event123",true,""]"#;
        match RelayMessage::parse(raw) {
            Some(RelayMessage::Ok {
                event_id,
                success,
                message,
            }) => {
                assert_eq!(event_id, "event123");
                assert!(success);
                assert!(message.is_empty());
            }
            other => panic!("expected OK, got {other:?}"),
        }
    }

    #[test]
    fn parse_notice_message() {
        let raw = r#"["NOTICE","rate limited"]"#;
        match RelayMessage::parse(raw) {
            Some(RelayMessage::Notice { message }) => {
                assert_eq!(message, "rate limited");
            }
            other => panic!("expected NOTICE, got {other:?}"),
        }
    }

    #[test]
    fn parse_unknown_message() {
        let raw = r#"["AUTH","challenge"]"#;
        match RelayMessage::parse(raw) {
            Some(RelayMessage::Unknown(_)) => {}
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(RelayMessage::parse("not json").is_none());
        assert!(RelayMessage::parse("[]").is_none());
    }

    #[test]
    fn build_req_message_format() {
        let msg = build_req_message("sub1", "abc123", 1700000000);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed[0], "REQ");
        assert_eq!(parsed[1], "sub1");
        assert_eq!(parsed[2]["kinds"][0], 4);
        assert_eq!(parsed[2]["#p"][0], "abc123");
        assert_eq!(parsed[2]["since"], 1700000000);
    }

    #[test]
    fn build_event_message_format() {
        let event_json = r#"{"id":"abc","pubkey":"def"}"#;
        let msg = build_event_message(event_json).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed[0], "EVENT");
        assert_eq!(parsed[1]["id"], "abc");
    }

    #[test]
    fn build_close_message_format() {
        let msg = build_close_message("sub1");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed[0], "CLOSE");
        assert_eq!(parsed[1], "sub1");
    }

    #[test]
    fn raw_event_is_dm() {
        let event = RawEvent {
            id: "x".into(),
            pubkey: "y".into(),
            created_at: 0,
            kind: 4,
            tags: vec![],
            content: "enc".into(),
            sig: "s".into(),
        };
        assert!(event.is_dm());

        let non_dm = RawEvent {
            kind: 1,
            ..event.clone()
        };
        assert!(!non_dm.is_dm());
    }

    #[test]
    fn raw_event_has_p_tag() {
        let event = RawEvent {
            id: "x".into(),
            pubkey: "y".into(),
            created_at: 0,
            kind: 4,
            tags: vec![
                vec!["e".into(), "ref_id".into()],
                vec!["p".into(), "target_pk".into()],
            ],
            content: "enc".into(),
            sig: "s".into(),
        };
        assert!(event.has_p_tag("target_pk"));
        assert!(!event.has_p_tag("other_pk"));
    }
}
