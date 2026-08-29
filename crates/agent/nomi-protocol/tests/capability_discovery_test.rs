use nomi_protocol::events::{Capabilities, ProtocolEvent};

#[test]
fn capabilities_serialize_with_all_fields() {
    let caps = Capabilities {
        thinking: true,
        effort: false,
        effort_levels: vec![],
        mcp: true,
    };
    let event = ProtocolEvent::Ready {
        version: "0.2.0".into(),
        session_id: None,
        capabilities: caps,
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["type"], "ready");
    assert_eq!(parsed["capabilities"]["thinking"], true);
    assert_eq!(parsed["capabilities"]["effort"], false);
    assert!(
        parsed["capabilities"]["effort_levels"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(parsed["capabilities"].as_object().unwrap().len(), 4);
}

#[test]
fn config_changed_event_serializes_correctly() {
    let caps = Capabilities {
        thinking: false,
        effort: true,
        effort_levels: vec!["low".into(), "medium".into(), "high".into()],
        mcp: false,
    };
    let event = ProtocolEvent::ConfigChanged { capabilities: caps };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["type"], "config_changed");
    assert_eq!(parsed["capabilities"]["effort_levels"][1], "medium");
}

#[test]
fn capabilities_with_effort_levels_roundtrip() {
    let caps = Capabilities {
        thinking: false,
        effort: true,
        effort_levels: vec!["low".into(), "medium".into(), "high".into()],
        mcp: true,
    };
    let event = ProtocolEvent::Ready {
        version: "0.2.0".into(),
        session_id: Some("test-session".into()),
        capabilities: caps,
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["capabilities"]["effort"], true);
    assert_eq!(
        parsed["capabilities"]["effort_levels"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(parsed["capabilities"]["effort_levels"][0], "low");
    assert_eq!(parsed["capabilities"]["effort_levels"][2], "high");
    assert_eq!(parsed["session_id"], "test-session");
}
