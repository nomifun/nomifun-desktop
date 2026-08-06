//! Deleting a host must take its live links with it.
//!
//! No sshd needed: the link is created by a dial that fails (port 1 is never
//! listening), which is enough to put a link in the pool — and a pooled link is
//! exactly what a delete has to withdraw. Driven through the real router rather
//! than by calling the pool directly, because the bug this pins is a *wiring*
//! one: `close_for_host` existed and worked, and nothing called it.
#[path = "support/mod.rs"]
mod support;

use axum::body::Body;
use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use nomifun_auth::CurrentUser;
use nomifun_common::UserId;
use nomifun_ssh::dto::CreateSshHostRequest;
use nomifun_ssh::{SshHostRouterState, SshLinkKey};
use tower::ServiceExt; // oneshot

#[tokio::test]
async fn deleting_a_host_withdraws_its_pooled_links() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let harness = support::harness(tmp.path().join("known_hosts"), support::brisk_tuning()).await;
    let id = harness
        .add_host(CreateSshHostRequest {
            name: "doomed".into(),
            host: "127.0.0.1".into(),
            // Reserved and never listening, so the dial fails fast instead of
            // hanging — the link still joins the pool, which is all this needs.
            port: 1,
            username: "nobody".into(),
            auth_type: "password".into(),
            password: Some("pw".into()),
            private_key: None,
            passphrase: None,
            certificate: None,
            sudo_password: None,
        })
        .await;
    let key = SshLinkKey::new("conv-1", id.clone());

    harness
        .pool
        .acquire(&harness.user_id, "conv-1", &id, "/")
        .await
        .expect_err("port 1 is not listening");
    assert!(
        harness.pool.is_pooled(&key),
        "a failed dial still leaves a link in the pool, which is what a delete must withdraw"
    );

    let router = nomifun_ssh::ssh_host_routes(SshHostRouterState {
        service: harness.service(),
        pool: Some(harness.pool.clone()),
    })
    .layer(Extension(CurrentUser {
        id: UserId::parse(harness.user_id.clone()).expect("seeded user id"),
        username: harness.user_id.clone(),
    }));

    let response = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/ssh-hosts/{}", id.as_str()))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router answers");
    assert_eq!(response.status(), StatusCode::OK);

    // The whole point: an agent holding this link would otherwise keep running
    // commands on a host the operator just deleted, with no pill on screen to
    // say which machine it is talking to.
    assert!(
        !harness.pool.is_pooled(&key),
        "deleting the host must close its links, not just its row"
    );
    assert_eq!(harness.pool.active_link_count(), 0);
}
