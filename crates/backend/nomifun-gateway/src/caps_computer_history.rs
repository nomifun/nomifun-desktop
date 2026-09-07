//! Computer-history capability domain (registry form): the recorder status,
//! activity-segment reads, per-app usage rollup, the master toggle and a
//! destructive purge.
//!
//! Read tools surface the user's locally observed activity; the toggle and
//! purge are `Write` / `Destructive` and the purge is denied on Channel so a
//! remote IM caller can never wipe local history.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::deps::GatewayDeps;
use crate::registry::{Capability, CapabilityMeta, DangerTier, Surface};
use crate::server::ok;

// ── param structs (single source: schema + runtime) ──────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatusParams {}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListParams {
    /// Window start, epoch milliseconds (defaults to 24 hours ago).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from_ms: Option<i64>,
    /// Window end, epoch milliseconds (defaults to now).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to_ms: Option<i64>,
    /// Max segments to return (1..=200, default 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AppUsageParams {
    /// Window start, epoch milliseconds (defaults to 24 hours ago).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from_ms: Option<i64>,
    /// Window end, epoch milliseconds (defaults to now).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to_ms: Option<i64>,
    /// Max rows to return (1..=100, default 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UrlsParams {
    /// Window start, epoch milliseconds (defaults to 24 hours ago).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from_ms: Option<i64>,
    /// Window end, epoch milliseconds (defaults to now).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to_ms: Option<i64>,
    /// Optional substring filter matched against the captured URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url_contains: Option<String>,
    /// Max rows to return (1..=100, default 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetEnabledParams {
    /// true starts capture (only when the macOS Accessibility permission is
    /// granted), false stops it and leaves stored history intact.
    enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PurgeParams {
    /// Delete segments that ended before this epoch-ms timestamp. Omit to
    /// purge the entire history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetSettingsParams {}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateSettingsParams {
    /// Full replacement settings object, identical in shape to
    /// `computer_history_get_settings`'s `result`.
    settings: Value,
}

// ── response shaping (contract shared with the settings UI) ──────────────

fn permission_wire(
    permission: nomifun_computer_history::PermissionState,
) -> &'static str {
    use nomifun_computer_history::PermissionState as P;
    match permission {
        P::Granted => "granted",
        P::Denied => "denied",
        P::NotRequired => "granted",
        P::Unknown => "unknown",
    }
}

fn state_wire(state: nomifun_computer_history::RecorderState) -> &'static str {
    use nomifun_computer_history::RecorderState as S;
    match state {
        S::Stopped => "stopped",
        S::Running => "running",
        S::Paused => "paused",
    }
}

fn paused_until_wire(status: &nomifun_computer_history::ServiceStatus) -> Option<String> {
    use chrono::TimeZone;
    status
        .paused_until_ms
        .and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single())
        .map(|dt| dt.to_rfc3339())
}

// ── handlers ──────────────────────────────────────────────────────────────

/// `None` when the feature-local store failed to open this boot: report a
/// disabled/unknown recorder rather than a tool error, so the settings page
/// renders the degraded state instead of a failure.
fn unavailable() -> Value {
    json!({
        "result": {
            "enabled": false,
            "state": "stopped",
            "permission": "unknown",
            "paused_until": null,
            "storage": {"segments": 0, "approx_bytes": 0, "path": null}
        }
    })
}

