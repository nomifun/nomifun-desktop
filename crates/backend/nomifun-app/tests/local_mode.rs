use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn isolated_config(prefix: &str) -> nomifun_app::AppConfig {
    let root = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .unwrap()
        .keep();
    nomifun_app::AppConfig {
        data_dir: root.join("data"),
        work_dir: root.join("work"),
        ..Default::default()
    }
}

#[tokio::test]
async fn test_local_mode_skips_auth() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let config = nomifun_app::AppConfig {
        auth_policy: nomifun_app::AuthPolicy::NoAuth,
        ..isolated_config("nomifun-local-mode-e2e-")
    };
    let services = nomifun_app::compatibility::AppServices::from_config(db, &config).await.unwrap();

    let router = nomifun_app::compatibility::create_router(&services).await;

    // Health check should work
    let response = router
        .clone()
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // An authenticated endpoint should work WITHOUT a token in local mode
    let response = router
        .oneshot(Request::builder().uri("/api/settings").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::FORBIDDEN);

    services.database.close().await;
}

#[tokio::test]
async fn test_non_local_mode_requires_auth() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = nomifun_app::compatibility::AppServices::from_config(
        db,
        &isolated_config("nomifun-auth-required-e2e-"),
    )
    .await
    .unwrap();

    let router = nomifun_app::compatibility::create_router(&services).await;

    let response = router
        .oneshot(Request::builder().uri("/api/settings").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    services.database.close().await;
}
