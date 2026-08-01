//! Shared builder for the `browser.inventory.changed` resync frame.
//!
//! This is an external WS contract emitted from two independent paths — the
//! realtime forwarder's lag branch (`services.rs`) and the coalesced lag
//! resync broadcaster (`router/routes.rs`) — so the frame is built in exactly
//! one place to keep the wire shape from drifting.
//!
//! Deliberately not feature-gated: the coalescer path exists in every build.

pub(crate) const BROWSER_INVENTORY_EVENT_NAME: &str = "browser.inventory.changed";
pub(crate) const BROWSER_INVENTORY_RESYNC_CHANGE_KIND: &str = "resync_required";

/// Build a protocol-compatible invalidation event for a lossy inventory hop.
///
/// This intentionally carries no synthetic `sequence`: only the Hub owns that
/// counter, and inventing one here could hide a real gap. Existing clients
/// already refresh on every inventory event; newer clients can use the
/// additive marker to explicitly classify the refresh as a full resync.
pub(crate) fn browser_inventory_resync_event(
    skipped: u64,
) -> nomifun_api_types::WebSocketMessage<serde_json::Value> {
    nomifun_api_types::WebSocketMessage::new(
        BROWSER_INVENTORY_EVENT_NAME,
        serde_json::json!({
            "change_kind": BROWSER_INVENTORY_RESYNC_CHANGE_KIND,
            "resync_required": true,
            "skipped": skipped,
        }),
    )
}
