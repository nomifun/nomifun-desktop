use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_local_mode_skips_auth() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let config = nomifun_app::AppConfig {
        auth_policy: nomifun_app::AuthPolicy::NoAuth,
        ..Default::default()
    };
    let services = nomifun_app::AppServices::from_config(db, &config).await.unwrap();

    let router = nomifun_app::create_router(&services).await;

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
    let services = nomifun_app::AppServices::from_config(db, &nomifun_app::AppConfig::default())
        .await
        .unwrap();

    let router = nomifun_app::create_router(&services).await;

    let response = router
        .oneshot(Request::builder().uri("/api/settings").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    services.database.close().await;
}
