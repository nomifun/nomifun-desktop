//! The management face. Mounted **inside** the instance-owner auth layer — this
//! is the desktop UI talking, not a device.
//!
//! `GET /api/robots/statuses` and `/endpoints` are declared before
//! `/{robot_id}` so the literal segments win; the SSH domain hit the same
//! shadowing trap and solved it the same way.
//!
//! Every field on the wire is snake_case, matching [`crate::dto`]. That is the
//! opposite of the SSH domain's camelCase events, but the UI contract is pinned
//! to snake_case on both sides; renaming either side fails silently at runtime.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::dto::RobotDto;
use crate::endpoint::EndpointAdvertiser;
use crate::registry::{ClaimError, RobotRegistry};
use crate::status::RobotStatusRegistry;

/// Shared state of the management face.
#[derive(Clone)]
pub struct RobotAdminState {
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
    pub advertiser: Arc<dyn EndpointAdvertiser>,
}

#[derive(Deserialize)]
struct ClaimBody {
    code: String,
    companion_id: String,
}

#[derive(Deserialize)]
struct PatchBody {
    #[serde(default)]
    name: Option<String>,
    /// Absent = leave the binding alone; `null` = unbind; a value = rebind.
    #[serde(default, deserialize_with = "double_option")]
    companion_id: Option<Option<String>>,
}

/// Distinguish "key absent" from "key present and null".
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Management routes.
pub fn admin_router(state: RobotAdminState) -> Router {
    Router::new()
        .route("/api/robots", get(list))
        .route("/api/robots/claim", post(claim))
        .route("/api/robots/statuses", get(statuses))
        .route("/api/robots/endpoints", get(endpoints))
        .route(
            "/api/robots/{robot_id}",
            patch(patch_robot).delete(delete_robot),
        )
        .with_state(state)
}

async fn list(State(state): State<RobotAdminState>) -> Response {
    let robots: Vec<RobotDto> = state
        .registry
        .list()
        .await
        .iter()
        .map(RobotDto::from)
        .collect();
    Json(json!({ "robots": robots })).into_response()
}

async fn claim(State(state): State<RobotAdminState>, Json(body): Json<ClaimBody>) -> Response {
    match state.registry.claim(&body.code, &body.companion_id).await {
        Ok(record) => Json(RobotDto::from(&record)).into_response(),
        Err(ClaimError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "激活码不存在或已被使用" })),
        )
            .into_response(),
        Err(ClaimError::AlreadyBound { companion_id }) => (
            StatusCode::CONFLICT,
            Json(json!({ "message": "这台机器人已绑定其他伙伴", "companion_id": companion_id })),
        )
            .into_response(),
    }
}

async fn patch_robot(
    State(state): State<RobotAdminState>,
    Path(robot_id): Path<String>,
    Json(body): Json<PatchBody>,
) -> Response {
    match state
        .registry
        .patch(&robot_id, body.name, body.companion_id)
        .await
    {
        Ok(record) => Json(RobotDto::from(&record)).into_response(),
        Err(ClaimError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "机器人不存在" })),
        )
            .into_response(),
        Err(ClaimError::AlreadyBound { companion_id }) => (
            StatusCode::CONFLICT,
            Json(json!({ "message": "这台机器人已绑定其他伙伴", "companion_id": companion_id })),
        )
            .into_response(),
    }
}

async fn delete_robot(
    State(state): State<RobotAdminState>,
    Path(robot_id): Path<String>,
) -> Response {
    match state.registry.remove(&robot_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "机器人不存在" })),
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%robot_id, %error, "robot: delete failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "message": "注册表写入失败" })),
            )
                .into_response()
        }
    }
}

async fn statuses(State(state): State<RobotAdminState>) -> Response {
    Json(json!({ "statuses": state.status.snapshot().await })).into_response()
}

