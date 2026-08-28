//! Transport + auth wiring for the Remote front door.
//!
//! Mounts rmcp's official `StreamableHttpService` at `/mcp` and wraps it with a
//! installation-token middleware. The host MUST mount this with `.nest("/mcp", ..)`
//! (NEVER `.merge` — see [`public_mcp_router`] for why); it then rides both the
//! desktop loopback/LAN listeners and the headless web host, sharing service
//! state with the SPA.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{Next, from_fn, from_fn_with_state},
    response::{IntoResponse, Response},
};
use nomifun_auth::InstanceTokenValidator;
use nomifun_common::UserId;
use nomifun_gateway::GatewayDeps;
use http_body_util::BodyExt;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use tower_http::limit::RequestBodyLimitLayer;

use crate::handler::RemoteMcpHandler;
use crate::session::{
    RemoteHttpRequestAdmissionError, RemoteHttpRequestPermit,
    RemoteMcpSessionAdmissionAuthority, RemoteSessionManager,
};

#[cfg(not(test))]
const REMOTE_MCP_BODY_READ_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const REMOTE_MCP_BODY_READ_TIMEOUT: Duration = Duration::from_millis(100);

/// The installation owner authenticated by the Remote token, stashed in the
/// request extensions by [`instance_token_middleware`] and read by both adapters
/// (MCP via `RequestContext.extensions`→`http::request::Parts`; REST via
/// `Extension<RemoteInstanceOwner>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteInstanceOwner(pub UserId);

/// State for the installation-token middleware.
#[derive(Clone)]
pub struct PublicMcpState {
    pub validator: Arc<InstanceTokenValidator>,
    pub authoritative_user_id: UserId,
}

/// Reject any request to the Remote surface that does not carry a valid
/// installation API token in `Authorization: Bearer <token>`. A valid token
/// authenticates the NomiFun Desktop installation owner; it never resolves or
/// binds a companion. The owner is stashed as [`RemoteInstanceOwner`]. Shared by
/// the `/mcp` and `/v1` (REST) adapters.
pub(crate) async fn instance_token_middleware(
    State(state): State<PublicMcpState>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !state.validator.validate(presented) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let mut request = request;
    request
        .extensions_mut()
        .insert(RemoteInstanceOwner(state.authoritative_user_id));
    next.run(request).await
}

#[derive(Clone)]
struct McpAuthState {
    public: PublicMcpState,
    sessions: Arc<RemoteSessionManager>,
}

#[derive(serde::Deserialize)]
struct InitializePreflight<'a> {
    #[serde(borrow)]
    method: &'a str,
}

fn presented_mcp_session_id(
    headers: &HeaderMap,
) -> Result<Option<&str>, ()> {
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

pub(crate) async fn initialize_preflight_middleware(
    request: Request,
    next: Next,
) -> Response {
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }

    // Read every POST here: rmcp consumes Body directly, so extractor timeouts
    // do not apply. This absolute deadline prevents a slow sender from holding
    // either a session or headerless-tenant request permit indefinitely. The
    // explicit byte limit applies before the same immutable Bytes are rebuilt.
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
    // rmcp 1.7 creates a LocalSession before it validates that a headerless
    // message is `initialize`; its early non-initialize return never calls the
    // SessionManager close hook. Inspect only the borrowed top-level method so
    // invalid traffic cannot mint orphaned transport sessions.
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
    // `map_frame` owns the closure. Capturing the permit keeps long-lived SSE
    // responses charged until their body completes or the client disconnects,
    // instead of refunding as soon as response headers are produced.
    let guarded = body.map_frame(move |frame| {
        let _keep_alive = &permit;
        frame
    });
    Response::from_parts(parts, Body::new(guarded))
}

