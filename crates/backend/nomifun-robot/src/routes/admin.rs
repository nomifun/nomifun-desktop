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
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::dto::RobotDto;
use crate::endpoint::EndpointAdvertiser;
use crate::registry::{ClaimError, RobotRegistry};
use crate::status::RobotStatusRegistry;

/// Shared state of the management face.
#[derive(Clone)]
pub struct RobotAdminState {
    pub tools: Arc<crate::tool_registry::RobotToolRegistry>,
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
    pub advertiser: Arc<dyn EndpointAdvertiser>,
    pub playback_source: Option<Arc<dyn crate::services::RobotPlaybackSource>>,
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
        .route("/api/robots/{robot_id}/permissions", patch(set_permissions))
        .route("/api/robots/{robot_id}/capabilities", get(capabilities).put(set_capabilities))
        .route("/api/robots/{robot_id}/capabilities/refresh", post(refresh_capabilities))
        .route("/api/robots/{robot_id}/speak", post(speak_response))
        .route(
            "/api/robots/{robot_id}",
            patch(patch_robot).delete(delete_robot),
        )
        .with_state(state)
}

async fn list(State(state): State<RobotAdminState>) -> Response {
    let mut robots = Vec::new();
    for record in state.registry.list().await {
        let mut robot = RobotDto::from(&record);
        for tool in state.tools.tools(&record.robot_id).await {
            if !crate::capability_profile::tool_enabled(&record.capability_profile, &tool.device_name) { continue; }
            let permission = match crate::capability_profile::resolve_tool_action(&record.capability_profile, &tool.device_name) {
                crate::capability::RobotAction::Display => "display",
                crate::capability::RobotAction::Motion => "motion",
                crate::capability::RobotAction::Vision => "vision",
                crate::capability::RobotAction::Device => "device_tools",
            };
            if !robot.supported_permissions.iter().any(|value| value == permission) { robot.supported_permissions.push(permission.to_owned()); }
            if crate::tool_registry::requires_continuous_vision_for_action(&tool.device_name,
                crate::capability_profile::resolve_tool_action(&record.capability_profile, &tool.device_name))
                && !robot.supported_permissions.iter().any(|value| value == "continuous_vision") {
                robot.supported_permissions.push("continuous_vision".to_owned());
            }
        }
        robots.push(robot);
    }
    Json(json!({ "robots": robots })).into_response()
}

