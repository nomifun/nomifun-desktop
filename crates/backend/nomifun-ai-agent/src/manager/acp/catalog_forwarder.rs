//! Forwards ACP session events to the catalog sync channel.
//!
//! Subscribes to the manager's stream event broadcast and projects
//! capability-advertising events (modes, models, commands, capabilities,
//! auth methods) into `AgentHandshake` partials that the registry's
//! catalog consumer writes to `agent_metadata`.

use nomifun_api_types::AgentHandshake;
use nomifun_common::normalize_keys_to_snake_case;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::protocol::events::AgentStreamEvent;
use crate::registry::CatalogSender;

/// Subscriber that projects session-driven ACP events into the
/// `agent_metadata` catalog so the stored handshake blob stays in sync
/// with what the CLI is actually advertising.
///
/// One task per `AcpAgentManager`; the task exits automatically when
/// the broadcast channel closes (i.e. the manager is dropped).
pub struct CatalogForwarder;

impl CatalogForwarder {
    /// Spawn the forwarder task. The returned handle is not normally
    /// awaited — callers drop it and rely on the broadcast channel
    /// closing to terminate the task.
    pub fn spawn(
        agent_id: String,
        mut event_rx: broadcast::Receiver<AgentStreamEvent>,
        catalog_tx: CatalogSender,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        if let Some(partial) = catalog_partial_from_event(&event) {
                            catalog_tx.send_partial(agent_id.clone(), partial);
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
            debug!(agent_id, "CatalogForwarder exiting");
        })
    }
}

/// Project an `AgentStreamEvent` onto the subset of `AgentHandshake`
/// fields the catalog cares about. Returns `None` for unrelated
/// events — the forwarder filters on that.
///
/// Event payloads may arrive here either already snake_case (from
/// `emit_snapshot_events`) or camelCase (from `SessionUpdate::*`
/// translation in `stream_event.rs`). We re-normalise unconditionally
/// so the persisted handshake blob is uniform; `camel_to_snake` is
/// idempotent on snake_case input.
fn catalog_partial_from_event(event: &AgentStreamEvent) -> Option<AgentHandshake> {
    fn snake(mut v: Value) -> Value {
        normalize_keys_to_snake_case(&mut v);
        v
    }
    match event {
        AgentStreamEvent::AcpModeInfo(v) => Some(AgentHandshake {
            available_modes: Some(snake(v.clone())),
            ..Default::default()
        }),
        AgentStreamEvent::AcpModelInfo(v) => Some(AgentHandshake {
            available_models: Some(snake(v.clone())),
            ..Default::default()
        }),
        AgentStreamEvent::AcpConfigOption(v) => Some(AgentHandshake {
            config_options: Some(snake(v.clone())),
            ..Default::default()
        }),
        AgentStreamEvent::AvailableCommands(data) => {
            // `AvailableCommand` is an ACP SDK struct — normalise on
            // the way into the catalog so the stored blob is snake_case.
            let mut cmds = serde_json::to_value(&data.commands).ok()?;
            normalize_keys_to_snake_case(&mut cmds);
            Some(AgentHandshake {
                available_commands: Some(cmds),
                ..Default::default()
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::events::StartEventData;
    use serde_json::json;

    /// Each session-driven event projects onto exactly one handshake
    /// field. Unrelated events produce `None` so the forwarder sends
    /// nothing for them.
    #[test]
    fn catalog_partial_covers_session_fields() {
        let modes = catalog_partial_from_event(&AgentStreamEvent::AcpModeInfo(json!({"x": 1})))
            .expect("mode event must project");
        assert_eq!(modes.available_modes, Some(json!({"x": 1})));
        assert!(modes.available_models.is_none());

        let models =
            catalog_partial_from_event(&AgentStreamEvent::AcpModelInfo(json!([1]))).expect("model event must project");
        assert_eq!(models.available_models, Some(json!([1])));

        let cfg = catalog_partial_from_event(&AgentStreamEvent::AcpConfigOption(json!([
            {"id":"mode"}
        ])))
        .expect("config event must project");
        assert_eq!(cfg.config_options, Some(json!([{"id":"mode"}])));

        // An unrelated event emits no update.
        assert!(catalog_partial_from_event(&AgentStreamEvent::Start(StartEventData { session_id: None })).is_none());
    }
}
