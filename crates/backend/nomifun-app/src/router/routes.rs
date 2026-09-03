//! Top-level router assembly: middleware stack + module route merges.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::DefaultBodyLimit;
use axum::http::Method;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::{Router, middleware};
use tower_http::cors::{Any, CorsLayer};

use nomifun_ai_agent::agent_routes;
use nomifun_assets::{AssetRouterState, asset_routes};
use nomifun_preset::preset_routes;
use nomifun_auth::{
    AuthRouterState, AuthState, InstanceOwnerState, TrustState, auth_middleware, auth_routes,
    csrf_middleware, require_instance_owner_middleware, require_local_trust_middleware,
    security_headers_middleware, trust_resolve_middleware,
};
use nomifun_channel::channel_routes;
use nomifun_companion::{companion_public_routes, companion_routes};
use nomifun_customer_service::customer_service_routes;
use nomifun_miniapp::{miniapp_public_routes, miniapp_routes};
use nomifun_workshop::{workshop_public_routes, workshop_routes};
use nomifun_creation::creation_routes;
use nomifun_conversation::{
    conversation_ops_routes, conversation_routes, creative_studio_agent_session_routes,
};
use nomifun_cron::cron_routes;
use nomifun_extension::{extension_routes, hub_routes, skill_routes};
use nomifun_file::file_routes;
use nomifun_idmm::idmm_routes;
use nomifun_knowledge::knowledge_routes;
use nomifun_mcp::mcp_routes;
use nomifun_office::{office_proxy_routes, office_routes};
use nomifun_agent_execution::{agent_execution_routes, agent_execution_template_routes};
use nomifun_realtime::{UserEventEnvelope, WebSocketManager, WsHandlerState, ws_upgrade_handler};
use nomifun_requirement::requirement_routes;
use nomifun_shell::shell_routes;
use nomifun_system::{connection_test_routes, system_routes};
use nomifun_terminal::terminal_routes;
use nomifun_webhook::webhook_routes;

use crate::services::AppServices;

use super::computer_permissions::{
    computer_permission_status, open_permission_settings, request_computer_permission,
};
use super::health::{
    health_check, knowledge_global_status_handler, mcp_register_template_handler,
    register_knowledge_global_handler, register_knowledge_handler,
    unregister_knowledge_global_handler,
};
use super::model_failover::{ModelFailoverRouterState, model_failover_routes};
use super::state::{ModuleStates, build_module_states, build_ws_state};
use super::trace::with_access_log;

