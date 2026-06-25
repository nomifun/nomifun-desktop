//! Regression checks for product surfaces that have been intentionally removed.

mod common;

use axum::http::StatusCode;
use tower::ServiceExt;

use common::{build_app, get_with_token, setup_and_login};

#[tokio::test]
async fn removed_console_home_api_is_not_registered() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(get_with_token("/api/console/home", &token))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
