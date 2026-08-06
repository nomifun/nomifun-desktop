//! The device face. Mounted with `nest("/robot", ...)` **outside** the session
//! auth layers — a robot has no cookie and no session.
//!
//! The OTA response is the only channel that configures the firmware's server
//! address, and it must obey two firmware rules absolutely: always include
//! `websocket`, never include `mqtt` (any `mqtt` object makes the firmware pick
//! MQTT with no fallback path).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use serde_json::json;

use crate::dto::DeviceReportBody;
use crate::endpoint::EndpointAdvertiser;
use crate::registry::{RobotRecord, RobotRegistry, RobotReport};

/// Message shown/spoken by the device while waiting to be claimed.
const ACTIVATION_MESSAGE: &str = "请在 nomifun 中输入此码绑定伙伴";
/// How long the firmware waits between activation polls, in milliseconds.
const ACTIVATION_TIMEOUT_MS: i64 = 30_000;

/// Shared state of the device face.
#[derive(Clone)]
pub struct RobotDeviceState {
    pub registry: Arc<RobotRegistry>,
    pub advertiser: Arc<dyn EndpointAdvertiser>,
}

/// Router for the device face, to be nested under `/robot`.
pub fn device_router(state: RobotDeviceState) -> Router {
    Router::new()
        .route("/ota", post(ota_report).get(ota_report_get))
        .route("/ota/activate", post(activate))
        .with_state(state)
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Peer address of a device request, when the listener supplies one.
///
/// `ConnectInfo<T>` itself is a *required* extractor in axum 0.8 (it has no
/// `OptionalFromRequestParts` impl), so the optional form goes through the
/// extension it is stored in. This matters because the LAN listener is served
/// with `into_make_service_with_connect_info::<SocketAddr>()` but the loopback
/// listener is not, and a robot report must never 500 over a missing peer.
type MaybePeer = Option<Extension<ConnectInfo<SocketAddr>>>;

/// Peer IP of a device request, or `0.0.0.0` when unknown — that matches no
/// interface prefix, so the advertiser falls back to the first detected one.
fn peer_ip(peer: MaybePeer) -> IpAddr {
    peer.map(|Extension(ConnectInfo(addr))| addr.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn local_timezone_offset_minutes() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

/// Build the OTA response body.
///
/// `firmware.version` deliberately echoes the device's own version so the
/// firmware concludes it is up to date — hosting firmware images is a
/// non-goal (spec §1), and `url` stays an empty string so the firmware's
/// parse of that field is still safe.
pub fn build_ota_response(
    record: &RobotRecord,
    token: &str,
    ws_url: Option<&str>,
    now_ms: i64,
    tz_offset_minutes: i32,
) -> serde_json::Value {
    let mut body = json!({
        "websocket": {
            "url": ws_url.unwrap_or_default(),
            "token": token,
            "version": 1,
        },
        "server_time": {
            "timestamp": now_ms,
            "timezone_offset": tz_offset_minutes,
        },
        "firmware": {
            "version": record.firmware_version,
            "url": "",
        },
    });
    if let Some(code) = &record.activation_code {
        body["activation"] = json!({
            "code": code,
            "message": ACTIVATION_MESSAGE,
            "timeout_ms": ACTIVATION_TIMEOUT_MS,
        });
    }
    body
}

fn report_from(headers: &HeaderMap, body: &DeviceReportBody) -> Option<RobotReport> {
    let robot_id = header(headers, "device-id").or_else(|| body.mac_address.clone())?;
    let client_id = header(headers, "client-id")
        .or_else(|| body.uuid.clone())
        .unwrap_or_default();
    Some(RobotReport {
        robot_id,
        client_id,
        board: body
            .board
            .board_type
            .clone()
            .unwrap_or_else(|| "unknown".to_owned()),
        firmware_version: body
            .application
            .version
            .clone()
            .unwrap_or_else(|| "0.0.0".to_owned()),
    })
}

async fn ota_report(
    State(state): State<RobotDeviceState>,
    peer: MaybePeer,
    headers: HeaderMap,
    body: Option<Json<DeviceReportBody>>,
) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let Some(report) = report_from(&headers, &body) else {
        return (StatusCode::BAD_REQUEST, "missing Device-Id").into_response();
    };
    let robot_id = report.robot_id.clone();
    let (record, token) = match state.registry.upsert_on_report(report, now_ms()).await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(%robot_id, %error, "robot: OTA upsert failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "registry write failed").into_response();
        }
    };
    let ws_url = state.advertiser.websocket_url(peer_ip(peer));
    tracing::info!(
        robot_id = %record.robot_id,
        board = %record.board,
        bound = record.companion_id.is_some(),
        reachable = ws_url.is_some(),
        "robot: OTA report"
    );
    Json(build_ota_response(
        &record,
        &token,
        ws_url.as_deref(),
        now_ms(),
        local_timezone_offset_minutes(),
    ))
    .into_response()
}

/// The firmware uses GET when its report body is empty.
async fn ota_report_get(
    state: State<RobotDeviceState>,
    peer: MaybePeer,
    headers: HeaderMap,
) -> Response {
    ota_report(state, peer, headers, None).await
}

async fn activate(State(state): State<RobotDeviceState>, headers: HeaderMap) -> Response {
    let Some(robot_id) = header(&headers, "device-id") else {
        return (StatusCode::BAD_REQUEST, "missing Device-Id").into_response();
    };
    let bound = state
        .registry
        .list()
        .await
        .into_iter()
        .find(|r| r.robot_id == robot_id)
        .and_then(|r| r.companion_id);
    if bound.is_some() {
        StatusCode::OK.into_response()
    } else {
        // 202 tells the firmware "still waiting for the user"; it keeps polling.
        StatusCode::ACCEPTED.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::{EndpointAdvertiser, LanAdvertiser, LanEndpointSnapshot};
    use crate::registry::{RobotRegistry, RobotReport};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use tower::ServiceExt;

    const REPORT_BODY: &str = r#"{
        "version": 2,
        "mac_address": "aa:bb:cc:dd:ee:ff",
        "uuid": "3f2b9c1e-0000-4000-8000-000000000001",
        "application": { "name": "xiaozhi", "version": "1.9.0" },
        "board": { "type": "esp32-s3n16r8-emoji", "name": "ESP32-S3N16R8-EMOJI" }
    }"#;

    fn advertiser(enabled: bool) -> Arc<dyn EndpointAdvertiser> {
        let (tx, rx) = tokio::sync::watch::channel(LanEndpointSnapshot {
            enabled,
            port: 25808,
            ipv4s: vec![Ipv4Addr::new(192, 168, 1, 20)],
        });
        // Keep the sender alive for the life of the receiver.
        std::mem::forget(tx);
        Arc::new(LanAdvertiser::new(rx))
    }

    async fn state(enabled: bool) -> (RobotDeviceState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        (
            RobotDeviceState {
                registry,
                advertiser: advertiser(enabled),
            },
            dir,
        )
    }

    #[tokio::test]
    async fn ota_response_never_contains_mqtt_and_always_contains_websocket() {
        let (state, _dir) = state(true).await;
        let app = device_router(state);

        let response = app
            .oneshot(
                Request::post("/ota")
                    .header("Device-Id", "aa:bb:cc:dd:ee:ff")
                    .header("Client-Id", "3f2b9c1e-0000-4000-8000-000000000001")
                    .header("content-type", "application/json")
                    .body(Body::from(REPORT_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(
            value.get("mqtt").is_none(),
            "an mqtt object makes the firmware pick MQTT with no fallback"
        );
        let ws = value.get("websocket").expect("websocket is mandatory");
        assert_eq!(ws["url"], "ws://192.168.1.20:25808/robot/v1");
        assert_eq!(ws["version"], 1);
        assert_eq!(ws["token"].as_str().unwrap().len(), 64);
        assert!(value["server_time"]["timestamp"].is_i64());
        // Unbound device gets an activation code.
        assert_eq!(value["activation"]["code"].as_str().unwrap().len(), 6);
        assert!(value["activation"]["message"].is_string());
        assert_eq!(value["activation"]["timeout_ms"], 30000);
    }

    #[tokio::test]
    async fn bound_device_gets_no_activation_section() {
        let (state, _dir) = state(true).await;
        let (record, _) = state
            .registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:ff".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();
        state
            .registry
            .claim(
                record.activation_code.as_deref().unwrap(),
                "0190f5fe-7c00-7a00-8000-0000000000aa",
            )
            .await
            .unwrap();

        let app = device_router(state);
        let response = app
            .oneshot(
                Request::post("/ota")
                    .header("Device-Id", "aa:bb:cc:dd:ee:ff")
                    .header("Client-Id", "cid")
                    .header("content-type", "application/json")
                    .body(Body::from(REPORT_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("activation").is_none());
    }

    #[tokio::test]
    async fn activate_returns_202_until_bound_then_200() {
        let (state, _dir) = state(true).await;
        let registry = state.registry.clone();
        let (record, _) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:01".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();

        let pending = device_router(state.clone())
            .oneshot(
                Request::post("/ota/activate")
                    .header("Device-Id", "aa:bb:cc:dd:ee:01")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            pending.status(),
            StatusCode::ACCEPTED,
            "202 = still waiting for the user"
        );

        registry
            .claim(
                record.activation_code.as_deref().unwrap(),
                "0190f5fe-7c00-7a00-8000-0000000000aa",
            )
            .await
            .unwrap();

        let done = device_router(state)
            .oneshot(
                Request::post("/ota/activate")
                    .header("Device-Id", "aa:bb:cc:dd:ee:01")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(done.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn ota_still_answers_when_lan_is_off_but_omits_the_url() {
        let (state, _dir) = state(false).await;
        let app = device_router(state);
        let response = app
            .oneshot(
                Request::post("/ota")
                    .header("Device-Id", "aa:bb:cc:dd:ee:02")
                    .header("Client-Id", "cid")
                    .header("content-type", "application/json")
                    .body(Body::from(REPORT_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("mqtt").is_none(), "still never mqtt");
        assert_eq!(
            value["websocket"]["url"], "",
            "empty url keeps the websocket object present"
        );
    }

    #[test]
    fn build_ota_response_shape_is_stable() {
        let record = crate::registry::RobotRecord {
            robot_id: "aa:bb:cc:dd:ee:ff".into(),
            client_id: "cid".into(),
            name: "表情机器人".into(),
            companion_id: None,
            token_hash: "hash".into(),
            activation_code: Some("483920".into()),
            board: "esp32-s3n16r8-emoji".into(),
            firmware_version: "1.9.0".into(),
            last_seen: Some(1),
            created_at: 1,
        };
        let value = build_ota_response(
            &record,
            "tok",
            Some("ws://x/robot/v1"),
            1_700_000_000_000,
            480,
        );
        assert_eq!(value["websocket"]["token"], "tok");
        assert_eq!(value["server_time"]["timezone_offset"], 480);
        assert_eq!(
            value["firmware"]["version"], "1.9.0",
            "echo the device's own version: no upgrade"
        );
        assert_eq!(value["firmware"]["url"], "");
    }
}