async fn forward_instance_events(
    mut receiver: tokio::sync::broadcast::Receiver<nomifun_api_types::WebSocketMessage<serde_json::Value>>,
    ws_manager: Arc<WebSocketManager>,
    authoritative_user_id: Arc<str>,
) {
    // Instance-bus lag drops events just like user-bus lag: the same
    // coalesced invalidation applies. Each bridge task owns its own
    // coalescer (they run on independent receivers), so a simultaneous lag
    // on both buses can at worst double the invalidation, which is idempotent.
    let resync = LagResyncCoalescer::new(ws_manager.clone(), RESYNC_COALESCE_INTERVAL);
    loop {
        match receiver.recv().await {
            Ok(event) => ws_manager.broadcast_to_user(&authoritative_user_id, event),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, audience = "instance", "realtime bridge lagged; continuing from newest event");
                resync.on_lag(skipped);
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn forward_user_events(
    mut receiver: tokio::sync::broadcast::Receiver<UserEventEnvelope>,
    ws_manager: Arc<WebSocketManager>,
) {
    let resync = LagResyncCoalescer::new(ws_manager.clone(), RESYNC_COALESCE_INTERVAL);
    loop {
        match receiver.recv().await {
            Ok(envelope) => ws_manager.broadcast_to_user(&envelope.user_id, envelope.event),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, audience = "user", "realtime bridge lagged; continuing from newest event");
                // Tokio's lag error does not expose the discarded envelopes,
                // so their audiences cannot be reconstructed safely. The
                // invalidation contains no inventory data: every connection
                // can safely receive it and refresh its own authenticated
                // snapshot. Sending directly avoids the already-lagged bus.
                // Backward-compatible clients refresh on every inventory event;
                // marker-aware clients explicitly classify this as a resync.
                //
                // Coalesced (F61): a sustained burst of unrelated events (
                // terminal scrollback, agent step updates) produces repeated
                // lag errors; without coalescing every one became another
                // all-clients browser-inventory refetch. At most one resync per
                // interval is broadcast, with a guaranteed trailing resync
                // covering the last suppressed lag.
                resync.on_lag(skipped);
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// Minimum spacing between all-clients lag-resync invalidations (F61).
const RESYNC_COALESCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Rate-limits the lag-resync invalidation broadcasts.
///
/// Leading edge: a lag outside the interval broadcasts immediately. Inside
/// the interval, skipped counts accumulate and exactly one trailing broadcast
/// is scheduled for the interval boundary, so no invalidation is ever lost —
/// clients that refetched on the previous resync still learn about events
/// dropped after it.
///
/// Each firing broadcasts TWO frames: the legacy
/// `browser.inventory.changed` invalidation (backward compat — older clients
/// only refresh browser inventory on it) and the generic
/// `sync.resync-required` marker every domain UI can consume.
#[derive(Clone)]
struct LagResyncCoalescer {
    ws_manager: Arc<WebSocketManager>,
    interval: std::time::Duration,
    state: Arc<std::sync::Mutex<ResyncCoalescerState>>,
}

struct ResyncCoalescerState {
    next_allowed: tokio::time::Instant,
    pending_skipped: u64,
    trailing_scheduled: bool,
}

impl LagResyncCoalescer {
    fn new(ws_manager: Arc<WebSocketManager>, interval: std::time::Duration) -> Self {
        Self {
            ws_manager,
            interval,
            state: Arc::new(std::sync::Mutex::new(ResyncCoalescerState {
                next_allowed: tokio::time::Instant::now(),
                pending_skipped: 0,
                trailing_scheduled: false,
            })),
        }
    }

    fn on_lag(&self, skipped: u64) {
        let now = tokio::time::Instant::now();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if now >= state.next_allowed && !state.trailing_scheduled {
            state.next_allowed = now + self.interval;
            drop(state);
            self.broadcast_resync(skipped);
            return;
        }
        state.pending_skipped = state.pending_skipped.saturating_add(skipped);
        if state.trailing_scheduled {
            return;
        }
        state.trailing_scheduled = true;
        let deadline = state.next_allowed;
        drop(state);
        let coalescer = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            let skipped = {
                let mut state = coalescer
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.trailing_scheduled = false;
                state.next_allowed = tokio::time::Instant::now() + coalescer.interval;
                std::mem::take(&mut state.pending_skipped)
            };
            coalescer.broadcast_resync(skipped);
        });
    }

    /// One coalesced firing: legacy inventory invalidation first, then the
    /// generic resync marker, so old clients act on the first frame and
    /// marker-aware clients on the second.
    fn broadcast_resync(&self, skipped: u64) {
        self.ws_manager
            .broadcast_all(crate::browser_inventory_events::browser_inventory_resync_event(skipped));
        self.ws_manager
            .broadcast_all(nomifun_api_types::WebSocketMessage::new(
                "sync.resync-required",
                serde_json::json!({"scope": "all", "skipped": skipped}),
            ));
    }
}

/// Apply the two installation-control-plane gates in the only valid order:
/// authentication runs first and injects `CurrentUser`, then the owner gate
/// compares that stable id with the canonical installation owner.
fn protect_instance_owner(
    router: Router,
    auth_state: &AuthState,
    owner_state: &InstanceOwnerState,
) -> Router {
    router
        .route_layer(from_fn_with_state(
            owner_state.clone(),
            require_instance_owner_middleware,
        ))
        .route_layer(from_fn_with_state(auth_state.clone(), auth_middleware))
}

/// Fallible transitional compatibility-router assembly with all legacy routes
/// and global middleware.
///
/// Middleware stack (outermost → innermost):
/// 1. Security response headers (X-Frame-Options, etc.)
/// 2. CSRF protection (Double Submit Cookie)
/// 3. Route handlers (auth routes + system routes + conversation routes + file routes + health check)
pub async fn try_create_router(services: &AppServices) -> anyhow::Result<Router> {
    services.require_legacy_compatibility_root("try_create_router")?;
    let boot = Instant::now();
    tracing::info!("startup: transitional compatibility router assembly started");

    // Bridge event bus → WebSocket manager: forward all broadcast events
    // to connected WebSocket clients.
    let event_rx = services.event_bus.subscribe();
    let ws_manager = services.ws_manager.clone();
    tokio::spawn(forward_instance_events(
        event_rx,
        ws_manager,
        services.authoritative_user_id.clone(),
    ));

    // User-scoped events travel on a separate internal channel. Server-side
    // observers can subscribe without exposing those events to other users,
    // while this bridge delivers each envelope only to its authenticated owner.
    let user_event_rx = services.event_bus.subscribe_user();
    let ws_manager = services.ws_manager.clone();
    tokio::spawn(forward_user_events(user_event_rx, ws_manager));
    match services.knowledge_service.drain_pending_tree_events().await {
        Ok(published) if published > 0 => {
            tracing::info!(published, "startup: published pending knowledge-tree events");
        }
        Ok(_) => {}
        Err(error) => {
            // The durable rows stay pending. Router availability must not be
            // blocked by a transient realtime bridge failure; the next
            // relocation and the next boot both retry the same outbox rows.
            tracing::warn!(%error, "startup: pending knowledge-tree event drain failed");
        }
    }

    let (states, channel_components) = build_module_states(services).await;
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: module states built"
    );

    // Wire the Platform Gateway MCP deps now that the module services exist.
    // The gateway server itself started inside `AppServices::from_config`
    // (before the agent factory, which carries its connection config).
    //
    // requirement_service / auto_work_runner / idmm_service come from the
    // ROUTER STATES (not the bare singletons): those instances carry the
    // conversation-service / terminal-driver attachments the gateway's
    // autowork + idmm tools need, and share the live loop maps with the REST
    // routes so a gateway toggle and a UI toggle act on the same state.
    let gateway_deps = Arc::new(nomifun_gateway::CompatibilityCapabilityHost {
        authoritative_user_id: services.authoritative_user_id.clone(),
        conversation: Arc::new(super::legacy_conversation_port::LegacyConversationCapabilityPort::new(
            states.conversation.service.clone(),
            services.agent_runtime_registry.clone(),
        )),
        cron_service: states.cron.cron_service.clone(),
        requirement_service: states.requirement.requirement_service.clone(),
        companion_service: services.companion_service.clone(),
        terminal_service: services.terminal_service.clone(),
        provider_repo: Arc::new(nomifun_db::SqliteProviderRepository::new(
            services.database.pool().clone(),
        )),
        provider_model_repo: Arc::new(nomifun_db::SqliteProviderModelRepository::new(
            services.database.pool().clone(),
        )),
        provider_model_capability_repo: Arc::new(
            nomifun_db::SqliteProviderModelCapabilityRepository::new(
                services.database.pool().clone(),
            ),
        ),
        idmm_service: states.idmm.service.clone(),
        knowledge_service: services.knowledge_service.clone(),
        // Creative Studio project/asset + generation services: the SAME
        // singletons used by `/api/creative-studio/*`, so Gateway operations and
        // product requests observe one project store and one live task queue.
        workshop_service: services.workshop_service.clone(),
        creation_service: services.creation_service.clone(),
        auto_work_runner: states.requirement.auto_work_runner.clone(),
        // System domain: reuse the SAME service instances the system routes use
        // (states.system is still owned here; it is moved into `system_routes`
        // later in `create_router_with_states`). A gateway theme/toggle/provider
        // change and a UI change then act on identical state.
        settings_service: states.system.settings_service.clone(),
        client_pref_service: states.system.client_pref_service.clone(),
        provider_service: states.system.provider_service.clone(),
        model_fetch_service: states.system.model_fetch_service.clone(),
        // Channel domain: same plugin manager / pairing / settings the
        // `/api/channels` routes use (states.channel is cloned, then moved
        // into `channel_routes` later).
        channel_state: states.channel.clone(),
        file_service: states.file.file_service.clone(),
        shell_service: states.shell.shell_service.clone(),
        mcp_config_service: states.mcp.config_service.clone(),
        extension_registry: states.extension.registry.clone(),
        hub_index_manager: states.hub.index_manager.clone(),
        hub_installer: states.hub.installer.clone(),
        skill_paths: states.skill.skill_paths.clone(),
        agent_service: states.agent.service.clone(),
        client_pref_repo: Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
            services.database.pool().clone(),
        )),
        // REST, model tools and boot recovery share the same public facade and
        // therefore one scheduler handle map and one durable state machine.
        agent_execution_engine: states.agent_execution.clone(),
    });
    services.inject_gateway_deps(gateway_deps.clone()).await;
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: gateway MCP deps injected"
    );

    // Start the channel message loop.
    tokio::spawn(
        channel_components
            .message_loop
            .run(channel_components.message_rx),
    );
    // Start the busy-time queue drain (spec D1): it consumes `turn.completed`
    // envelopes from the same in-process bus the conversation service
    // publishes through, recovers persisted queued prompts on startup, and
    // expires stale ones.
    tokio::spawn(
        channel_components
            .queue_drain
            .run(services.event_bus.subscribe_user()),
    );

    // Spec D2: register the delivery-notify observer on the conversation
    // service instance that executes gateway `nomi_send_to_conversation`
    // turns (the same instance wired into CompatibilityCapabilityHost above). When a watched
    // turn completes, the observer injects a receipt message into the
    // requester session; a channel-bound requester relays the companion's
    // summary to its IM chat through the standard stream relay.
    let delivery_notify_observer = Arc::new(crate::delivery_notify::DeliveryNotifyObserver::new(
        states.conversation.service.clone(),
        services.agent_runtime_registry.clone(),
        services.authoritative_user_id.clone(),
        states.channel.repo.clone(),
        channel_components.manager.clone()
            as Arc<dyn nomifun_channel::stream_relay::ChannelSender>,
        channel_components.message_service.stop_confirmations(),
        channel_components.message_service.asset_resolver(),
    ));
    states
        .conversation
        .service
        .with_turn_completion_observer(delivery_notify_observer);
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: channel message loop spawned"
    );

    // Restore enabled channel plugins (starts receiving IM messages)
    let chan_mgr = channel_components.manager;
    let chan_factory = channel_components.plugin_factory;
    {
        let mgr = chan_mgr.clone();
        let factory = chan_factory.clone();
        let companion_service = services.companion_service.clone();
        tokio::spawn(async move {
            // Self-heal ghost owner bindings BEFORE restoring: a channel row
            // bound to a 伙伴 that was deleted before the delete-hook existed
            // (or missed by it) keeps reserving its bot identity
            // (UNIQUE(type,bot_key)), so re-enabling that bot under a live owner
            // fails with "already bound" forever. Unbind rows whose owner is no
            // longer in the roster so they become adoptable again. The roster is
            // scanned into memory at service construction, so an empty list
            // here means the owner really is gone.
            let live_companions: std::collections::HashSet<String> = companion_service
                .list_companions()
                .await
                .into_iter()
                .map(|c| c.companion_id)
                .filter(|id| !id.is_empty())
                .collect();
            // Safety valve: never mass-unbind on an ambiguous "no owners at all"
            // signal. If the user genuinely has zero companions, there is
            // nothing to reconcile against — skip rather than risk unbinding
            // every row.
            if live_companions.is_empty() {
                tracing::info!("reconcile_orphaned_owners: empty roster, skipping to avoid mass-unbind");
            } else {
                mgr.reconcile_orphaned_owners(&live_companions).await;
            }

            if let Err(e) = mgr.restore_plugins(&factory).await {
                tracing::warn!(error = %e, "failed to restore channel plugins");
            }
        });
    }
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: channel plugin restore scheduled"
    );

    // Watchdog: plugin receive loops give up after exhausting their
    // reconnect budget, leaving DB + frontend claiming "running" for a dead
    // plugin. The watchdog persists the real status, broadcasts the change,
    // and attempts rate-limited automatic restarts.
    let _channel_watchdog = chan_mgr.spawn_watchdog(
        chan_factory,
        nomifun_channel::manager::WatchdogConfig::default(),
    );
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: channel plugin watchdog spawned"
    );

    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: route tree build started"
    );
    let ws_state = build_ws_state(services);
    let remote_auth_admission =
        Arc::new(nomifun_auth::RemoteAuthAdmissionFence::new());
    let router = create_transitional_router_with_all_state(
        services,
        states,
        ws_state,
        remote_auth_admission.clone(),
    );
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: transitional compatibility route assembly completed"
    );
    Ok(router)
}

