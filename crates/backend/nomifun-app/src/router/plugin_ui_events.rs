//! Read-only projection of the existing user event bus. Never owns a turn or a log.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_plugin_platform::runtime::PluginRuntimeApplicationService;
use nomifun_realtime::{UserEventEnvelope, WebSocketManager};
use serde_json::{Value, json};
use tokio::sync::broadcast;

const BATCH_LIMIT: usize = 32;
const EVENT_BYTE_LIMIT: usize = 64 * 1024;

pub(super) async fn forward(
    mut receiver: broadcast::Receiver<UserEventEnvelope>,
    manager: Arc<WebSocketManager>,
    application: Arc<PluginRuntimeApplicationService>,
) {
    if !crate::router::plugin_product::agent_ui_admission::enabled() {
        return;
    }
    let resync = super::LagResyncCoalescer::new(manager.clone(), super::RESYNC_COALESCE_INTERVAL);
    loop {
        let first = match receiver.recv().await {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                resync.on_lag(skipped);
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let mut batch = vec![first];
        while batch.len() < BATCH_LIMIT {
            match receiver.try_recv() {
                Ok(event) => batch.push(event),
                Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                    batch.clear();
                    resync.on_lag(skipped);
                    break;
                }
                Err(_) => break,
            }
        }
        // Group only within this bounded batch, preserving each Session's order.
        // Scope queries never block the primary user WebSocket bridge.
        let mut groups: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
        let mut gaps = BTreeSet::new();
        for envelope in batch {
            if envelope.event.name != "message.stream" {
                continue;
            }
            let Some(id) = envelope.event.data["conversation_id"].as_str() else {
                continue;
            };
            if !envelope.event.data["type"].is_string() {
                continue;
            }
            if serde_json::to_vec(&envelope.event.data)
                .map_or(true, |bytes| bytes.len() > EVENT_BYTE_LIMIT)
            {
                gaps.insert(envelope.user_id);
                continue;
            }
            groups
                .entry((envelope.user_id, id.to_owned()))
                .or_default()
                .push(envelope.event.data);
        }
        for ((owner, session), events) in groups {
            // Do not deliver older fragments after this batch's resync notice.
            if gaps.contains(&owner) {
                continue;
            }
            match application
                .project_agent_session_stream(&owner, &session, &events)
                .await
            {
                Ok(projected) => {
                    for event in projected {
                        if let Ok(value) = serde_json::to_value(event) {
                            manager.broadcast_to_user(
                                &owner,
                                WebSocketMessage::new("plugin.agent-session.stream", value),
                            );
                        }
                    }
                }
                Err(error) => {
                    // Never forward private Session diagnostics to an old view.
                    tracing::debug!(%error, "Plugin Session event projection unavailable");
                    gaps.insert(owner);
                }
            }
        }
        for owner in gaps {
            notify_resync(&manager, &owner);
        }
    }
}

fn notify_resync(manager: &WebSocketManager, owner: &str) {
    manager.broadcast_to_user(
        owner,
        WebSocketMessage::new("plugin.agent-session.resync-required", json!({})),
    );
}
