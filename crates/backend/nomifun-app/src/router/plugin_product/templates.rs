//! Templates are ordinary recoverable drafts, never implicit publications or grants.
use super::*;

const AGENT_SESSION_HTML: &str = include_str!("templates/agent-session.html");

pub(super) async fn agent_session_view(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Draft>>, AppError> {
    let service = service(&state)?;
    Ok(Json(ApiResponse::ok(create_agent_session_draft(&service, user.id.as_str()).await?)))
}

async fn create_agent_session_draft(service: &PluginProductService, owner: &str) -> Result<Draft, AppError> {
    let mut draft = Draft {
        id: uuid::Uuid::now_v7().to_string(),
        revision: 0,
        name: "Agent Session reference".into(),
        description: "Editable Session page: persisted history, text input and cancellation. Select explicitly from a Session after saving.".into(),
        html: AGENT_SESSION_HTML.into(),
        service_source: None,
        source_manifest: Some(serde_json::json!({"agent_view": {
            "name": "Agent Session reference",
            "description": "Persisted history, text input and cancellation via the existing Session owner"
        }})),
        messages: vec![],
        status: "ready".into(),
        error: None,
        plugin_id: None,
        base_release_digest: None,
        base_source_digest: None,
        updated_at: nomifun_common::now_ms(),
        import: None,
    };
    authoring::validate_plugin_source(&draft.html, None, draft.source_manifest.as_ref())?;
    service.put_draft(owner, &mut draft).await?;
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reference_draft_is_owned_editable_and_only_explicit_save_publishes() {
        let (_db, _root, service, owner) = super::super::tests::fixture().await;
        let mut draft = create_agent_session_draft(&service, &owner).await.unwrap();
        let other = create_agent_session_draft(&service, &owner).await.unwrap();
        assert_ne!(draft.id, other.id);
        valid_id(&draft.id).unwrap();
        assert_eq!(draft.status, "ready");
        assert_eq!(draft.revision, 1);
        assert!(draft.service_source.is_none() && draft.import.is_none());
        assert_eq!(service.draft(&owner, &draft.id).await.unwrap().html, AGENT_SESSION_HTML);
        assert!(service.draft(&uuid::Uuid::now_v7().to_string(), &draft.id).await.is_err());
        assert!(service.application.library(&owner).await.unwrap().plugins.is_empty());
        let saved = service.save_draft(&owner, &mut draft).await.unwrap();
        assert!(saved.plugin.releases.active.is_some());
        let source = service.application.source_file(&owner, &saved.plugin.plugin_id, "ui/index.html").await.unwrap();
        assert_eq!(source.content, AGENT_SESSION_HTML);
    }
}
