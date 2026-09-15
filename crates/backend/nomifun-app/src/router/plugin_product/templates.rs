//! Templates are ordinary recoverable drafts, never implicit publications or grants.
use super::*;

const AGENT_SESSION_HTML: &str = include_str!("templates/agent-session.html");
const BEFORE_TOOL_SERVICE: &str = include_str!("templates/before-tool.mjs");

pub(super) async fn before_tool(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Draft>>, AppError> {
    let service = service(&state)?;
    Ok(Json(ApiResponse::ok(create_before_tool_draft(&service, user.id.as_str()).await?)))
}

async fn create_before_tool_draft(service: &PluginProductService, owner: &str) -> Result<Draft, AppError> {
    let descriptor = nomifun_agent_contracts::tool_middleware::before_action();
    let schemas = nomifun_agent_contracts::tool_middleware::schemas();
    let mut draft = Draft {
        service_test_confirmation: None,
        id: uuid::Uuid::now_v7().to_string(), revision: 0,
        name: "敏感文件检查".into(),
        description: "在 Nomi 执行工具前检查参数，阻止直接访问 .env 和私钥文件。发布后需主动选入 Agent，可按业务规则修改。".into(),
        html: include_str!("templates/before-tool.html").into(),
        service_source: Some(BEFORE_TOOL_SERVICE.into()),
        source_manifest: Some(serde_json::json!({"actions":[{
            "id":descriptor.action_id,"name":"敏感文件检查",
            "description":"工具执行前检查脱敏参数；禁止直接引用 .env、id_rsa、id_ed25519。只作为业务检查示例，不替代平台权限或完整的命令安全策略。仅支持 Nomi。",
            "input_schema":schemas[&descriptor.input_schema],
            "output_schema":schemas[&descriptor.output_schema],"effect":"pure"
        }]})),
        messages: vec![], status: "ready".into(), error: None, plugin_id: None,
        base_release_digest: None, base_source_digest: None,
        updated_at: nomifun_common::now_ms(), import: None,
    };
    authoring::validate_plugin_source(&draft.html, draft.service_source.as_deref(), draft.source_manifest.as_ref())?;
    service.put_draft(owner, &mut draft).await?;
    Ok(draft)
}

pub(super) async fn agent_session_view(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Draft>>, AppError> {
    let service = service(&state)?;
    Ok(Json(ApiResponse::ok(create_agent_session_draft(&service, user.id.as_str()).await?)))
}

async fn create_agent_session_draft(service: &PluginProductService, owner: &str) -> Result<Draft, AppError> {
    let mut draft = Draft {
        service_test_confirmation: None,
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
    async fn before_tool_draft_is_private_and_does_not_publish_or_run_service() {
        let (_db, _root, service, owner) = super::super::tests::fixture().await;
        let draft = create_before_tool_draft(&service, &owner).await.unwrap();
        assert_eq!(draft.status, "ready");
        assert!(draft.service_source.is_some());
        assert!(service.draft(&uuid::Uuid::now_v7().to_string(), &draft.id).await.is_err());
        assert!(service.application.library(&owner).await.unwrap().plugins.is_empty());
        assert!(draft.plugin_id.is_none());
        // The real Node publication path is tested through the public routes
        // in plugin_product_discovery::before_tool, not this no-runtime fixture.
    }

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
        let SaveOutcome::Saved(saved) = service.save_draft(&owner, &mut draft, None).await.unwrap() else { panic!("UI-only save unexpectedly requested Service input") };
        assert!(saved.plugin.releases.active.is_some());
        let source = service.application.source_file(&owner, &saved.plugin.plugin_id, "ui/index.html").await.unwrap();
        assert_eq!(source.content, AGENT_SESSION_HTML);
    }
}
