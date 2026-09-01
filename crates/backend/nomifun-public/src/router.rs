//! Transport and authentication wiring for the canonical Remote front door.
//!
//! This module owns only installation-token admission and the rmcp transport
//! lifecycle. Product identity and execution are delegated to the canonical
//! AgentPlatform adapter in `canonical.rs`.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
#[cfg(test)]
use axum::{
    Router,
    middleware::from_fn,
};
use http_body_util::BodyExt;
use nomifun_auth::{InstanceTokenValidator, RemoteAuthAdmissionFence};
use nomifun_common::UserId;

use crate::session::{
    RemoteHttpRequestAdmissionError, RemoteHttpRequestPermit, RemoteSessionManager,
};

#[cfg(not(test))]
const REMOTE_MCP_BODY_READ_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const REMOTE_MCP_BODY_READ_TIMEOUT: Duration = Duration::from_millis(100);

/// The installation owner authenticated by the Remote token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteInstanceOwner(pub UserId);

/// State for a plain installation-token middleware.
#[derive(Clone)]
pub struct PublicMcpState {
    pub validator: Arc<InstanceTokenValidator>,
    pub authoritative_user_id: UserId,
}

/// State for a middleware that also participates in the D-026 auth fence.
#[derive(Clone)]
pub struct PublicMcpAdmissionState {
    pub public: PublicMcpState,
    pub admission: RemoteAuthAdmissionFence,
}

/// Authenticate an installation owner and attach it to the request.
pub async fn instance_token_middleware(
    State(state): State<PublicMcpState>,
    request: Request,
    next: Next,
) -> Response {
    let presented = presented_token(&request);
    if !state.validator.validate(presented) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "success": false,
                "error": "Remote installation authentication is required",
                "code": "REMOTE_AUTH_REQUIRED"
            })),
        )
            .into_response();
    }
    let mut request = request;
    request
        .extensions_mut()
        .insert(RemoteInstanceOwner(state.authoritative_user_id));
    next.run(request).await
}

/// Authenticate while holding the shared side of the D-026 request fence.
pub async fn instance_token_middleware_with_admission(
    State(state): State<PublicMcpAdmissionState>,
    request: Request,
    next: Next,
) -> Response {
    let admission = state.admission.acquire_request_admission().await;
    let presented = presented_token(&request);
    if !state.public.validator.validate(presented) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "success": false,
                "error": "Remote installation authentication is required",
                "code": "REMOTE_AUTH_REQUIRED"
            })),
        )
            .into_response();
    }
    let mut request = request;
    request
        .extensions_mut()
        .insert(RemoteInstanceOwner(state.public.authoritative_user_id));
    response_with_auth_admission(next.run(request).await, admission)
}

fn presented_token(request: &Request) -> &str {
    request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("")
}

fn response_with_auth_admission(
    response: Response,
    admission: nomifun_auth::RemoteRequestAdmissionPermit,
) -> Response {
    let (parts, body) = response.into_parts();
    let guarded = body.map_frame(move |frame| {
        let _keep_alive = &admission;
        frame
    });
    Response::from_parts(parts, Body::new(guarded))
}

#[derive(Clone)]
pub(crate) struct McpAuthState {
    pub(crate) public: PublicMcpState,
    pub(crate) sessions: Arc<RemoteSessionManager>,
}

#[derive(serde::Deserialize)]
struct InitializePreflight<'a> {
    #[serde(borrow)]
    method: &'a str,
}

fn presented_mcp_session_id(headers: &HeaderMap) -> Result<Option<&str>, ()> {
    let mut values = headers.get_all("mcp-session-id").iter();
    let value = match (values.next(), values.next()) {
        (None, None) => return Ok(None),
        (Some(value), None) => value.to_str().map_err(|_| ())?,
        _ => return Err(()),
    };
    if value.is_empty() || value.len() > 128 || value.trim() != value {
        return Err(());
    }
    Ok(Some(value))
}

/// Read and validate a POST before rmcp can allocate a transport session.
pub(crate) async fn initialize_preflight_middleware(
    request: Request,
    next: Next,
) -> Response {
    if request.uri().query().is_some() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "success": false,
                "error": "canonical Remote MCP does not accept query parameters",
                "code": "REMOTE_INVALID_REQUEST"
            })),
        )
            .into_response();
    }
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }

    let needs_initialize = !request.headers().contains_key("mcp-session-id");
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(
        REMOTE_MCP_BODY_READ_TIMEOUT,
        to_bytes(body, nomifun_common::constants::BODY_LIMIT),
    )
    .await
    {
        Err(_) => {
            return (
                StatusCode::REQUEST_TIMEOUT,
                "Remote MCP request body read timed out",
            )
                .into_response();
        }
        Ok(Err(_)) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Remote MCP request body exceeds its byte limit",
            )
                .into_response();
        }
        Ok(Ok(bytes)) => bytes,
    };

    if !needs_initialize {
        return next.run(Request::from_parts(parts, Body::from(bytes))).await;
    }

    match serde_json::from_slice::<InitializePreflight<'_>>(&bytes) {
        Ok(preflight) if preflight.method == "initialize" => {
            next.run(Request::from_parts(parts, Body::from(bytes))).await
        }
        _ => (
            StatusCode::BAD_REQUEST,
            "a headerless Remote MCP POST must be an initialize request",
        )
            .into_response(),
    }
}

