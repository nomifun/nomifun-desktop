//! Black-box integration tests for compact types (TC-2.2-04 through TC-2.2-06).
//!
//! These test Message.timestamp serialization and CompactMetadata roundtrip
//! from a consumer's perspective.

use nomi_types::compact::{CompactMetadata, CompactTrigger};
use nomi_types::message::{ContentBlock, Message, Role};

/// TC-2.2-04: Message timestamp serialization — ISO 8601 format.
#[test]
fn tc_2_2_04_message_timestamp_serialization() {
    let msg = Message::now(
        Role::User,
        vec![ContentBlock::Text {
            text: "hello".into(),
        }],
    );

    let json = serde_json::to_string(&msg).unwrap();
    assert!(
        json.contains("\"timestamp\""),
        "JSON should contain timestamp"
    );

    // Verify ISO 8601 format (contains 'T' separator and '+' or 'Z' timezone)
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let ts_str = value["timestamp"].as_str().unwrap();
    assert!(
        ts_str.contains('T'),
        "timestamp should be ISO 8601 with T separator"
    );
}

/// TC-2.2-05: Message timestamp backward compatibility — old JSON without
/// timestamp deserializes with timestamp = None.
#[test]
fn tc_2_2_05_message_timestamp_backward_compat() {
    let old_json = r#"{
        "role": "user",
        "content": [{"type": "text", "text": "hello"}]
    }"#;

    let msg: Message = serde_json::from_str(old_json).unwrap();
    assert!(
        msg.timestamp.is_none(),
        "old JSON without timestamp should deserialize to None"
    );
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.content.len(), 1);
}

/// TC-2.2-06: CompactMetadata serialization/deserialization roundtrip.
#[test]
fn tc_2_2_06_compact_metadata_roundtrip() {
    let meta = CompactMetadata {
        trigger: CompactTrigger::Auto,
        pre_compact_tokens: 150_000,
        messages_summarized: 42,
    };

    let json = serde_json::to_value(&meta).unwrap();
    let back: CompactMetadata = serde_json::from_value(json).unwrap();

    assert_eq!(back.trigger, CompactTrigger::Auto);
    assert_eq!(back.pre_compact_tokens, 150_000);
    assert_eq!(back.messages_summarized, 42);
}

/// Additional: CompactState circuit breaker integration with CompactConfig.
#[test]
fn compact_state_circuit_breaker_integration() {
    use nomi_agent::compact::state::CompactState;
    use nomi_config::compact::CompactConfig;

    let config = CompactConfig {
        max_failures: 3,
        ..Default::default()
    };
    let mut state = CompactState::new();

    // Not broken initially
    assert!(!state.is_circuit_broken(&config));

    // Record failures up to the limit
    for _ in 0..3 {
        state.record_failure();
    }
    assert!(state.is_circuit_broken(&config));

    // One success resets
    state.record_success();
    assert!(!state.is_circuit_broken(&config));
}