async fn mcp_instance_token_middleware(
    State(state): State<McpAuthState>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !state.public.validator.validate(presented) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let owner_user_id = state.public.authoritative_user_id.clone();

    let presented_session_id = match presented_mcp_session_id(request.headers()) {
        Ok(value) => value,
        Err(()) => {
            return (StatusCode::BAD_REQUEST, "invalid session id")
                .into_response();
        }
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
            session_id.is_none()
                && request.method() == axum::http::Method::POST,
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

/// Build the Remote front-door sub-router (MCP Streamable-HTTP) gated by the
/// installation token. The caller MUST mount it with `.nest("/mcp", ..)` (NOT
/// `.merge`): `nest` scopes both the token-auth layer and this router's
/// fallback service to the `/mcp` prefix, so it cannot hijack the host app's
/// global 404 fallback (merging a layered router would route every unmatched
/// path through the token middleware → spurious 401s). `deps` is the SAME
/// `Arc<GatewayDeps>` the SPA/inward gateway use (shared state, one dispatch
/// authority). `domains = None` advertises the full Remote surface; `Some(..)`
/// advertises a curated profile (e.g. `AGENT_PROFILE_DOMAINS`).
pub fn public_mcp_router(
    deps: Arc<GatewayDeps>,
    validator: Arc<InstanceTokenValidator>,
    domains: Option<&'static [&'static str]>,
) -> Router {
    let admission =
        RemoteMcpSessionAdmissionAuthority::for_gateway(deps.as_ref());
    public_mcp_router_with_admission(deps, validator, domains, admission)
}

/// Build a Remote MCP endpoint on a shared admission authority. Hosts exposing
/// more than one profile must use this form so session/rate/retained-byte caps
/// cannot be multiplied by switching endpoint paths.
pub fn public_mcp_router_with_admission(
    deps: Arc<GatewayDeps>,
    validator: Arc<InstanceTokenValidator>,
    domains: Option<&'static [&'static str]>,
    admission: RemoteMcpSessionAdmissionAuthority,
) -> Router {
    // The installation token (a 256-bit Bearer in the Authorization header, NOT a
    // cookie) is the real gate — it is non-ambient, so a DNS-rebinding browser
    // page cannot read it or have it auto-attached, and any rebound request is
    // rejected 401 before reaching a tool. rmcp's own Host check defaults to
    // loopback-only (would reject LAN/public hosts), so we disable it; on the
    // desktop LAN listener the app additionally layers a host_guard (DNS-rebind)
    // — the headless web host relies on the token + your TLS/reverse proxy.
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();

    let authoritative_user_id = UserId::parse(deps.authoritative_user_id.as_ref())
        .expect("GatewayDeps authoritative user id must be canonical");
    let session_manager = Arc::new(
        RemoteSessionManager::with_admission_authority(
            deps.clone(),
            domains,
            admission,
        ),
    );
    let service: StreamableHttpService<RemoteMcpHandler, RemoteSessionManager> = StreamableHttpService::new(
        {
            let deps = deps.clone();
            move || {
                Ok(match domains {
                    Some(d) => RemoteMcpHandler::with_domains(deps.clone(), d),
                    None => RemoteMcpHandler::new(deps.clone()),
                })
            }
        },
        session_manager.clone(),
        config,
    );

    // `fallback_service` serves every path within the `/mcp` nest; the token
    // layer wraps it. Scoped by `nest`, so the global fallback is untouched.
    Router::new()
        .fallback_service(service)
        // rmcp consumes the Body directly with `collect`; Axum's extractor
        // default does not apply. Install the streaming limit on this exact
        // nested service before any JSON/session allocation can occur.
        .layer(RequestBodyLimitLayer::new(
            nomifun_common::constants::BODY_LIMIT,
        ))
        .layer(from_fn(initialize_preflight_middleware))
        .layer(from_fn_with_state(
            McpAuthState {
                public: PublicMcpState {
                    validator,
                    authoritative_user_id,
                },
                sessions: session_manager,
            },
            mcp_instance_token_middleware,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use nomifun_auth::token_sha256_hex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt; // oneshot

    #[test]
    fn invalid_or_ambiguous_session_headers_fail_closed() {
        for value in [
            axum::http::HeaderValue::from_static(""),
            axum::http::HeaderValue::from_static(" padded "),
            axum::http::HeaderValue::from_str(&"x".repeat(129)).unwrap(),
            axum::http::HeaderValue::from_bytes(&[0x80]).unwrap(),
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
        assert_eq!(
            presented_mcp_session_id(&HeaderMap::new()).unwrap(),
            None
        );
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
        for _ in 0..16 {
            let response = app
                .clone()
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
        }
        assert_eq!(downstream_calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn oversized_remote_mcp_body_is_413_before_session_service() {
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
            .layer(RequestBodyLimitLayer::new(
                nomifun_common::constants::BODY_LIMIT,
            ))
            .layer(from_fn(initialize_preflight_middleware));
        for session_id in [None, Some("established-session")] {
            let mut request = HttpRequest::post("/mcp");
            if let Some(session_id) = session_id {
                request = request.header("mcp-session-id", session_id);
            }
            let response = app
                .clone()
                .oneshot(
                    request
                        .body(Body::from(vec![
                            b'x';
                            nomifun_common::constants::BODY_LIMIT + 1
                        ]))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        }
        assert_eq!(downstream_calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn slow_remote_post_body_is_408_before_session_service() {
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
        for session_id in [None, Some("established-session")] {
            let mut request = HttpRequest::post("/mcp");
            if let Some(session_id) = session_id {
                request = request.header("mcp-session-id", session_id);
            }
            let response = app
                .clone()
                .oneshot(
                    request
                        .body(Body::from_stream(futures::stream::pending::<
                            Result<axum::body::Bytes, std::io::Error>,
                        >()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        }
        assert_eq!(downstream_calls.load(Ordering::Acquire), 0);
    }

    // A request with no Authorization header (or an unknown token) is rejected
    // before reaching the MCP service; a valid token authenticates the owner.
    #[tokio::test]
    async fn missing_token_is_unauthorized() {
        let owner_user_id = UserId::new();
        let validator = Arc::new(InstanceTokenValidator::new(Some(
            token_sha256_hex("secret-token"),
        )));
        // We can't easily build GatewayDeps in a unit test, so exercise the
        // middleware in isolation over a trivial inner router.
        let state = PublicMcpState {
            validator: validator.clone(),
            authoritative_user_id: owner_user_id.clone(),
        };
        let app = Router::new()
            .route("/mcp", axum::routing::post(|| async { "ok" }))
            .layer(from_fn_with_state(state, instance_token_middleware));

        let res = app
            .clone()
            .oneshot(HttpRequest::post("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let ok = app
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer secret-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);

        // Revocation closes it.
        validator.clear_token();
        let res2 = Router::new()
            .route("/mcp", axum::routing::post(|| async { "ok" }))
            .layer(from_fn_with_state(
                PublicMcpState {
                    validator,
                    authoritative_user_id: owner_user_id,
                },
                instance_token_middleware,
            ))
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer secret-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res2.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_token_inserts_remote_instance_owner_extension() {
        use axum::routing::get;
        use nomifun_auth::token_sha256_hex;

        let owner_user_id = UserId::new();
        let validator = std::sync::Arc::new(nomifun_auth::InstanceTokenValidator::new(Some(
            token_sha256_hex("secret-tok"),
        )));
        // A probe handler that echoes whether the extension is present + its value.
        async fn probe(ext: Option<axum::Extension<RemoteInstanceOwner>>) -> String {
            match ext {
                Some(axum::Extension(RemoteInstanceOwner(owner))) => format!("owner={owner}"),
                None => "none".into(),
            }
        }
        let app = axum::Router::new()
            .route("/probe", get(probe))
            .layer(axum::middleware::from_fn_with_state(
                PublicMcpState {
                    validator,
                    authoritative_user_id: owner_user_id.clone(),
                },
                instance_token_middleware,
            ));

        // Valid token → 200 + installation owner echoed.
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/probe")
                    .header(axum::http::header::AUTHORIZATION, "Bearer secret-tok")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let expected = format!("owner={owner_user_id}");
        assert_eq!(body.as_ref(), expected.as_bytes());

        // Bad token → 401.
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/probe")
                    .header(axum::http::header::AUTHORIZATION, "Bearer nope")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
}