/// Create the transitional compatibility router.
///
/// Fresh-v4 production hosts must use `FreshV4Application::router`; this
/// `AppServices` graph remains only for legacy-compatible tests and explicitly
/// selected transitional embeddings.
pub async fn create_router(services: &AppServices) -> Router {
    try_create_router(services)
        .await
        .unwrap_or_else(|error| panic!("application router assembly failed: {error:#}"))
}

#[cfg(test)]
mod realtime_bridge_tests {
    use super::{LagResyncCoalescer, forward_instance_events, forward_user_events};
    use nomifun_api_types::WebSocketMessage;
    use nomifun_realtime::{BroadcastEventBus, EventBroadcaster, UserEventSink, WebSocketManager, WsOutbound};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    async fn receive_event(receiver: &mut mpsc::Receiver<WsOutbound>) -> WebSocketMessage<serde_json::Value> {
        let outbound = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("bridge must forward after lag")
            .expect("client channel must remain open");
        let WsOutbound::Text(text) = outbound else {
            panic!("expected a text event")
        };
        serde_json::from_str(&text).expect("forwarded websocket event must be valid JSON")
    }

    /// Each coalesced firing emits the backward-compatible inventory
    /// invalidation first, then the generic resync marker.
    async fn receive_resync_pair(
        receiver: &mut mpsc::Receiver<WsOutbound>,
        expected_skipped: u64,
    ) {
        let inventory = receive_event(receiver).await;
        assert_eq!(inventory.name, "browser.inventory.changed");
        assert_eq!(inventory.data["change_kind"], "resync_required");
        assert_eq!(inventory.data["skipped"], expected_skipped);

        let generic = receive_event(receiver).await;
        assert_eq!(generic.name, "sync.resync-required");
        assert_eq!(generic.data["scope"], "all");
        assert_eq!(generic.data["skipped"], expected_skipped);
    }