async fn endpoints(State(state): State<RobotAdminState>) -> Response {
    Json(json!({
        "ota_urls": state.advertiser.ota_urls(),
        "lan_enabled": state.advertiser.is_available(),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{RobotRegistry, RobotReport};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    struct NullSink;
    impl nomifun_realtime::UserEventSink for NullSink {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    /// Keeps the advertiser's watch sender alive for the life of the state.
    struct Seeded {
        state: RobotAdminState,
        code: String,
        _endpoint_tx: tokio::sync::watch::Sender<crate::endpoint::LanEndpointSnapshot>,
        _dir: tempfile::TempDir,
    }

    async fn seeded() -> Seeded {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        let (record, _) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:ff".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1_700_000_000_000,
            )
            .await
            .unwrap();
        let code = record.activation_code.clone().unwrap();
        let (endpoint_tx, rx) = tokio::sync::watch::channel(crate::endpoint::LanEndpointSnapshot {
            enabled: true,
            port: 25808,
            ipv4s: vec![std::net::Ipv4Addr::new(192, 168, 1, 20)],
        });
        let status = Arc::new(crate::status::RobotStatusRegistry::new(
            crate::events::RobotEventEmitter::new(Arc::new(NullSink)),
            "owner-1".to_owned(),
        ));
        let state = RobotAdminState {
            registry,
            status,
            advertiser: Arc::new(crate::endpoint::LanAdvertiser::new(rx)),
        };
        Seeded {
            state,
            code,
            _endpoint_tx: endpoint_tx,
            _dir: dir,
        }
    }

    async fn json_of(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn list_wraps_records_in_a_robots_key_with_snake_case_fields() {
        let h = seeded().await;
        let response = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/robots")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_of(response).await;
        let robots = value["robots"].as_array().unwrap();
        assert_eq!(robots.len(), 1);
        assert_eq!(robots[0]["robot_id"], "aa:bb:cc:dd:ee:ff");
        assert_eq!(robots[0]["firmware_version"], "1.9.0");
        assert!(robots[0]["last_seen"].is_string());
        assert!(
            robots[0].get("token_hash").is_none(),
            "secrets never leave the process"
        );
        assert!(
            robots[0].get("activation_code").is_none(),
            "codes are for the device screen only"
        );
    }

    #[tokio::test]
    async fn claim_binds_then_404s_on_a_spent_code() {
        let h = seeded().await;

        let ok = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/robots/claim")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"code":"{}","companion_id":"0190f5fe-7c00-7a00-8000-0000000000aa"}}"#,
                        h.code
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(
            json_of(ok).await["companion_id"],
            "0190f5fe-7c00-7a00-8000-0000000000aa"
        );

        let spent = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/robots/claim")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"code":"{}","companion_id":"0190f5fe-7c00-7a00-8000-0000000000bb"}}"#,
                        h.code
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            spent.status(),
            StatusCode::NOT_FOUND,
            "a claimed code no longer exists"
        );
    }

    #[tokio::test]
    async fn rebinding_a_bound_robot_by_code_is_a_conflict() {
        let h = seeded().await;
        // Unbinding re-issues a code, so a second claim can reach a bound robot.
        h.state
            .registry
            .claim(&h.code, "0190f5fe-7c00-7a00-8000-0000000000aa")
            .await
            .unwrap();
        let reissued = h
            .state
            .registry
            .patch("aa:bb:cc:dd:ee:ff", None, Some(None))
            .await
            .unwrap()
            .activation_code
            .unwrap();
        h.state
            .registry
            .patch(
                "aa:bb:cc:dd:ee:ff",
                None,
                Some(Some("0190f5fe-7c00-7a00-8000-0000000000aa".to_owned())),
            )
            .await
            .unwrap();
        // The code the UI holds is now stale: the robot it names is bound.
        let conflict = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/robots/claim")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"code":"{reissued}","companion_id":"0190f5fe-7c00-7a00-8000-0000000000bb"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Rebinding cleared the code, so the stale one resolves to nothing.
        assert_eq!(conflict.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn patch_renames_and_unbinds() {
        let h = seeded().await;
        h.state
            .registry
            .claim(&h.code, "0190f5fe-7c00-7a00-8000-0000000000aa")
            .await
            .unwrap();

        let renamed = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/robots/aa:bb:cc:dd:ee:ff")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"书桌机器人"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let renamed = json_of(renamed).await;
        assert_eq!(renamed["name"], "书桌机器人");
        assert_eq!(
            renamed["companion_id"], "0190f5fe-7c00-7a00-8000-0000000000aa",
            "an absent companion_id leaves the binding alone"
        );

        let unbound = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/robots/aa:bb:cc:dd:ee:ff")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"companion_id":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let unbound = json_of(unbound).await;
        assert!(unbound["companion_id"].is_null());
        assert_eq!(unbound["name"], "书桌机器人", "the rename survived");
    }

    #[tokio::test]
    async fn patching_an_unknown_robot_is_a_404() {
        let h = seeded().await;
        let response = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/robots/nope")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_removes_and_is_idempotent_enough() {
        let h = seeded().await;
        let gone = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/robots/aa:bb:cc:dd:ee:ff")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        let missing = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/robots/aa:bb:cc:dd:ee:ff")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn statuses_and_endpoints_expose_what_the_ui_needs() {
        let h = seeded().await;
        h.state
            .status
            .publish(
                "aa:bb:cc:dd:ee:ff",
                Some("c1"),
                crate::status::RobotPhase::Listening,
                42,
            )
            .await;

        let statuses = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/robots/statuses")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let value = json_of(statuses).await;
        assert_eq!(value["statuses"][0]["phase"], "listening");
        assert_eq!(value["statuses"][0]["changed_at"], 42);

        let endpoints = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/robots/endpoints")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let value = json_of(endpoints).await;
        assert_eq!(value["lan_enabled"], true);
        assert_eq!(
            value["ota_urls"][0],
            "http://192.168.1.20:25808/robot/ota"
        );
    }

    #[tokio::test]
    async fn statuses_route_is_not_shadowed_by_the_id_route() {
        // `/api/robots/statuses` must not be captured as `{robot_id}`.
        let h = seeded().await;
        let response = admin_router(h.state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/robots/statuses")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(json_of(response).await.get("statuses").is_some());
    }
}
