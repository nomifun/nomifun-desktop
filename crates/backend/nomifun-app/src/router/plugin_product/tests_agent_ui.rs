use super::*;
use nomifun_agent_contracts::{PluginAgentSessionRequest, StrictJsonValue};
use nomifun_plugin_platform::runtime::{PluginAgentSessionPort, PluginRuntimeApplicationError};
use tokio::sync::Notify;

#[test]
fn authoring_agent_view_requires_html_but_not_a_service() {
    let declaration = serde_json::json!({"agent_view": {"name": "My Agent", "description": "My Session page"}});
    assert!(super::super::authoring::validate_plugin_source(HTML, None, Some(&declaration)).is_ok());
    assert!(super::super::authoring::validate_plugin_source("", Some("export function start() {}"), Some(&declaration)).is_err());
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
        let SaveOutcome::Saved(saved) = service.save_draft(&owner, &mut value, None).await.unwrap() else { panic!("UI-only save unexpectedly requested Service input") };
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
