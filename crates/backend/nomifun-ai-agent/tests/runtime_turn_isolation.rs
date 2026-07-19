use std::time::Duration;

use nomifun_ai_agent::protocol::events::TextEventData;
use nomifun_ai_agent::{AgentRuntimeState, AgentStreamEvent, TurnStopReason};
use nomifun_common::ConversationStatus;

/// A reusable runtime must reject a detached terminal from the previous turn
/// after the next turn has already entered Running. This is the cross-backend
/// guard against an old cancel/provider callback poisoning every later prompt.
#[tokio::test]
async fn previous_turn_terminal_cannot_finish_the_next_turn() {
    let runtime = AgentRuntimeState::new("conv-turn-isolation", "/workspace", 8);

    let previous_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
    assert!(runtime.emit_finish_for_turn(
        previous_turn,
        None,
        Some(TurnStopReason::EndTurn),
    ));

    let current_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
    let mut receiver = runtime.subscribe();

    assert!(
        !runtime.emit_finish_for_turn(
            previous_turn,
            None,
            Some(TurnStopReason::Cancelled),
        ),
        "a stale terminal must be rejected by turn identity"
    );
    assert_eq!(runtime.status(), Some(ConversationStatus::Running));
    assert!(
        !runtime.emit_for_turn(
            previous_turn,
            AgentStreamEvent::Text(TextEventData {
                content: "late first-turn content".into(),
            }),
        ),
        "stale content/artifact projections must also be rejected"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), receiver.recv())
            .await
            .is_err(),
        "a stale terminal must not enter the current turn's event stream"
    );

    assert!(runtime.emit_finish_for_turn(
        current_turn,
        None,
        Some(TurnStopReason::EndTurn),
    ));
    let event = receiver.recv().await.expect("current turn terminal");
    assert!(matches!(
        event,
        AgentStreamEvent::Finish(data)
            if data.stop_reason == Some(TurnStopReason::EndTurn)
    ));
    assert_eq!(runtime.status(), Some(ConversationStatus::Finished));
}