#[derive(Serialize)]
struct CapabilityToolDto {
    device_name: String,
    exposed_name: String,
    description: String,
    input_schema: serde_json::Value,
    action: crate::capability::RobotAction,
    allowed_actions: Vec<crate::capability::RobotAction>,
    enabled: bool,
    effective_description: String,
    default_arguments: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
struct CapabilityCatalogDto {
    robot_id: String,
    connected: bool,
    revision: u64,
    profile: crate::capability_profile::RobotCapabilityProfile,
    tools: Vec<CapabilityToolDto>,
}

async fn capability_catalog(state: &RobotAdminState, record: &crate::registry::RobotRecord) -> CapabilityCatalogDto {
    let connection = state.registry.current_connection(&record.robot_id).await;
    let catalog = match connection {
        Some(connection) => state.tools.tools_for_connection(&record.robot_id, &connection).await,
        None => None,
    };
    let connected = catalog.is_some();
    let profile = &record.capability_profile;
    let tools = catalog.unwrap_or_default().into_iter().map(|tool| {
        let effective_description = crate::capability_profile::effective_description(profile, &tool);
        let action = crate::capability_profile::resolve_tool_action(profile, &tool.device_name);
        let allowed_actions = crate::capability_profile::allowed_tool_actions(&tool.device_name);
        let enabled = crate::capability_profile::tool_enabled(profile, &tool.device_name);
        let default_arguments = profile.tool_overrides.get(&tool.device_name)
            .map(|value| value.default_arguments.clone()).unwrap_or_default();
        CapabilityToolDto { device_name:tool.device_name, exposed_name:tool.exposed_name,
            description:tool.description, input_schema:tool.input_schema, action, allowed_actions,
            enabled, effective_description, default_arguments }
    }).collect();
    CapabilityCatalogDto { robot_id:record.robot_id.clone(), connected, revision:record.capability_revision,
        profile:profile.clone(), tools }
}

async fn capabilities(State(state): State<RobotAdminState>, Path(robot_id): Path<String>) -> Response {
    let Some(record) = state.registry.get(&robot_id).await else { return StatusCode::NOT_FOUND.into_response(); };
    Json(capability_catalog(&state, &record).await).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetCapabilitiesBody {
    expected_revision: u64,
    profile: crate::capability_profile::RobotCapabilityProfile,
}

async fn set_capabilities(State(state): State<RobotAdminState>, Path(robot_id): Path<String>, Json(body): Json<SetCapabilitiesBody>) -> Response {
    use crate::registry::CapabilityUpdateError;
    match state.registry.set_capability_profile(&robot_id, body.expected_revision, body.profile, &state.tools).await {
        Ok(record) => Json(capability_catalog(&state, &record).await).into_response(),
        Err(error) => {
            let (status, code) = match &error {
                CapabilityUpdateError::NotFound => (StatusCode::NOT_FOUND,"ROBOT_NOT_FOUND"),
                CapabilityUpdateError::Conflict => (StatusCode::CONFLICT,"ROBOT_CAPABILITIES_CHANGED"),
                CapabilityUpdateError::Offline => (StatusCode::CONFLICT,"ROBOT_OFFLINE"),
                CapabilityUpdateError::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY,"ROBOT_CAPABILITIES_INVALID"),
                CapabilityUpdateError::Persistence(_) => (StatusCode::INTERNAL_SERVER_ERROR,"ROBOT_CAPABILITIES_SAVE_FAILED"),
            };
            (status,Json(json!({"code":code,"error":error.to_string()}))).into_response()
        }
    }
}

async fn refresh_capabilities(State(state): State<RobotAdminState>, Path(robot_id): Path<String>) -> Response {
    use crate::registry::CapabilityRefreshError;
    match state.registry.refresh_capabilities(&robot_id,&state.tools).await {
        Ok(record) => Json(capability_catalog(&state,&record).await).into_response(),
        Err(error) => {
            let (status,code)=match &error {
                CapabilityRefreshError::NotFound => (StatusCode::NOT_FOUND,"ROBOT_NOT_FOUND"),
                CapabilityRefreshError::Offline => (StatusCode::CONFLICT,"ROBOT_OFFLINE"),
                CapabilityRefreshError::Device(_) => (StatusCode::BAD_GATEWAY,"ROBOT_CAPABILITIES_REFRESH_FAILED"),
                CapabilityRefreshError::Persistence(_) => (StatusCode::INTERNAL_SERVER_ERROR,"ROBOT_CAPABILITIES_SAVE_FAILED"),
            };
            (status,Json(json!({"code":code,"error":error.to_string()}))).into_response()
        }
    }
}

async fn set_permissions(
    State(state): State<RobotAdminState>, Path(robot_id): Path<String>,
    Json(permissions): Json<crate::registry::RobotPermissions>,
) -> Response {
    if state.registry.get(&robot_id).await.is_none() { return StatusCode::NOT_FOUND.into_response(); }
    match state.registry.set_permissions(&robot_id, permissions).await {
        Ok(record) => Json(RobotDto::from(&record)).into_response(),
        Err(error) => {
            tracing::error!(%error, "robot permission update failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "robot permission update failed").into_response()
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpeakBody { conversation_id: String }

async fn speak_response(State(state): State<RobotAdminState>, Path(robot_id): Path<String>, Json(body): Json<SpeakBody>) -> Response {
    let Some(source) = state.playback_source.as_ref() else { return StatusCode::SERVICE_UNAVAILABLE.into_response(); };
    let Some(record) = state.registry.get(&robot_id).await else { return StatusCode::NOT_FOUND.into_response(); };
    if !record.permissions.proactive_speech {
        return (StatusCode::FORBIDDEN, Json(json!({"message":"请先允许桌面播报"}))).into_response();
    }
    let Some(connection_id) = state.registry.current_connection(&robot_id).await else {
        return (StatusCode::CONFLICT, Json(json!({"message":"设备离线"}))).into_response();
    };
    match source.response_source(&robot_id, &body.conversation_id).await {
        Ok(response) => match state.registry.replay_saved_response(&robot_id, &connection_id, response).await {
            Ok(()) => Json(json!({"accepted":true})).into_response(),
            Err(error) => (StatusCode::CONFLICT, Json(json!({"message":error.to_string()}))).into_response(),
        },
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"message":error}))).into_response(),
    }
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
        Err(ClaimError::Persistence(error)) => {
            tracing::error!(%error, "robot claim could not persist its authority change");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn patch_robot(
    State(state): State<RobotAdminState>,
    Path(robot_id): Path<String>,
    Json(body): Json<PatchBody>,
) -> Response {
    let binding_changed = body.companion_id.is_some();
    match state
        .registry
        .patch(&robot_id, body.name, body.companion_id)
        .await
    {
        Ok(record) => {
            if binding_changed {
                state.tools.detach_if_disconnected(&state.registry, &robot_id).await;
                state.status.mark_offline_if_disconnected(&state.registry, &robot_id, chrono::Utc::now().timestamp_millis()).await;
            }
            Json(RobotDto::from(&record)).into_response()
        },
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
        Err(ClaimError::Persistence(error)) => {
            tracing::error!(%error, robot_id, "robot update could not persist its authority change");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_robot(
    State(state): State<RobotAdminState>,
    Path(robot_id): Path<String>,
) -> Response {
    match state.registry.remove(&robot_id).await {
        Ok(true) => {
            state.tools.detach_if_disconnected(&state.registry, &robot_id).await;
            state.status.mark_offline_if_disconnected(&state.registry, &robot_id, chrono::Utc::now().timestamp_millis()).await;
            StatusCode::NO_CONTENT.into_response()
        },
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
            tools: Arc::new(crate::tool_registry::RobotToolRegistry::default()),
            playback_source: None,
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

    async fn attach_catalog(h: &Seeded) {
        let id="aa:bb:cc:dd:ee:ff";
        h.state.registry.claim(&h.code,"companion-1").await.unwrap();
        h.state.registry.connect(id,"companion-1","capability-connection").await.unwrap();
        let (tx,_rx)=tokio::sync::mpsc::channel(4);
        h.state.tools.attach(id,Arc::new(crate::mcp_bridge::RobotMcpClient::new(tx,"capability-connection".into())),vec![crate::mcp_bridge::RobotToolDescriptor {
            device_name:"vendor.neck.turn".into(),exposed_name:"robot_vendor_neck_turn".into(),description:"Turn".into(),
            input_schema:json!({"type":"object","properties":{"angle":{"type":"integer","minimum":0,"maximum":90}},"required":["angle"],"additionalProperties":false}),
        }]).await;
    }

    #[tokio::test]
    async fn capability_catalog_supports_offline_read_and_online_cas_update() {
        let h=seeded().await;
        let uri="/api/robots/aa:bb:cc:dd:ee:ff/capabilities";
        let response=admin_router(h.state.clone()).oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let offline=json_of(response).await;
        assert_eq!(offline["connected"],false);
        assert_eq!(offline["revision"],0);
        let body=json!({"expected_revision":0,"profile":{"tool_overrides":{"vendor.neck.turn":{"action":"motion","default_arguments":{"angle":30}}},"presets":[{"id":"greeting","name":"Greeting","tool_name":"vendor.neck.turn","arguments":{"angle":45},"enabled":true}]}});
        let response=admin_router(h.state.clone()).oneshot(Request::builder().method("PUT").uri(uri).header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::CONFLICT);
        attach_catalog(&h).await;
        let response=admin_router(h.state.clone()).oneshot(Request::builder().method("PUT").uri(uri).header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        let saved=json_of(response).await;
        assert_eq!(saved["revision"],1);
        assert_eq!(saved["tools"][0]["action"],"motion");
        assert_eq!(saved["tools"][0]["default_arguments"]["angle"],30);
        assert_eq!(saved["tools"][0]["allowed_actions"].as_array().unwrap().len(),4);
        assert!(!h.state.registry.get("aa:bb:cc:dd:ee:ff").await.unwrap().permissions.motion);
        let stale=admin_router(h.state.clone()).oneshot(Request::builder().method("PUT").uri(uri).header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
        assert_eq!(stale.status(),StatusCode::CONFLICT);
        let mut invalid=body;
        invalid["expected_revision"]=json!(1);
        invalid["profile"]["tool_overrides"]["vendor.neck.turn"]["default_arguments"]["angle"]=json!(91);
        let response=admin_router(h.state.clone()).oneshot(Request::builder().method("PUT").uri(uri).header("content-type","application/json").body(Body::from(invalid.to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(h.state.registry.get("aa:bb:cc:dd:ee:ff").await.unwrap().capability_revision,1);
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
