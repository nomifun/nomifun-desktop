use super::*;
use nomifun_agent_contracts::{PluginAgentSessionRequest, StrictJsonValue};
use nomifun_plugin_platform::runtime::{PluginAgentSessionPort, PluginRuntimeApplicationError};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[test]
fn authoring_agent_view_requires_html_but_not_a_service() {
    let declaration = serde_json::json!({"agent_view": {"name": "My Agent", "description": "My Session page"}});
    assert!(super::super::authoring::validate_plugin_source(HTML, None, Some(&declaration)).is_ok());
    assert!(super::super::authoring::validate_plugin_source("", Some("export function start() {}"), Some(&declaration)).is_err());
}

struct StreamAdmission {
    pause: AtomicBool,
    entered: Notify,
    release: Notify,
}

#[async_trait::async_trait]
impl PluginAgentSessionPort for StreamAdmission {
    async fn authorize(&self, _: &str, _: &str) -> Result<(), PluginRuntimeApplicationError> {
        if self.pause.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(())
    }

    async fn request(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: PluginAgentSessionRequest,
    ) -> Result<StrictJsonValue, PluginRuntimeApplicationError> {
        panic!("stream observation must not execute a Session command")
    }
}

#[tokio::test]
async fn session_stream_requires_current_exact_surface_scope_and_release() {
    let (db, _root, service, owner) = fixture().await;
    let session = uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO conversations (conversation_id, user_id, name, type, created_at, updated_at) VALUES (?, ?, 'Stream view', 'nomi', 1, 1)")
        .bind(&session).bind(&owner).execute(db.pool()).await.unwrap();
    service
        .application
        .install_agent_session_port(Arc::new(StreamAdmission {
            pause: AtomicBool::new(false),
            entered: Notify::new(),
            release: Notify::new(),
        }))
        .await;
    let mut value = draft();
    service.put_draft(&owner, &mut value).await.unwrap();
    let saved = service.save_draft(&owner, &mut value).await.unwrap();
    let id = saved.plugin.plugin_id;
    let digest = saved.plugin.releases.active.unwrap().release_digest;
    let events =
        vec![serde_json::json!({"conversation_id": session, "type": "text", "data": "private"})];
    let app = &service.application;
    app.open_surface(&owner, &id).await.unwrap();
    assert!(
        app.project_agent_session_stream(&owner, &session, &events)
            .await
            .unwrap()
            .is_empty()
    );
    let surface = app
        .open_surface_with_agent_session(&owner, &id, Some((&session, &digest)))
        .await
        .unwrap();
    let projected = app
        .project_agent_session_stream(&owner, &session, &events)
        .await
        .unwrap();
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].plugin_id, id);
    assert_eq!(projected[0].surface_session_id, surface.surface_session_id);
    assert_eq!(projected[0].surface_generation, surface.surface_generation);
    assert_eq!(projected[0].event, events[0]);
    assert!(
        app.project_agent_session_stream(&uuid::Uuid::now_v7().to_string(), &session, &events)
            .await
            .unwrap()
            .is_empty()
    );
    let other = uuid::Uuid::now_v7().to_string();
    assert!(
        app.project_agent_session_stream(
            &owner,
            &other,
            &[serde_json::json!({"conversation_id": other, "type": "text"})]
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert!(
        app.project_agent_session_stream(&owner, &other, &events)
            .await
            .is_err()
    );
    assert!(
        app.project_agent_session_stream(&owner, &session, &vec![events[0].clone(); 33])
            .await
            .is_err()
    );
    // Controlled corruption fixtures prove active release/lifecycle are checked,
    // not just ownership of a lingering Surface row.
    sqlx::query("UPDATE plugin_products SET active_release_epoch = active_release_epoch + 1 WHERE plugin_product_id = ?")
        .bind(&id).execute(db.pool()).await.unwrap();
    assert!(
        app.project_agent_session_stream(&owner, &session, &events)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("UPDATE plugin_products SET active_release_epoch = active_release_epoch - 1, lifecycle = 'disabled' WHERE plugin_product_id = ?")
        .bind(&id).execute(db.pool()).await.unwrap();
    assert!(
        app.project_agent_session_stream(&owner, &session, &events)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("UPDATE plugin_products SET lifecycle = 'enabled' WHERE plugin_product_id = ?")
        .bind(&id)
        .execute(db.pool())
        .await
        .unwrap();
    app.close_surface(
        &owner,
        &id,
        &surface.surface_session_id,
        &surface.surface_capability,
    )
    .await
    .unwrap();
    assert!(
        app.project_agent_session_stream(&owner, &session, &events)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn session_stream_rechecks_revocation_and_reopen_after_async_admission() {
    let (db, _root, service, owner) = fixture().await;
    let session = uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO conversations (conversation_id, user_id, name, type, created_at, updated_at) VALUES (?, ?, 'Stream race', 'nomi', 1, 1)")
        .bind(&session).bind(&owner).execute(db.pool()).await.unwrap();
    let port = Arc::new(StreamAdmission {
        pause: AtomicBool::new(false),
        entered: Notify::new(),
        release: Notify::new(),
    });
    service
        .application
        .install_agent_session_port(port.clone())
        .await;
    let mut value = draft();
    service.put_draft(&owner, &mut value).await.unwrap();
    let saved = service.save_draft(&owner, &mut value).await.unwrap();
    let id = saved.plugin.plugin_id;
    let digest = saved.plugin.releases.active.unwrap().release_digest;
    let surface = service
        .application
        .open_surface_with_agent_session(&owner, &id, Some((&session, &digest)))
        .await
        .unwrap();
    port.pause.store(true, Ordering::SeqCst);
    let task = tokio::spawn({
        let app = service.application.clone();
        let owner = owner.clone();
        let session = session.clone();
        async move {
            app.project_agent_session_stream(
                &owner,
                &session,
                &[serde_json::json!({"conversation_id": session, "type": "text"})],
            )
            .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), port.entered.notified())
        .await
        .unwrap();
    service
        .application
        .close_surface(
            &owner,
            &id,
            &surface.surface_session_id,
            &surface.surface_capability,
        )
        .await
        .unwrap();
    let reopened = service
        .application
        .open_surface_with_agent_session(&owner, &id, Some((&session, &digest)))
        .await
        .unwrap();
    assert_ne!(surface.surface_session_id, reopened.surface_session_id);
    port.release.notify_one();
    assert!(
        task.await.unwrap().unwrap().is_empty(),
        "old batch must not be transferred to the new Surface"
    );
}

struct PausedSessionRead {
    entered: Notify,
    release: Notify,
    fail: bool,
}

#[async_trait::async_trait]
impl PluginAgentSessionPort for PausedSessionRead {
    async fn authorize(&self, _: &str, _: &str) -> Result<(), PluginRuntimeApplicationError> {
        Ok(())
    }

    async fn request(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: PluginAgentSessionRequest,
    ) -> Result<StrictJsonValue, PluginRuntimeApplicationError> {
        self.entered.notify_one();
        self.release.notified().await;
        if self.fail {
            Err(PluginRuntimeApplicationError::AgentSession {
                status: 409,
                code: "SESSION_PRIVATE_STATE".into(),
                message: "private Session diagnostics".into(),
                details: None,
            })
        } else {
            Ok(StrictJsonValue(
                serde_json::json!({"content": "private Session history"}),
            ))
        }
    }
}

#[tokio::test]
async fn surface_revocation_during_session_read_withholds_success_and_private_error() {
    for fail in [false, true] {
        let (db, _root, service, owner) = fixture().await;
        let session_id = uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO conversations (conversation_id, user_id, name, type, created_at, updated_at) VALUES (?, ?, 'Scoped view', 'nomi', 1, 1)")
            .bind(&session_id).bind(&owner).execute(db.pool()).await.unwrap();
        let port = Arc::new(PausedSessionRead {
            entered: Notify::new(),
            release: Notify::new(),
            fail,
        });
        service
            .application
            .install_agent_session_port(port.clone())
            .await;
        let mut value = draft();
        service.put_draft(&owner, &mut value).await.unwrap();
        let saved = service.save_draft(&owner, &mut value).await.unwrap();
        let id = saved.plugin.plugin_id;
        let digest = saved.plugin.releases.active.unwrap().release_digest;
        let surface = service
            .application
            .open_surface_with_agent_session(&owner, &id, Some((&session_id, &digest)))
            .await
            .unwrap();
        let task = tokio::spawn({
            let application = service.application.clone();
            let owner = owner.clone();
            let id = id.clone();
            let surface = surface.clone();
            async move {
                application.surface_bridge_request(&owner, &id, &surface.surface_capability,
                    surface.active_release_epoch, &surface.expected_release_digest,
                    serde_json::from_value(serde_json::json!({"call_id": "observe", "target": {
                        "target": "agent_session", "request": {"operation": "observe", "after_seq": 0, "limit": 100}
                    }})).unwrap()).await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), port.entered.notified())
            .await
            .unwrap();
        service
            .application
            .close_surface(
                &owner,
                &id,
                &surface.surface_session_id,
                &surface.surface_capability,
            )
            .await
            .unwrap();
        port.release.notify_one();
        assert!(
            matches!(
                task.await.unwrap(),
                Err(PluginRuntimeApplicationError::NotFound)
            ),
            "revoked view must not receive either a private result or private error"
        );
    }
}
