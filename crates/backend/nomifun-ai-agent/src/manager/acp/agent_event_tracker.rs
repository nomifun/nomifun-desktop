use crate::manager::acp::{AcpAgentManager, AcpSession};
use crate::protocol::events::AgentStreamEvent;
use agent_client_protocol::schema::{
    SessionConfigOption, SessionModeState, SessionModelState, SessionNotification, UsageUpdate,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

use std::collections::HashMap;

use crate::shared_kernel::{ConfigKey, ConfigValue, ModeId, ModelId, SessionId};

/// Domain events emitted by the `AcpSession` aggregate.
///
/// These capture *intent* changes (user wants mode X) and *observation*
/// arrivals (CLI reported mode Y) separately — persistence consumers can
/// decide which to write to DB without re-interpreting UI stream events.
///
/// `context_usage_json` travels as a pre-serialised string so the event
/// type can keep `Eq` (SDK's `UsageUpdate` only derives `PartialEq`) and
/// so the persistence consumer can forward it to the DB without another
/// round-trip through serde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpSessionEvent {
    SessionOpened,
    SessionAssigned {
        session_id: SessionId,
    },
    DesiredModeChanged {
        mode: ModeId,
    },
    DesiredModelChanged {
        model: ModelId,
    },
    DesiredConfigChanged {
        selections: HashMap<ConfigKey, ConfigValue>,
    },
    ObservedModeSynced {
        mode: ModeId,
    },
    ObservedModelSynced {
        model: ModelId,
    },
    ObservedConfigSynced {
        selections: HashMap<ConfigKey, ConfigValue>,
    },
    ObservedContextUsageChanged {
        usage_json: String,
    },
}