    #[tokio::test]
    async fn lag_resyncs_are_coalesced_with_a_guaranteed_trailing_broadcast() {
        let manager = Arc::new(WebSocketManager::new());
        let (client_tx, mut client_rx) = mpsc::channel(8);
        manager.add_client("owner-a".into(), "token".into(), client_tx);
        let coalescer = LagResyncCoalescer::new(
            Arc::clone(&manager),
            std::time::Duration::from_millis(200),
        );

        // A burst of lag errors: leading edge broadcasts immediately, the
        // rest coalesce into exactly one trailing broadcast at the interval.
        coalescer.on_lag(1);
        coalescer.on_lag(2);
        coalescer.on_lag(3);

        receive_resync_pair(&mut client_rx, 1).await;
        assert!(
            client_rx.try_recv().is_err(),
            "suppressed lags must not broadcast before the interval boundary"
        );

        // The scheduled trailing task fires at the interval boundary and no
        // invalidation is lost: the suppressed counts accumulate into it.
        receive_resync_pair(&mut client_rx, 5).await;
        assert!(
            client_rx.try_recv().is_err(),
            "three lag errors must produce exactly two coalesced firings"
        );

        // A later lag (outside the interval) broadcasts immediately again.
        tokio::time::sleep(std::time::Duration::from_millis(450)).await;
        coalescer.on_lag(7);
        receive_resync_pair(&mut client_rx, 7).await;
    }

    #[tokio::test]
    async fn instance_bridge_emits_resync_to_all_clients_after_lag() {
        let bus = Arc::new(BroadcastEventBus::new(1));
        let receiver = bus.subscribe();
        bus.broadcast(WebSocketMessage::new("dropped", json!({})));
        bus.broadcast(WebSocketMessage::new("after-lag", json!({"seq": 2})));

        let manager = Arc::new(WebSocketManager::new());
        let (client_tx, mut client_rx) = mpsc::channel(4);
        let (other_tx, mut other_rx) = mpsc::channel(4);
        manager.add_client("owner-a".into(), "token".into(), client_tx);
        manager.add_client("owner-b".into(), "other-token".into(), other_tx);
        let task = tokio::spawn(forward_instance_events(
            receiver,
            manager,
            Arc::from("owner-a"),
        ));

        // Instance-bus lag drops instance-scoped events too, so every client
        // gets the same coalesced invalidation as on user-bus lag.
        receive_resync_pair(&mut client_rx, 1).await;
        receive_resync_pair(&mut other_rx, 1).await;

        // The bridge then continues from the newest event, still scoped to
        // the authoritative user.
        let event = receive_event(&mut client_rx).await;
        assert_eq!(event.name, "after-lag");
        assert_eq!(event.data["seq"], 2);
        assert!(other_rx.try_recv().is_err());
        task.abort();
    }

