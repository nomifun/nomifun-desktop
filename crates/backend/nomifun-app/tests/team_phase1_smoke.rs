//! D11 — Wave-2 app assembly smoke test.
//!
//! Minimum guarantee: after D7/D8/D9/D10 merged, `AppServices` composes into
//! a router that actually exposes the `/api/teams` endpoints. Anything beyond
//! compile-check is validated by `team_e2e.rs`; this file is kept intentionally
//! tiny so assembly regressions surface first.

mod common;

use axum::http::StatusCode;
use tower::ServiceExt;

use common::{build_app, get_with_token, setup_and_login};

/// Router boots and `/api/teams` is wired through `build_team_state` into
/// `nomifun_team::team_routes`. If the team module failed to assemble, the
/// route would 404 (or compile would have failed earlier).
#[tokio::test]
async fn phase1_router_assembles_with_team_module() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = get_with_token("/api/teams", &token);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "GET /api/teams must be wired through build_team_state"
    );
}