pub(crate) fn response_with_request_permit(
    response: Response,
    permit: RemoteHttpRequestPermit,
) -> Response {
    let (parts, body) = response.into_parts();
    let guarded = body.map_frame(move |frame| {
        let _keep_alive = &permit;
        frame
    });
    Response::from_parts(parts, Body::new(guarded))
}

/// Authenticate each MCP request and enforce the transport admission budget.
pub(crate) async fn mcp_instance_token_middleware(
    State(state): State<McpAuthState>,
    request: Request,
    next: Next,
) -> Response {
    let presented = presented_token(&request);
    if !state.public.validator.validate(presented) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "success": false,
                "error": "Remote installation authentication is required",
                "code": "REMOTE_AUTH_REQUIRED"
            })),
        )
            .into_response();
    }
    let owner_user_id = state.public.authoritative_user_id.clone();
    let presented_session_id = match presented_mcp_session_id(request.headers()) {
        Ok(value) => value,
        Err(()) => return (StatusCode::BAD_REQUEST, "invalid session id").into_response(),
    };
    let session_id = presented_session_id.map(Into::into);
    if session_id.is_none() && request.method() == axum::http::Method::DELETE {
        return (StatusCode::BAD_REQUEST, "session id required").into_response();
    }
    let permit = match state
        .sessions
        .acquire_http_request_permit(
            session_id.as_ref(),
            &owner_user_id,
            session_id.is_none() && request.method() == axum::http::Method::POST,
            request.method() == axum::http::Method::DELETE,
        )
        .await
    {
        Ok(permit) => permit,
        Err(RemoteHttpRequestAdmissionError::IdentityMismatch) => {
            return (
                StatusCode::UNAUTHORIZED,
                "session is bound to a different installation owner",
            )
                .into_response();
        }
        Err(RemoteHttpRequestAdmissionError::CapacityExceeded) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "Remote MCP request capacity is temporarily exhausted",
            )
                .into_response();
        }
    };

    let mut request = request;
    request
        .extensions_mut()
        .insert(RemoteInstanceOwner(owner_user_id));
    let response = next.run(request).await;
    match permit {
        Some(permit) => response_with_request_permit(response, permit),
        None => response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    #[test]
    fn invalid_or_ambiguous_session_headers_fail_closed() {
        for value in [
            axum::http::HeaderValue::from_static(""),
            axum::http::HeaderValue::from_static(" padded "),
            axum::http::HeaderValue::from_str(&"x".repeat(129)).unwrap(),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("mcp-session-id", value);
            assert!(presented_mcp_session_id(&headers).is_err());
        }
        let mut duplicate = HeaderMap::new();
        duplicate.append(
            "mcp-session-id",
            axum::http::HeaderValue::from_static("first"),
        );
        duplicate.append(
            "mcp-session-id",
            axum::http::HeaderValue::from_static("second"),
        );
        assert!(presented_mcp_session_id(&duplicate).is_err());
        assert_eq!(presented_mcp_session_id(&HeaderMap::new()).unwrap(), None);
    }

    #[tokio::test]
    async fn headerless_non_initialize_is_rejected_before_session_service() {
        let downstream_calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&downstream_calls);
        let app = Router::new()
            .route(
                "/mcp",
                axum::routing::post(move || {
                    let probe = Arc::clone(&probe);
                    async move {
                        probe.fetch_add(1, Ordering::AcqRel);
                        "unexpected"
                    }
                }),
            )
            .layer(from_fn(initialize_preflight_middleware));
        let response = app
            .oneshot(
                HttpRequest::post("/mcp")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(downstream_calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn canonical_mcp_selector_queries_are_rejected_before_session_service() {
        let downstream_calls = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&downstream_calls);
        let app = Router::new()
            .route(
                "/mcp",
                axum::routing::post(move || {
                    let probe = Arc::clone(&probe);
                    async move {
                        probe.fetch_add(1, Ordering::AcqRel);
                        "unexpected"
                    }
                }),
            )
            .layer(from_fn(initialize_preflight_middleware));
        let response = app
            .oneshot(
                HttpRequest::post("/mcp?profile=agent")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(downstream_calls.load(Ordering::Acquire), 0);
    }
}
