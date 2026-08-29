use nomi_protocol::commands::ProtocolCommand;
use rstest::rstest;

#[rstest]
#[case(
    r#"{"type":"message","msg_id":"m1","content":"Hello"}"#,
    ProtocolCommand::Message {
        msg_id: "m1".to_string(),
        content: "Hello".to_string(),
    }
)]
#[case(
    // Wire-compatibility regression: unknown keys (like the removed `files`)
    // must be tolerated, not rejected.
    r#"{"type":"message","msg_id":"m2","content":"Read this","files":["/tmp/a.rs"]}"#,
    ProtocolCommand::Message {
        msg_id: "m2".to_string(),
        content: "Read this".to_string(),
    }
)]
#[case(r#"{"type":"stop"}"#, ProtocolCommand::Stop)]
#[case(
    r#"{"type":"init_history","text":"history"}"#,
    ProtocolCommand::InitHistory {
        text: "history".to_string(),
    }
)]
fn deserializes_protocol_commands(#[case] json: &str, #[case] expected: ProtocolCommand) {
    let cmd: ProtocolCommand = serde_json::from_str(json).expect("command should deserialize");
    assert_eq!(cmd, expected);
}