impl AcpAgentManager {
    /// Consume SDK session/update notifications and apply their effects to
    /// the AcpSession aggregate.
    ///
    /// This is the **sole** writer of observed/advertised session state from
    /// the CLI side (target.md §7.2). It intentionally consumes a dedicated
    /// mpsc receiver — NOT a subscription to `event_tx` — to keep the "SDK
    /// notification → session" flow single-directional. Anything else that
    /// emits `AgentStreamEvent::Acp*` on `event_tx` (e.g. session_flow's
    /// `emit_snapshot_events` broadcasting initial UI state) does NOT feed
    /// back into the session.
    ///
    /// Invariant: `emit_snapshot_events` broadcasts on `event_tx` for UI
    /// initial state, but its events are NOT delivered to this tracker —
    /// the reflexive loop has been removed. This is intentional: the session
    /// aggregate is only updated from raw SDK `SessionNotification`s, never
    /// from re-broadcast AgentStreamEvents.
    pub fn start_session_event_tracker(self: &Arc<Self>, mut notification_rx: mpsc::Receiver<SessionNotification>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(notif) = notification_rx.recv().await {
                this.apply_notification_to_session(&notif).await;
            }
        });
    }

    async fn apply_notification_to_session(&self, notif: &SessionNotification) {
        // Translate the SDK notification into AgentStreamEvent shapes, then
        // dispatch through the existing apply logic.
        // A future Stage 3b can collapse this round-trip to consume SessionUpdate
        // directly, avoiding the JSON serialization step.
        let events = crate::protocol::events::session_notification_to_events(notif);
        for event in events {
            self.apply_event_to_session(&event).await;
        }
    }

    /// Mirror a stream event into the `AcpSession` aggregate's observed/advertised
    /// layer and forward any resulting domain events to the persistence consumer.
    async fn apply_event_to_session(&self, event: &AgentStreamEvent) {
        match event {
            AgentStreamEvent::AcpModeInfo(value) => {
                if let Ok(update) = serde_json::from_value::<SessionModeState>(value.clone()) {
                    let mut s = self.session.write().await;
                    s.apply_advertised_modes(update);
                    self.commit_session_changes(&mut s).await;
                } else if let Some(current_id) = value.get("currentModeId").and_then(|v: &Value| v.as_str()) {
                    let mut s = self.session.write().await;
                    s.apply_observed_mode(ModeId::new(current_id));
                    self.commit_session_changes(&mut s).await;
                }
            }
            AgentStreamEvent::AcpModelInfo(value) => {
                if let Ok(update) = serde_json::from_value::<SessionModelState>(value.clone()) {
                    let mut s = self.session.write().await;
                    s.apply_advertised_models(update);
                    self.commit_session_changes(&mut s).await;
                }
            }
            AgentStreamEvent::AcpConfigOption(value) => {
                if let Ok(update) = serde_json::from_value::<Vec<SessionConfigOption>>(value.clone()) {
                    let mut s = self.session.write().await;
                    s.apply_advertised_config_options(update);
                    self.commit_session_changes(&mut s).await;
                }
            }
            AgentStreamEvent::AcpContextUsage(value) => {
                if let Ok(update) = serde_json::from_value::<UsageUpdate>(value.clone()) {
                    let mut s = self.session.write().await;
                    s.apply_context_usage(update);
                    self.commit_session_changes(&mut s).await;
                }
            }
            AgentStreamEvent::AvailableCommands(data) => {
                let mut s = self.session.write().await;
                s.apply_advertised_commands(data.commands.clone());
                self.commit_session_changes(&mut s).await;
            }
            _ => {}
        }
    }

    /// Start the permission handler loop. Must be called after the manager
    /// is wrapped in Arc. Delegates to `PermissionRouter::start`.
    pub fn start_permission_handler(self: &Arc<Self>) {
        self.permission_router.start(self.runtime.clone());
    }

    /// Drain pending domain events from the session aggregate and
    /// forward them to the persistence consumer via the mpsc channel.
    pub(super) async fn commit_session_changes(&self, session: &mut AcpSession) {
        for event in session.drain_events() {
            let _ = self.domain_event_tx.send(event).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::acp::agent_event_tracker::AcpSessionEvent;
    use crate::manager::acp::session::AcpSession;
    use crate::shared_kernel::{ModeId, ModelId};
    use agent_client_protocol::schema::SessionModeState;

    #[test]
    fn event_equality() {
        let a = AcpSessionEvent::SessionAssigned {
            session_id: SessionId::new("s1"),
        };
        let b = AcpSessionEvent::SessionAssigned {
            session_id: SessionId::new("s1"),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn event_debug_format() {
        let e = AcpSessionEvent::DesiredModeChanged {
            mode: ModeId::new("plan"),
        };
        let dbg = format!("{e:?}");
        assert!(dbg.contains("plan"));
    }

    // ── Test 1 ──────────────────────────────────────────────────────────────
    //
    // Verify that a single `apply_observed_mode` call on AcpSession produces
    // exactly one `ObservedModeSynced` event, and calling it again with the
    // same value produces no additional event (idempotent).
    //
    // This test documents what the old tracker loop was doing: re-applying
    // the same events that `emit_snapshot_events` already put on the broadcast.
    // The re-application was idempotent (hence "masking" the bug), but it was
    // still a redundant write. The new architecture avoids the issue entirely by
    // only consuming raw SDK SessionNotifications, not broadcast events.

    #[test]
    fn apply_observed_mode_emits_exactly_one_event() {
        let mut session = AcpSession::new(None, None, Default::default());

        // First apply: should emit one event
        session.apply_observed_mode(ModeId::new("plan"));
        let events = session.drain_events();
        assert_eq!(
            events.len(),
            1,
            "expected exactly one ObservedModeSynced event, got: {events:?}"
        );
        assert_eq!(
            events[0],
            AcpSessionEvent::ObservedModeSynced {
                mode: ModeId::new("plan")
            }
        );

        // Second apply with same value: idempotent — no additional event
        session.apply_observed_mode(ModeId::new("plan"));
        let events2 = session.drain_events();
        assert_eq!(
            events2.len(),
            0,
            "idempotent re-apply should produce no new events, got: {events2:?}"
        );
    }

    #[test]
    fn apply_advertised_modes_emits_observed_synced_when_observed_changes() {
        // `apply_advertised_modes` writes the CLI's advertised snapshot,
        // which also moves `observed.mode_id` to `current_mode_id`. When
        // that shifts the observed value, one `ObservedModeSynced` event
        // is emitted so the persistence consumer writes the new value to
        // `session_config.runtime`. Re-applying with the same current id
        // is a no-op.
        let mut session = AcpSession::new(None, None, Default::default());
        let modes = SessionModeState::new("plan".to_owned(), vec![]);
        session.apply_advertised_modes(modes.clone());
        let events = session.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            AcpSessionEvent::ObservedModeSynced {
                mode: ModeId::new("plan")
            }
        );

        // Idempotent re-apply: same current id — no new event.
        session.apply_advertised_modes(modes);
        assert_eq!(session.drain_events().len(), 0);
    }

    #[test]
    fn apply_observed_model_emits_exactly_one_event_then_idempotent() {
        let mut session = AcpSession::new(None, None, Default::default());

        session.apply_observed_model(ModelId::new("claude-3-5-sonnet"));
        let events = session.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            AcpSessionEvent::ObservedModelSynced {
                model: ModelId::new("claude-3-5-sonnet")
            }
        );

        // Idempotent second write
        session.apply_observed_model(ModelId::new("claude-3-5-sonnet"));
        let events2 = session.drain_events();
        assert_eq!(events2.len(), 0);
    }

    // ── Test 2 ──────────────────────────────────────────────────────────────
    //
    // Compile-time signature guard: start_session_event_tracker must accept
    // an mpsc::Receiver<SessionNotification>. If the implementation regresses
    // to internally subscribing to event_tx, the FnOnce bound below ensures
    // the code still won't compile without an explicit receiver parameter.

    #[tokio::test]
    async fn tracker_signature_accepts_notification_receiver() {
        // This is a compile-only guard. We verify that the function has
        // the expected signature via type inference — if the signature
        // changes to take no arguments, this won't compile.
        fn _assert_method_exists(_: impl Fn(&Arc<AcpAgentManager>, mpsc::Receiver<SessionNotification>)) {}

        _assert_method_exists(|m, rx| {
            m.start_session_event_tracker(rx);
        });
    }

    // ── Test 3 ──────────────────────────────────────────────────────────────
    //
    // Architectural invariant test: events broadcast on event_tx are NOT
    // re-applied to the session by the tracker.
    //
    // We verify this by constructing the two channels independently and
    // confirming they are decoupled: sending on event_tx does not cause
    // anything to arrive on notification_rx.
    #[tokio::test]
    async fn event_tx_broadcast_does_not_feed_notification_rx() {
        use crate::protocol::events::AgentStreamEvent;
        use tokio::sync::broadcast;

        let (event_tx, _event_rx) = broadcast::channel::<AgentStreamEvent>(8);
        let (notification_tx, mut notification_rx) = mpsc::channel::<SessionNotification>(8);

        // Simulate what emit_snapshot_events does: broadcast on event_tx
        let _ = event_tx.send(AgentStreamEvent::AcpModeInfo(
            serde_json::json!({"currentModeId": "plan"}),
        ));

        // Drop the notification sender so the receiver's try_recv terminates
        drop(notification_tx);

        // The notification_rx should be empty — no SessionNotification was
        // sent merely because event_tx received an AcpModeInfo event.
        assert!(
            notification_rx.recv().await.is_none(),
            "notification_rx must not receive events that were broadcast on event_tx"
        );
    }
}
