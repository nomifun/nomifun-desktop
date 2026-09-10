use std::sync::Arc;

use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::get;
use axum::{Router, body::Body};
use nomifun_auth::{
    CurrentUser, InstallationTokenTrustState, InstanceTokenValidator,
    installation_token_trust_resolve_middleware, require_local_product_trust_middleware,
    token_sha256_hex,
};
use tower::ServiceExt;

const TEST_OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
const TEST_TOKEN: &str = "installation-product-token";

fn product_router(require_product_trust: bool) -> Router {
    let state = InstallationTokenTrustState {
        validator: Arc::new(InstanceTokenValidator::new(Some(token_sha256_hex(
            TEST_TOKEN,
        )))),
        authoritative_user_id: Arc::from(TEST_OWNER_ID),
    };
    let router = Router::new().route(
        "/product",
        get(|Extension(user): Extension<CurrentUser>| async move { user.id.to_string() }),
    );
    let router = if require_product_trust {
        router.route_layer(from_fn(require_local_product_trust_middleware))
    } else {
        router
    };
    router.route_layer(from_fn_with_state(
        state,
        installation_token_trust_resolve_middleware,
    ))
}

#[tokio::test]
async fn installation_token_resolves_the_canonical_owner_for_product_routes() {
    let response = product_router(true)
        .oneshot(
            Request::get("/product")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body, TEST_OWNER_ID);
}

#[tokio::test]
async fn product_trust_rejects_unknown_or_missing_tokens() {
    for token in [Some("wrong"), None] {
        let mut request = Request::get("/product");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = product_router(true)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn resolver_leaves_unknown_tokens_for_normal_authentication() {
    let state = InstallationTokenTrustState {
        validator: Arc::new(InstanceTokenValidator::new(Some(token_sha256_hex(
            TEST_TOKEN,
        )))),
        authoritative_user_id: Arc::from(TEST_OWNER_ID),
    };
    let response = Router::new()
        .route("/product", get(|| async { StatusCode::NO_CONTENT }))
        .route_layer(from_fn_with_state(
            state,
            installation_token_trust_resolve_middleware,
        ))
        .oneshot(
            Request::get("/product")
                .header("authorization", "Bearer ordinary-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}
