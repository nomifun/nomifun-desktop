//! Transport + auth wiring for the Remote front door.
//!
//! Mounts rmcp's official `StreamableHttpService` at `/mcp` and wraps it with a
//! companion-token middleware. The host MUST mount this with `.nest("/mcp", ..)`
//! (NEVER `.merge` — see [`public_mcp_router`] for why); it then rides both the
//! desktop loopback/LAN listeners and the headless web host, sharing service
//! state with the SPA.

use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{Next, from_fn_with_state},
    response::{IntoResponse, Response},
};
use nomifun_auth::CompanionTokenValidator;
use nomifun_gateway::GatewayDeps;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

use crate::handler::RemoteMcpHandler;

/// The companion a validated Remote token is bound to, stashed in the request
/// extensions by [`companion_token_middleware`] and read by both adapters
/// (MCP via `RequestContext.extensions`→`http::request::Parts`; REST via
/// `Extension<RemoteCompanion>`).
#[derive(Clone, Debug)]
pub struct RemoteCompanion(pub String);

/// State for the companion-token middleware.
#[derive(Clone)]
pub struct PublicMcpState {
    pub validator: Arc<CompanionTokenValidator>,
}

/// Reject any request to the Remote surface that does not carry a valid
/// per-companion API token in `Authorization: Bearer <token>`. A valid token
/// resolves to the companion it is bound to; the resolved companion id is
/// stashed in the request extensions as [`RemoteCompanion`] so both adapters
/// can thread it into `CallerCtx.companion_id`. Anything else is 401. Shared by
/// the `/mcp` and `/v1` (REST) adapters.
pub(crate) async fn companion_token_middleware(
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
    match state.validator.resolve(presented) {
        Some(companion_id) => {
            let mut request = request;
            request.extensions_mut().insert(RemoteCompanion(companion_id));
            next.run(request).await
        }
        None => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    }
}

/// Build the Remote front-door sub-router (MCP Streamable-HTTP) gated by the
/// companion token. The caller MUST mount it with `.nest("/mcp", ..)` (NOT
/// `.merge`): `nest` scopes both the token-auth layer and this router's
/// fallback service to the `/mcp` prefix, so it cannot hijack the host app's
/// global 404 fallback (merging a layered router would route every unmatched
/// path through the token middleware → spurious 401s). `deps` is the SAME
/// `Arc<GatewayDeps>` the SPA/inward gateway use (shared state, one dispatch
/// authority). `domains = None` advertises the full Remote surface; `Some(..)`
/// advertises a curated profile (e.g. `AGENT_PROFILE_DOMAINS`).
pub fn public_mcp_router(
    deps: Arc<GatewayDeps>,
    validator: Arc<CompanionTokenValidator>,
    domains: Option<&'static [&'static str]>,
) -> Router {
    // The companion token (a 256-bit Bearer in the Authorization header, NOT a
    // cookie) is the real gate — it is non-ambient, so a DNS-rebinding browser
    // page cannot read it or have it auto-attached, and any rebound request is
    // rejected 401 before reaching a tool. rmcp's own Host check defaults to
    // loopback-only (would reject LAN/public hosts), so we disable it; on the
    // desktop LAN listener the app additionally layers a host_guard (DNS-rebind)
    // — the headless web host relies on the token + your TLS/reverse proxy.
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();

    let service: StreamableHttpService<RemoteMcpHandler, LocalSessionManager> = StreamableHttpService::new(
        {
            let deps = deps.clone();
            move || {
                Ok(match domains {
                    Some(d) => RemoteMcpHandler::with_domains(deps.clone(), d),
                    None => RemoteMcpHandler::new(deps.clone()),
                })
            }
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );

    // `fallback_service` serves every path within the `/mcp` nest; the token
    // layer wraps it. Scoped by `nest`, so the global fallback is untouched.
    Router::new()
        .fallback_service(service)
        .layer(from_fn_with_state(PublicMcpState { validator }, companion_token_middleware))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use nomifun_auth::token_sha256_hex;
    use tower::ServiceExt; // oneshot

    // A request with no Authorization header (or an unknown token) is rejected
    // before reaching the MCP service; a valid token resolves to its companion.
    #[tokio::test]
    async fn missing_token_is_unauthorized() {
        let validator =
            Arc::new(CompanionTokenValidator::new(vec![("comp".into(), token_sha256_hex("secret-token"))]));
        // We can't easily build GatewayDeps in a unit test, so exercise the
        // middleware in isolation over a trivial inner router.
        let state = PublicMcpState { validator: validator.clone() };
        let app = Router::new()
            .route("/mcp", axum::routing::post(|| async { "ok" }))
            .layer(from_fn_with_state(state, companion_token_middleware));

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
        validator.remove_token("comp");
        let res2 = Router::new()
            .route("/mcp", axum::routing::post(|| async { "ok" }))
            .layer(from_fn_with_state(PublicMcpState { validator }, companion_token_middleware))
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
    async fn valid_token_inserts_remote_companion_extension() {
        use axum::routing::get;
        use nomifun_auth::token_sha256_hex;

        let validator = std::sync::Arc::new(
            nomifun_auth::CompanionTokenValidator::new(vec![("comp-x".into(), token_sha256_hex("secret-tok"))]),
        );
        // A probe handler that echoes whether the extension is present + its value.
        async fn probe(ext: Option<axum::Extension<RemoteCompanion>>) -> String {
            match ext {
                Some(axum::Extension(RemoteCompanion(c))) => format!("companion={c}"),
                None => "none".into(),
            }
        }
        let app = axum::Router::new()
            .route("/probe", get(probe))
            .layer(axum::middleware::from_fn_with_state(
                PublicMcpState { validator },
                companion_token_middleware,
            ));

        // Valid token → 200 + companion echoed.
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
        assert_eq!(&body[..], b"companion=comp-x");

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
