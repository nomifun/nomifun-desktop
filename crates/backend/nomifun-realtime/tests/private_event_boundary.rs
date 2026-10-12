/// User-owned producers are a closed architectural set. Keep this guard close
/// to the two realtime audience traits so a private domain cannot silently
/// regress to the instance-wide sink during later refactors.
#[test]
fn user_owned_producers_cannot_depend_on_instance_event_broadcaster() {
    let producers = [
        ("channel/manager", include_str!("../../nomifun-channel/src/manager.rs")),
        ("channel/pairing", include_str!("../../nomifun-channel/src/pairing.rs")),
        ("companion/events", include_str!("../../nomifun-companion/src/events.rs")),
        ("cron/events", include_str!("../../nomifun-cron/src/events.rs")),
        ("file/service", include_str!("../../nomifun-file/src/service.rs")),
        ("file/watch_service", include_str!("../../nomifun-file/src/watch_service.rs")),
        ("ssh/events", include_str!("../../nomifun-ssh/src/events.rs")),
        ("terminal/events", include_str!("../../nomifun-terminal/src/events.rs")),
    ];

    for (name, source) in producers {
        assert!(
            source.contains("UserEventSink"),
            "{name} must retain the owner-scoped realtime boundary"
        );
        assert!(
            !source.contains("EventBroadcaster"),
            "{name} must not depend on the instance-wide event boundary"
        );
        assert!(
            !source.contains("WebSocketManager"),
            "{name} must not bypass the shared owner-scoped event bus"
        );
    }

    // Canonical cron execution no longer emits realtime events itself. Its
    // event emitter above owns the user boundary; keep execution from acquiring
    // either broad audience dependency during future changes.
    let executor = include_str!("../../nomifun-cron/src/executor.rs");
    assert!(!executor.contains("EventBroadcaster"), "cron execution must not emit instance-wide events");
    assert!(!executor.contains("WebSocketManager"), "cron execution must not bypass the owner event emitter");
}