    #[tokio::test]
    async fn user_bridge_emits_resync_after_lag_and_keeps_normal_events_scoped() {
        // Keep only the newest envelope so every pre-existing event is
        // deterministically discarded before the bridge resumes.  With a
        // capacity of two, the middle event could survive the lag and be
        // legitimately delivered to the other owner, making this test assert
        // on scheduling rather than the resync contract.
        let bus = Arc::new(BroadcastEventBus::new(1));
        let receiver = bus.subscribe_user();
        // This first inventory event is observed before the later burst makes
        // the receiver lag. It models an already-open browser page.
        bus.send_to_user(
            "owner-a",
            WebSocketMessage::new("browser.inventory.changed", json!({"sequence": 1})),
        );

        let manager = Arc::new(WebSocketManager::new());
        let (owner_tx, mut owner_rx) = mpsc::channel(8);
        let (other_tx, mut other_rx) = mpsc::channel(8);
        manager.add_client("owner-a".into(), "token-a".into(), owner_tx);
        manager.add_client("owner-b".into(), "token-b".into(), other_tx);
        let task = tokio::spawn(forward_user_events(receiver, manager));

        let initial = receive_event(&mut owner_rx).await;
        assert_eq!(initial.name, "browser.inventory.changed");
        assert!(other_rx.try_recv().is_err());

        // Pause the task so a deterministic capacity overflow occurs after it
        // has already consumed the initial inventory event.
        task.abort();
        let receiver = bus.subscribe_user();
        bus.send_to_user("owner-a", WebSocketMessage::new("dropped-a", json!({})));
        bus.send_to_user("owner-b", WebSocketMessage::new("dropped-b", json!({})));
        bus.send_to_user(
            "owner-a",
            WebSocketMessage::new("after-lag", json!({"seq": 3})),
        );
        let manager = Arc::new(WebSocketManager::new());
        let (owner_tx, mut owner_rx) = mpsc::channel(8);
        let (other_tx, mut other_rx) = mpsc::channel(8);
        manager.add_client("owner-a".into(), "token-a".into(), owner_tx);
        manager.add_client("owner-b".into(), "token-b".into(), other_tx);
        let task = tokio::spawn(forward_user_events(receiver, manager));

        // Both clients receive the invalidation pair; neither frame carries
        // any inventory data from the dropped envelopes.
        for rx in [&mut owner_rx, &mut other_rx] {
            let inventory = receive_event(rx).await;
            assert_eq!(inventory.name, "browser.inventory.changed");
            assert_eq!(inventory.data["change_kind"], "resync_required");
            assert_eq!(inventory.data["resync_required"], true);
            assert!(inventory.data.get("sequence").is_none());

            let generic = receive_event(rx).await;
            assert_eq!(generic.name, "sync.resync-required");
            assert_eq!(generic.data["scope"], "all");
            assert!(generic.data.get("sequence").is_none());
        }

        let event = receive_event(&mut owner_rx).await;
        assert_eq!(event.name, "after-lag");
        assert_eq!(event.data["seq"], 3);
        assert!(other_rx.try_recv().is_err());
        task.abort();
    }
}

/// Create the application router with custom module states.
///
/// Used for testing when specific service overrides are needed
/// (e.g. injecting a mock HTTP server URL for version check).
pub fn create_router_with_states(services: &AppServices, states: ModuleStates) -> Router {
    services
        .require_legacy_compatibility_root("create_router_with_states")
        .unwrap_or_else(|error| panic!("application router assembly failed: {error:#}"));
    let ws_state = build_ws_state(services);
    create_router_with_all_state(services, states, ws_state)
}

/// Create the application router with custom module states and WebSocket state.
///
/// Full-control variant used by tests that need to override
/// module services and WebSocket behaviour.
pub fn create_router_with_all_state(
    services: &AppServices,
    states: ModuleStates,
    ws_state: WsHandlerState,
) -> Router {
    services
        .require_legacy_compatibility_root("create_router_with_all_state")
        .unwrap_or_else(|error| panic!("application router assembly failed: {error:#}"));
    create_transitional_router_with_all_state(
        services,
        states,
        ws_state,
        Arc::new(nomifun_auth::RemoteAuthAdmissionFence::new()),
    )
}