async fn status(deps: Arc<GatewayDeps>, _p: StatusParams) -> Value {
    let Some(service) = deps.computer_history.as_ref() else {
        return unavailable();
    };
    match service.status().await {
        Ok(status) => {
            let chat_analytics = match service.chat_analytics_status().await {
                Some(chat) => json!({ "available": chat.available, "db_path": chat.db_path }),
                None => json!({ "available": false, "db_path": null }),
            };
            json!({
                "result": {
                    "enabled": status.enabled,
                    "state": state_wire(status.state),
                    "permission": permission_wire(status.permission),
                    "paused_until": paused_until_wire(&status),
                    "storage": {
                        "segments": status.storage.segment_count,
                        "approx_bytes": status.storage.db_bytes + status.storage.segments_dir_bytes,
                        "path": status.event_stream_root_path,
                    },
                    "chat_analytics": chat_analytics
                }
            })
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn filter_from(from_ms: Option<i64>, to_ms: Option<i64>, limit: Option<u32>) -> nomifun_computer_history::SegmentFilter {
    nomifun_computer_history::SegmentFilter {
        from_ms,
        to_ms,
        app_name: None,
        url_contains: None,
        limit: limit.unwrap_or(0),
    }
}

async fn list(deps: Arc<GatewayDeps>, p: ListParams) -> Value {
    let Some(service) = deps.computer_history.as_ref() else {
        return ok(Vec::<Value>::new());
    };
    let filter = filter_from(p.from_ms, p.to_ms, p.limit);
    match service.store().query_segments(&filter).await {
        Ok(segments) => {
            let rows: Vec<Value> = segments
                .into_iter()
                .map(|segment| {
                    json!({
                        "event_id": segment.event_id,
                        "app_name": segment.app_name,
                        "window_title": segment.window_title,
                        "browser_url": segment.browser_url,
                        "started_at_ms": segment.started_at_ms,
                        "ended_at_ms": segment.ended_at_ms,
                        "source": segment.source,
                    })
                })
                .collect();
            ok(rows)
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn app_usage(deps: Arc<GatewayDeps>, p: AppUsageParams) -> Value {
    let Some(service) = deps.computer_history.as_ref() else {
        return ok(Vec::<Value>::new());
    };
    let filter = filter_from(p.from_ms, p.to_ms, p.limit);
    match service.store().app_usage(&filter).await {
        Ok(rows) => ok(rows),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn urls(deps: Arc<GatewayDeps>, p: UrlsParams) -> Value {
    let Some(service) = deps.computer_history.as_ref() else {
        return ok(Vec::<Value>::new());
    };
    let mut filter = filter_from(p.from_ms, p.to_ms, p.limit);
    filter.url_contains = p.url_contains;
    match service.store().url_history(&filter).await {
        Ok(rows) => ok(rows),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn set_enabled(deps: Arc<GatewayDeps>, p: SetEnabledParams) -> Value {
    let Some(service) = deps.computer_history.as_ref() else {
        return json!({ "error": "computer history service is unavailable" });
    };
    match service.set_enabled(p.enabled).await {
        Ok(()) => json!({ "result": { "ok": true, "enabled": p.enabled } }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn purge(deps: Arc<GatewayDeps>, p: PurgeParams) -> Value {
    let Some(service) = deps.computer_history.as_ref() else {
        return json!({ "error": "computer history service is unavailable" });
    };
    let now = nomifun_common::now_ms();
    let result = match p.before_ms {
        Some(before_ms) => service.store().purge_before(before_ms.min(now)).await,
        None => service.store().purge_all().await,
    };
    match result {
        Ok(deleted) => json!({ "result": { "deleted": deleted } }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn get_settings(deps: Arc<GatewayDeps>, _p: GetSettingsParams) -> Value {
    let Some(service) = deps.computer_history.as_ref() else {
        return json!({ "error": "computer history service is unavailable" });
    };
    match service.observation_settings().await {
        Ok(settings) => ok(settings),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn update_settings(deps: Arc<GatewayDeps>, p: UpdateSettingsParams) -> Value {
    let Some(service) = deps.computer_history.as_ref() else {
        return json!({ "error": "computer history service is unavailable" });
    };
    let settings: nomifun_computer_history::ObservationSettings = match serde_json::from_value(p.settings) {
        Ok(settings) => settings,
        Err(error) => return json!({ "error": format!("invalid observation settings: {error}") }),
    };
    match service.update_observation_settings(&settings).await {
        Ok(()) => ok(settings),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ── registration ─────────────────────────────────────────────────────────

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<StatusParams, _, _>(
        CapabilityMeta::new(
            "computer_history_status",
            "computer_history",
            "Get computer-history capture status: enabled, recorder state, macOS permission state, storage usage and the segment-store path.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| status(deps, p),
    ));
    out.push(Capability::new::<ListParams, _, _>(
        CapabilityMeta::new(
            "computer_history_list",
            "computer_history",
            "List activity segments inside a time window (app, window title, browser URL, time range), newest first.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| list(deps, p),
    ));
    out.push(Capability::new::<AppUsageParams, _, _>(
        CapabilityMeta::new(
            "computer_history_app_usage",
            "computer_history",
            "Aggregate foreground time per application inside a time window, ranked by usage.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| app_usage(deps, p),
    ));
    out.push(Capability::new::<UrlsParams, _, _>(
        CapabilityMeta::new(
            "computer_history_urls",
            "computer_history",
            "Aggregate browsing time per URL inside a time window, optionally filtered by a URL substring.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| urls(deps, p),
    ));
    out.push(Capability::new::<SetEnabledParams, _, _>(
        CapabilityMeta::new(
            "computer_history_set_enabled",
            "computer_history",
            "Enable or disable computer-history capture. Enabling starts the observer only when the macOS Accessibility permission is granted; disabling stops capture but keeps stored history.",
            DangerTier::Write,
        ),
        |deps, _ctx, p| set_enabled(deps, p),
    ));
    out.push(Capability::new::<GetSettingsParams, _, _>(
        CapabilityMeta::new(
            "computer_history_get_settings",
            "computer_history",
            "Read the observation settings (app/URL defaults, allowlist/blocklist, private-browsing handling).",
            DangerTier::Read,
        ),
        |deps, _ctx, p| get_settings(deps, p),
    ));
    out.push(Capability::new::<UpdateSettingsParams, _, _>(
        CapabilityMeta::new(
            "computer_history_update_settings",
            "computer_history",
            "Replace the whole observation settings object. Fetch first with computer_history_get_settings and re-send unchanged fields.",
            DangerTier::Write,
        ),
        |deps, _ctx, p| update_settings(deps, p),
    ));
    out.push(Capability::new::<PurgeParams, _, _>(
        CapabilityMeta::new(
            "computer_history_purge",
            "computer_history",
            "Permanently delete stored activity segments (optionally only those ended before a timestamp). Irreversible.",
            DangerTier::Destructive,
        )
        .deny_on(&[Surface::Channel]),
        |deps, _ctx, p| purge(deps, p),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_params_reject_unknown_fields() {
        assert!(serde_json::from_value::<StatusParams>(json!({})).is_ok());
        assert!(serde_json::from_value::<StatusParams>(json!({"nope": 1})).is_err());
    }

    #[test]
    fn list_params_accept_the_documented_window_shape() {
        let parsed: ListParams = serde_json::from_value(json!({
            "from_ms": 1000, "to_ms": 2000, "limit": 10
        }))
        .unwrap();
        assert_eq!(parsed.from_ms, Some(1000));
        assert_eq!(parsed.to_ms, Some(2000));
        assert_eq!(parsed.limit, Some(10));
        assert!(serde_json::from_value::<ListParams>(json!({"limit": "10"})).is_err());
    }

    #[test]
    fn set_enabled_requires_explicit_boolean() {
        assert!(
            serde_json::from_value::<SetEnabledParams>(json!({"enabled": true}))
                .is_ok()
        );
        assert!(serde_json::from_value::<SetEnabledParams>(json!({})).is_err());
    }

    #[test]
    fn update_settings_params_reject_missing_settings() {
        // `settings` is required; its inner shape is validated at the handler
        // boundary (`serde_json::from_value::<ObservationSettings>`), not at
        // the param boundary (opaque Value passthrough).
        assert!(serde_json::from_value::<UpdateSettingsParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<UpdateSettingsParams>(json!({"settings": {"any": "shape"}}))
                .is_ok()
        );
    }

    #[test]
    fn observation_settings_roundtrip_rejects_unknown_fields() {
        // The handler-level contract the test above points at.
        assert!(
            serde_json::from_value::<nomifun_computer_history::ObservationSettings>(json!({
                "nope": 1
            }))
            .is_err()
        );
        let parsed: nomifun_computer_history::ObservationSettings =
            serde_json::from_value(json!({
                "defaultApplicationBehavior": "observe",
                "defaultURLBehavior": "observe"
            }))
            .unwrap();
        assert!(!parsed.observe_private_browsing);
    }

    #[tokio::test]
    async fn status_reports_disabled_when_service_unavailable() {
        let payload = unavailable();
        assert_eq!(payload["result"]["enabled"], false);
        assert_eq!(payload["result"]["state"], "stopped");
        assert_eq!(payload["result"]["permission"], "unknown");
        // Exactly one of result/error (registry envelope contract).
        assert!(payload.get("error").is_none());
    }

    #[test]
    fn wire_strings_match_the_ui_contract() {
        use nomifun_computer_history::{PermissionState, RecorderState};
        assert_eq!(state_wire(RecorderState::Stopped), "stopped");
        assert_eq!(state_wire(RecorderState::Running), "running");
        assert_eq!(state_wire(RecorderState::Paused), "paused");
        assert_eq!(permission_wire(PermissionState::Granted), "granted");
        assert_eq!(permission_wire(PermissionState::Denied), "denied");
        assert_eq!(permission_wire(PermissionState::NotRequired), "granted");
        assert_eq!(permission_wire(PermissionState::Unknown), "unknown");
    }

    #[test]
    fn every_computer_history_tool_is_registered() {
        use crate::registry::Registry;
        let specs = Registry::global().tool_specs(Surface::Desktop);
        for name in [
            "computer_history_status",
            "computer_history_list",
            "computer_history_app_usage",
            "computer_history_urls",
            "computer_history_set_enabled",
            "computer_history_get_settings",
            "computer_history_update_settings",
            "computer_history_purge",
        ] {
            assert!(
                specs.iter().any(|spec| spec.name == name),
                "missing {name}"
            );
        }
        // No retired nomi_* prefixed spelling may appear.
        assert!(!specs.iter().any(|spec| spec.name == "nomi_computer_history_status"));
    }

    #[test]
    fn purge_schema_is_closed_and_optional_before_ms() {
        use crate::registry::Registry;
        let specs = Registry::global().tool_specs(Surface::Desktop);
        let spec = specs
            .iter()
            .find(|spec| spec.name == "computer_history_purge")
            .expect("purge registered");
        let properties = spec.input_schema.get("properties").expect("properties");
        assert!(properties.get("before_ms").is_some());
        assert!(spec.input_schema.get("additionalProperties").is_none_or(|v| v != &json!(true)));
        assert!(serde_json::from_value::<PurgeParams>(json!({})).is_ok());
        assert!(serde_json::from_value::<PurgeParams>(json!({"before_ms": 123})).is_ok());
        assert!(serde_json::from_value::<PurgeParams>(json!({"extra": 1})).is_err());
    }
}