fn create_transitional_router_with_all_state(
    services: &AppServices,
    states: ModuleStates,
    ws_state: WsHandlerState,
    remote_auth_admission: Arc<nomifun_auth::RemoteAuthAdmissionFence>,
) -> Router {
    let boot = Instant::now();
    tracing::info!("startup: route tree build with states started");
    services
        .ws_manager
        .ensure_heartbeat(ws_state.token_authenticator.clone());

    let auth_state = AuthRouterState {
        jwt_service: services.jwt_service.clone(),
        user_repo: services.user_repo.clone(),
        cookie_config: services.cookie_config.clone(),
        qr_token_store: services.qr_token_store.clone(),
    };

    let auth_mw_state = AuthState {
        jwt_service: services.jwt_service.clone(),
        user_repo: services.user_repo.clone(),
        cookie_config: services.cookie_config.clone(),
    };
    let instance_owner_state =
        InstanceOwnerState::new(services.authoritative_user_id.clone());

    // LAN robot gateway. Assembled here because this is where the
    // `ConversationService` the robot sessions dispatch through exists; the two
    // faces are mounted separately below because they belong in different
    // middleware groups.
    let robot_faces = services.robot.as_ref().map(|robot| {
        crate::robot_wiring::mount(
            robot,
            states.conversation.service.clone(),
            services.agent_runtime_registry.clone(),
            services.companion_service.clone(),
            services.authoritative_user_id.clone(),
            services.data_dir.clone(),
        )
    });

    // Installation-scoped Remote access-token mint/revoke/status endpoints.
    // Local-trust gated and independent of every companion lifecycle.
    let instance_token_state = crate::router::instance_token_routes::InstanceTokenRouterState {
        provider_repo: services.provider_repo.clone(),
        token_repo: services.instance_token_repo.clone(),
        token_validator: services.instance_token_validator.clone(),
        admission: remote_auth_admission,
    };

    // System routes protected by auth middleware
    let system_authenticated = protect_instance_owner(
        system_routes(states.system),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Conversation routes protected by auth middleware
    let conversation_authenticated = conversation_routes(states.conversation.clone())
        .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    let creative_studio_agent_session_authenticated = protect_instance_owner(
        creative_studio_agent_session_routes(states.conversation.clone()),
        &auth_mw_state,
        &instance_owner_state,
    );

    let conversation_ops_authenticated = conversation_ops_routes(states.conversation)
        .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // SSH host book (owner-only): saved connection profiles + test-connection.
    let ssh_host_authenticated = protect_instance_owner(
        nomifun_ssh::ssh_host_routes(states.ssh_host),
        &auth_mw_state,
        &instance_owner_state,
    );

    // 小程序 (mini-app) library (owner-only): metadata CRUD. The document serve
    // route is split off into `miniapp_public_routes` below and mounted
    // auth-exempt, because an iframe document load carries no trust header.
    // `states.miniapp` is cloned so both routers share the one service.
    let miniapp_authenticated = protect_instance_owner(
        miniapp_routes(states.miniapp.clone()),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Unified agent listing/refresh/test routes protected by auth middleware
    let agent_authenticated = protect_instance_owner(
        agent_routes(states.agent),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Phase 3 (review #6/#12): global model-failover config GET/PUT, auth-gated.
    // Path string must match the frontend `agentModelFailover` exactly.
    let model_failover_authenticated = protect_instance_owner(
        model_failover_routes(ModelFailoverRouterState {
            client_prefs: Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
                services.database.pool().clone(),
            )),
        }),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Connection test routes (Bedrock, Gemini) protected by auth middleware
    let connection_test_authenticated = protect_instance_owner(
        connection_test_routes(states.connection_test),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Filesystem access executes as the backend OS user and includes the app
    // data directory. It is therefore installation-owner control, not a
    // row-scoped multi-user resource.
    let file_authenticated = protect_instance_owner(
        file_routes(states.file),
        &auth_mw_state,
        &instance_owner_state,
    );

    // MCP routes protected by auth middleware
    let mcp_authenticated = protect_instance_owner(
        mcp_routes(states.mcp),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Extension routes protected by auth middleware
    let extension_authenticated = protect_instance_owner(
        extension_routes(states.extension),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Hub routes protected by auth middleware
    let hub_authenticated = protect_instance_owner(
        hub_routes(states.hub),
        &auth_mw_state,
        &instance_owner_state,
    );

    // This router is itself the explicitly selected transitional composition.
    // Fresh-v4 owns its canonical control-plane routes in its own application
    // router and never reaches this legacy skill catalog.
    let skill_authenticated = Some(protect_instance_owner(
        skill_routes(states.skill),
        &auth_mw_state,
        &instance_owner_state,
    ));

    // Channel routes protected by auth middleware
    let channel_authenticated = protect_instance_owner(
        channel_routes(states.channel),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Cron routes protected by auth middleware
    let cron_authenticated = cron_routes(states.cron)
        .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Requirements Platform routes protected by auth middleware
    let requirement_authenticated = protect_instance_owner(
        requirement_routes(states.requirement),
        &auth_mw_state,
        &instance_owner_state,
    );

    // IDMM (Intelligent Decision-Making Mode) routes protected by auth middleware
    let idmm_authenticated = protect_instance_owner(
        idmm_routes(states.idmm),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Companion (nomi) routes protected by auth middleware
    let companion_authenticated = protect_instance_owner(
        companion_routes(states.companion.clone()),
        &auth_mw_state,
        &instance_owner_state,
    );

    // 客服独立域 (customer-service domain) — roster/bindings/notes/dialogues
    // REST surface. Protected by auth middleware.
    let customer_service_authenticated = protect_instance_owner(
        customer_service_routes(states.customer_service.clone()),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Creative Studio project/asset/template routes plus generation task routes
    // — owner-only management surfaces behind auth middleware. The read-only
    // `/api/creative-studio/files/{id}` asset channel is split into
    // `workshop_public_routes` below because browser media elements cannot send
    // the local-trust header. Both routers share one live service.
    let workshop_authenticated = protect_instance_owner(
        workshop_routes(states.workshop.clone()),
        &auth_mw_state,
        &instance_owner_state,
    );
    let creation_authenticated = protect_instance_owner(
        creation_routes(states.creation),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Knowledge Base platform routes protected by auth middleware
    let knowledge_authenticated = protect_instance_owner(
        knowledge_routes(states.knowledge),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Webhook + tag-settings routes protected by auth middleware
    let webhook_authenticated = protect_instance_owner(
        webhook_routes(states.webhook),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Persistent Agent collaboration routes protected by auth middleware.
    let agent_execution_authenticated = protect_instance_owner(
        agent_execution_routes(states.agent_execution.clone()),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Reusable collaboration inputs are configuration, not a second runtime
    // state machine. They share the same Engine facade and auth boundary.
    let agent_execution_template_authenticated = protect_instance_owner(
        agent_execution_template_routes(states.agent_execution.clone()),
        &auth_mw_state,
        &instance_owner_state,
    );

    // PTY, Office and shell operations all execute in the backend OS account.
    // SQL owner columns cannot sandbox processes sharing that uid.
    let terminal_authenticated = protect_instance_owner(
        terminal_routes(states.terminal),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Office routes protected by auth middleware
    let office_authenticated = protect_instance_owner(
        office_routes(states.office.clone()),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Shell + STT routes protected by auth middleware
    let shell_authenticated = protect_instance_owner(
        shell_routes(states.shell),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Preset catalog and resolver routes protected by auth middleware.
    let preset_authenticated = protect_instance_owner(
        preset_routes(states.preset),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Computer-use OS permission status + prompt (macOS TCC). Stateless: the
    // handlers probe/trigger the host process's own grants. Auth-gated like the
    // other diagnostic endpoints. Registered on every build (handlers degrade to
    // null/no-op off macOS / non-computer-use), so the shared settings UI can
    // always query without a 404.
    let computer_permissions_authenticated = protect_instance_owner(
        Router::new()
            .route("/api/computer/permissions", get(computer_permission_status))
            .route(
                "/api/computer/permissions/request",
                post(request_computer_permission),
            )
            .route(
                "/api/computer/permissions/open-settings",
                post(open_permission_settings),
            ),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Registration templates and status are read-only owner diagnostics.
    let knowledge_registration_read_authenticated = protect_instance_owner(
        Router::new()
            .route(
                "/api/terminals/mcp-register-template",
                get(mcp_register_template_handler),
            )
            .route(
                "/api/terminals/knowledge-global-status",
                get(knowledge_global_status_handler),
            ),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Config and CLI mutations require BOTH the installation owner identity
    // and the per-boot local-desktop trust proof. A remote login, even for the
    // owner account, cannot write files or execute `codex mcp` on the host.
    let knowledge_registration_write_local = protect_instance_owner(
        Router::new()
            .route(
                "/api/terminals/register-knowledge",
                post(register_knowledge_handler),
            )
            .route(
                "/api/terminals/register-knowledge-global",
                post(register_knowledge_global_handler),
            )
            .route(
                "/api/terminals/unregister-knowledge-global",
                post(unregister_knowledge_global_handler),
            )
            .route_layer(middleware::from_fn(require_local_trust_middleware)),
        &auth_mw_state,
        &instance_owner_state,
    );

    // Office iframe GETs cannot carry the app auth header. Authenticated start
    // mints a high-entropy, in-memory session capability in the URL path; these
    // routes accept only that revocable capability and never a caller-owned port.
    let office_proxy = office_proxy_routes(states.office);
    let public_assets = asset_routes(AssetRouterState::default());
    // Figure-image serving — exempt from auth: `<img>`/`new Image()` can't carry
    // the local-trust header, so the desktop webview would 403 every figure
    // thumbnail and the desktop companion would render blank. GET-only, opaque
    // unguessable ids; listing/creation stay authenticated. See `companion_public_routes`.
    let companion_public = companion_public_routes(states.companion);

    // 创意工坊 asset/thumbnail serving — exempt from auth for the same reason as
    // companion figure images: `<img>`/`<video>` subresource loads can't carry
    // the local-trust header, so an authenticated route would 403 every asset
    // preview and canvas gallery thumbnail. GET-only, opaque bare UUIDv7 asset
    // and canvas ids; listing/upload/mutation stay authenticated.
    let workshop_public = workshop_public_routes(states.workshop);

    // 小程序 document serving — exempt from auth for the same reason as the
    // workshop binaries: an `<iframe>` document load can't carry the local-trust
    // header, so an authenticated route would 403 every mini-app the user opens.
    // GET-only, opaque bare UUIDv7 ids; every metadata read and every write stays
    // authenticated.
    let miniapp_public = miniapp_public_routes(states.miniapp);

    // WebSocket upgrade route — exempt from CSRF (no cookie-based
    // double-submit) but still gets security response headers.
    let ws_routes = Router::new()
        .route("/ws", get(ws_upgrade_handler))
        .with_state(ws_state.clone());
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: route groups built"
    );

    // Phase 2b: 「登录我的浏览器」——用户一键拉起可见登录浏览器(共享 profile),登录一次后静默会话复用。
    // 仅 browser-use 构建(需 CDP 引擎);面向桌面(headful 需显示器)。auth 中间件保护(与其它诊断端点同)。
    #[cfg(feature = "browser-use")]
    let browser_login_authenticated = {
        let login_state = crate::router::browser_login::BrowserLoginState::new(
            services.browser_session_hub.clone(),
            services.authoritative_user_id.clone(),
            // Boot-time snapshot source for the effective-source echo (F67);
            // the same store the composition root froze into the Hub's engine
            // template at startup.
            Some(Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
                services.database.pool().clone(),
            ))),
        );
        protect_instance_owner(
            Router::new()
                .route(
                    "/api/browser/login/open",
                    post(crate::router::browser_login::open_browser_login),
                )
                .route(
                    "/api/browser/login/close",
                    post(crate::router::browser_login::close_browser_login),
                )
                .route(
                    "/api/browser/login/status",
                    get(crate::router::browser_login::browser_login_status),
                )
                .with_state(login_state),
            &auth_mw_state,
            &instance_owner_state,
        )
    };

    // Browser inventory and lifecycle management are projections over the
    // process-wide Hub. Page execution remains Agent-only.
    // The state may deliberately carry `None` while a browser-enabled host is
    // degraded; handlers then return a stable 501 and never launch a private
    // fallback engine.
    #[cfg(feature = "browser-use")]
    let (browser_management_user_authenticated, browser_management_owner_authenticated) = {
        let state = crate::router::browser_management::BrowserManagementState::new(
            services.browser_session_hub.clone(),
            Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
                services.database.pool().clone(),
            )),
            services.authoritative_user_id.clone(),
        );
        let user_routes =
            crate::router::browser_management::browser_management_user_routes(state.clone())
                .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));
        let owner_routes = protect_instance_owner(
            crate::router::browser_management::browser_management_owner_routes(state),
            &auth_mw_state,
            &instance_owner_state,
        );
        (user_routes, owner_routes)
    };

    let router = Router::new()
        .route("/health", get(health_check))
        .merge(auth_routes(auth_state))
        .merge(crate::router::instance_token_routes::instance_token_routes(instance_token_state))
        .merge(system_authenticated)
        .merge(computer_permissions_authenticated)
        .merge(knowledge_registration_read_authenticated)
        .merge(knowledge_registration_write_local)
        .merge(conversation_authenticated)
        .merge(creative_studio_agent_session_authenticated)
        .merge(conversation_ops_authenticated)
        .merge(ssh_host_authenticated)
        .merge(miniapp_authenticated)
        .merge(agent_authenticated)
        .merge(model_failover_authenticated)
        .merge(connection_test_authenticated)
        .merge(file_authenticated)
        .merge(mcp_authenticated)
        .merge(extension_authenticated)
        .merge(hub_authenticated);
    let router = match skill_authenticated {
        Some(skill) => router.merge(skill),
        None => router,
    }
        .merge(channel_authenticated)
        .merge(cron_authenticated)
        .merge(requirement_authenticated)
        .merge(idmm_authenticated)
        .merge(companion_authenticated)
        .merge(customer_service_authenticated)
        .merge(workshop_authenticated)
        .merge(creation_authenticated)
        .merge(knowledge_authenticated)
        .merge(webhook_authenticated)
        .merge(agent_execution_authenticated)
        .merge(agent_execution_template_authenticated)
        .merge(terminal_authenticated)
        .merge(office_authenticated)
        .merge(shell_authenticated)
        .merge(preset_authenticated);
    // Robot management face (owner-only), same group and same gates as the SSH
    // host book: the desktop UI is talking, not a device.
    let router = match robot_faces.as_ref() {
        Some(faces) => router.merge(protect_instance_owner(
            faces.admin.clone(),
            &auth_mw_state,
            &instance_owner_state,
        )),
        None => router,
    };

    // Phase 2b: mount the login-browser routes (browser-use builds only).
    #[cfg(feature = "browser-use")]
    let router = router
        .merge(browser_management_user_authenticated)
        .merge(browser_management_owner_authenticated)
        .merge(browser_login_authenticated);

    // CSRF (Double Submit Cookie) protects cookie-authenticated (remote
    // browser) requests. It is skipped entirely under NoAuth, and skips
    // per-request for locally-trusted (header-trusted) requests inside the
    // middleware itself.
    let router = if services.auth_policy.is_no_auth() {
        router
    } else {
        router.layer(middleware::from_fn_with_state(
            services.cookie_config.clone(),
            csrf_middleware,
        ))
    };
    let router = router
    .merge(ws_routes)
    .merge(office_proxy)
    .merge(public_assets)
    .merge(companion_public)
    .merge(workshop_public)
    .merge(miniapp_public);

    // Robot device face. `nest` (not `merge`) scopes it to `/robot`, and it sits
    // in this post-CSRF group on purpose: a robot presents a bearer token minted
    // by its own OTA response and has neither a cookie nor a session, so
    // cookie-CSRF has nothing to protect and would only reject it. It still
    // inherits the security headers, the body limit and the access log below.
    let router = match robot_faces {
        Some(faces) => router.nest("/robot", faces.device),
        None => router,
    };

    let router = router.layer(middleware::from_fn(security_headers_middleware));

    // Raise the default request body limit from axum's 2MB default to
    // `BODY_LIMIT` (10MB). Routes that need a larger cap (e.g. `/api/fs/upload`)
    // disable this default and install their own `RequestBodyLimitLayer`.
    let router = router.layer(DefaultBodyLimit::max(nomifun_common::constants::BODY_LIMIT));

    let router = with_access_log(router);

    // Global, OUTERMOST trust resolution: runs before CSRF and per-route auth so
    // both can read the `LocalTrusted` marker / injected system `CurrentUser`.
    // Under TrustLocalToken the desktop webview's per-boot secret header grants
    // trust; under NoAuth every request is trusted; under Required none is.
    let trust_state = TrustState {
        policy: services.auth_policy,
        local_trust_secret: services.local_trust_secret.clone(),
        authoritative_user_id: services.authoritative_user_id.clone(),
    };
    let router = router.layer(middleware::from_fn_with_state(
        trust_state,
        trust_resolve_middleware,
    ));

    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: route tree build with states completed"
    );

    // Permissive CORS for the desktop's own cross-origin webview (its document
    // origin is `tauri://` / `http://tauri.localhost`, not the loopback port).
    // Safe even on the LAN-bound listener: the trust secret rides a header (not
    // a cookie), so an `Any`-origin attacker page can neither read it nor read
    // cross-origin responses. Remote browsers are served same-origin and do not
    // rely on CORS.
    if services.auth_policy.allows_local_webview() {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers(Any);
        router.layer(cors)
    } else {
        router
    }
}
