use std::sync::Arc;
use std::ops::Deref;

use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use nomifun_agent_contracts::UserId;
use nomifun_api_types::{
    AgentBindingRecordDto, AgentCatalogResponse, AgentPresetEditorResponse, AgentPresetLibraryResponse,
    AgentPresetRevisionImpactResponse, ApiResponse, CapabilityCatalogItemDto,
    CreateAgentPresetFromTemplateRequest, CreateAgentPresetRequest, CreateRemoteBindingRequest,
    McpToolCatalogItemDto, PutAgentBindingRequest, RemoteBindingDto,
    SaveAgentPresetRevisionRequest, SaveAgentPresetRevisionResponse, SkillCatalogItemDto,
    UpdateRemoteBindingRequest,
};
use serde::Deserialize;

use crate::{AgentControlPlane, ControlPlaneError};

#[derive(Clone, Debug)]
pub struct AuthenticatedOwner(pub UserId);

impl Deref for AuthenticatedOwner {
    type Target = UserId;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Deserialize)]
struct OfficialTemplateQuery {
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EditorQuery {
    revision: Option<u64>,
}

pub fn control_plane_router(control_plane: Arc<AgentControlPlane>) -> Router {
    control_plane_router_with_legacy_skill_route(control_plane, true)
}

/// Build the control-plane routes without claiming the legacy `/api/skills`
/// endpoint.  The current Nomi-core application still exposes the historical
/// skill-management API at that path, so its canonical Agent Settings catalog
/// uses `/api/agent-catalog/skills` instead.
pub fn control_plane_router_without_legacy_skills(
    control_plane: Arc<AgentControlPlane>,
) -> Router {
    control_plane_router_with_legacy_skill_route(control_plane, false)
}

fn control_plane_router_with_legacy_skill_route(
    control_plane: Arc<AgentControlPlane>,
    include_legacy_skill_route: bool,
) -> Router {
    let router = Router::new()
        .route("/api/agent-catalog", get(get_catalog))
        .route("/api/agent-catalog/ui/agent-session", get(get_agent_ui_contributions))
        .route("/api/agent-role-defaults", get(get_role_defaults))
        .route("/api/agent-role-defaults/{role_id}", put(put_role_default))
        .route("/api/agent-preset-templates", get(list_official_templates))
        .route("/api/capabilities", get(list_capabilities))
        .route("/api/mcp-tool-mappings", get(list_mcp_tools))
        .route("/api/agent-presets", post(create_preset))
        .route("/api/agent-presets/{preset_id}/ui-binding", get(get_ui_binding).put(put_ui_binding))
        .route(
            "/api/agent-presets/{preset_id}",
            delete(retire_preset),
        )
        .route(
            "/api/agent-presets/from-template/{template_id}",
            post(create_from_template),
        )
        .route(
            "/api/agent-presets/{preset_id}/editor",
            get(get_editor),
        )
        .route(
            "/api/agent-presets/{preset_id}/revisions",
            post(save_revision),
        )
        .route(
            "/api/agent-presets/{preset_id}/revisions/{revision}",
            get(get_revision),
        )
        .route(
            "/api/agent-presets/{preset_id}/revisions/{revision}/impact",
            get(get_revision_impact),
        )
        .route(
            "/api/agent-bindings/{target_kind}/{target_id}",
            get(get_agent_binding).put(put_agent_binding),
        )
        .route(
            "/api/remote-bindings",
            get(list_remote_bindings).post(create_remote_binding),
        )
        .route(
            "/api/remote-bindings/{binding_id}",
            put(update_remote_binding).delete(delete_remote_binding),
        );
    let router = if include_legacy_skill_route {
        router
            .route("/api/skills", get(list_skills))
            .route("/api/agent-catalog/skills", get(list_skills))
    } else {
        router.route("/api/agent-catalog/skills", get(list_skills))
    };
    router.with_state(control_plane)
}

async fn list_official_templates(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Query(query): Query<OfficialTemplateQuery>,
) -> Result<Json<ApiResponse<AgentPresetLibraryResponse>>, ControlPlaneError> {
    if query.source.as_deref().is_some_and(|source| source != "official") {
        return Err(ControlPlaneError::canonical(
            "OFFICIAL_PRESET_KEY_SET_MISMATCH",
            axum::http::StatusCode::BAD_REQUEST,
            "only source=official is supported by the canonical template API",
        ));
    }
    Ok(Json(ApiResponse::ok(control_plane.library(&owner).await?)))
}

async fn get_catalog(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(_owner): Extension<AuthenticatedOwner>,
) -> Result<Json<ApiResponse<AgentCatalogResponse>>, ControlPlaneError> {
    // One materialization for capability members and exact Provider candidates.
    Ok(Json(ApiResponse::ok(control_plane.catalog()?)))
}

async fn get_agent_ui_contributions(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(_owner): Extension<AuthenticatedOwner>,
) -> Result<Json<ApiResponse<Vec<nomifun_api_types::AgentUiContributionDto>>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(control_plane.agent_ui_contributions()?)))
}

async fn get_ui_binding(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>, Path(preset_id): Path<String>,
) -> Result<Json<ApiResponse<nomifun_api_types::AgentPresetUiBindingResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(control_plane.ui_binding(&owner, &preset_id).await?)))
}

async fn put_ui_binding(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>, Path(preset_id): Path<String>,
    Json(request): Json<nomifun_api_types::PutAgentUiBindingRequest>,
) -> Result<Json<ApiResponse<nomifun_api_types::AgentPresetUiBindingResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(control_plane.put_ui_binding(&owner, &preset_id, request).await?)))
}

async fn get_role_defaults(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
) -> Result<Json<ApiResponse<Vec<nomifun_api_types::InstallationRoleBindingDto>>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(control_plane.role_defaults(&owner).await?)))
}

async fn put_role_default(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(role_id): Path<String>,
    Json(request): Json<nomifun_api_types::PutAgentRoleDefaultRequest>,
) -> Result<Json<ApiResponse<nomifun_api_types::InstallationRoleBindingDto>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(control_plane.put_role_default(&owner, &role_id, request).await?)))
}

async fn list_capabilities(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(_owner): Extension<AuthenticatedOwner>,
) -> Result<Json<ApiResponse<Vec<CapabilityCatalogItemDto>>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane.catalog()?.capabilities,
    )))
}

async fn list_skills(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(_owner): Extension<AuthenticatedOwner>,
) -> Result<Json<ApiResponse<Vec<SkillCatalogItemDto>>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(control_plane.catalog()?.skills)))
}

async fn list_mcp_tools(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(_owner): Extension<AuthenticatedOwner>,
) -> Result<Json<ApiResponse<Vec<McpToolCatalogItemDto>>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(control_plane.catalog()?.mcp_tools)))
}

async fn create_preset(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Json(request): Json<CreateAgentPresetRequest>,
) -> Result<Json<ApiResponse<AgentPresetEditorResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane.create_preset(&owner, request).await?,
    )))
}

async fn retire_preset(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(preset_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ControlPlaneError> {
    control_plane.retire_preset(&owner, &preset_id).await?;
    Ok(Json(ApiResponse::success()))
}

async fn create_from_template(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(template_id): Path<String>,
    Json(request): Json<CreateAgentPresetFromTemplateRequest>,
) -> Result<Json<ApiResponse<AgentPresetEditorResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .create_from_template(&owner, &template_id, request)
            .await?,
    )))
}

async fn get_editor(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(preset_id): Path<String>,
    Query(query): Query<EditorQuery>,
) -> Result<Json<ApiResponse<AgentPresetEditorResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .editor(&owner, &preset_id, query.revision)
            .await?,
    )))
}

async fn save_revision(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(preset_id): Path<String>,
    Json(request): Json<SaveAgentPresetRevisionRequest>,
) -> Result<Json<ApiResponse<SaveAgentPresetRevisionResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .save_revision(&owner, &preset_id, request)
            .await?,
    )))
}

async fn get_revision(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((preset_id, revision)): Path<(String, u64)>,
) -> Result<Json<ApiResponse<nomifun_api_types::AgentPresetRevisionDto>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .get_revision(&owner, &preset_id, revision)
            .await?,
    )))
}

async fn get_revision_impact(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((preset_id, revision)): Path<(String, u64)>,
) -> Result<Json<ApiResponse<AgentPresetRevisionImpactResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .revision_impact(&owner, &preset_id, revision)
            .await?,
    )))
}

async fn get_agent_binding(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((target_kind, target_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<Option<AgentBindingRecordDto>>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .get_agent_binding(&owner, target_kind, target_id)
            .await?,
    )))
}

async fn put_agent_binding(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((target_kind, target_id)): Path<(String, String)>,
    Json(request): Json<PutAgentBindingRequest>,
) -> Result<Json<ApiResponse<AgentBindingRecordDto>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .put_agent_binding(&owner, target_kind, target_id, request)
            .await?,
    )))
}

async fn list_remote_bindings(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
) -> Result<Json<ApiResponse<Vec<RemoteBindingDto>>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane.list_remote_bindings(&owner).await?,
    )))
}

async fn create_remote_binding(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Json(request): Json<CreateRemoteBindingRequest>,
) -> Result<Json<ApiResponse<RemoteBindingDto>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .create_remote_binding(&owner, request)
            .await?,
    )))
}

async fn update_remote_binding(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(binding_id): Path<String>,
    Json(request): Json<UpdateRemoteBindingRequest>,
) -> Result<Json<ApiResponse<RemoteBindingDto>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .update_remote_binding(&owner, &binding_id, request)
            .await?,
    )))
}

async fn delete_remote_binding(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(binding_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ControlPlaneError> {
    control_plane
        .delete_remote_binding(&owner, &binding_id)
        .await?;
    Ok(Json(ApiResponse::success()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use nomifun_agent_contracts::{AgentPreset, AgentPresetId, AgentPresetSource};
    use tower::ServiceExt;

    use crate::{
        ControlPlaneStore, InMemoryControlPlaneStore, OfficialTemplateCatalog,
        PresetRevisionCompiler, StaticCatalogProvider, StoredPreset,
    };

    fn test_control_plane(
        store: Arc<InMemoryControlPlaneStore>,
    ) -> Arc<AgentControlPlane> {
        let templates = OfficialTemplateCatalog::load().unwrap();
        Arc::new(AgentControlPlane::new(
            store,
            Arc::new(StaticCatalogProvider::new(Default::default())),
            templates.clone(),
            PresetRevisionCompiler::new(templates),
        ))
    }

    #[test]
    fn router_source_has_no_editor_test_endpoint() {
        let source = include_str!("routes.rs");
        assert!(!source.contains(&("/api/".to_owned() + "test")));
        assert!(!source.contains(&("/test-".to_owned() + "sessions")));
        assert!(!source.contains(&("/api/agent-".to_owned() + "sessions")));
        assert!(!source.contains(&["resolve", "-preview"].concat()));
        assert!(source.contains(
            "/api/agent-presets/{preset_id}/revisions/{revision}/impact"
        ));
        let handler = source
            .split("async fn get_revision_impact")
            .nth(1)
            .expect("revision impact handler");
        assert!(
            handler.contains("Extension(owner): Extension<AuthenticatedOwner>"),
            "impact inspection must remain authenticated and owner-scoped"
        );
    }

    #[tokio::test]
    async fn catalog_route_returns_one_complete_projection_and_requires_owner_context() {
        let router = control_plane_router(test_control_plane(Arc::new(InMemoryControlPlaneStore::new())));
        let request = || Request::builder().uri("/api/agent-catalog").body(Body::empty()).unwrap();
        let missing_owner = router.clone().oneshot(request()).await.unwrap();
        assert!(!missing_owner.status().is_success());
        let response = router.layer(Extension(AuthenticatedOwner(UserId::from("owner-1"))))
            .oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        for key in ["capabilities", "skills", "mcp_tools", "roles"] {
            assert_eq!(value["data"][key], serde_json::json!([]));
        }
    }

    #[tokio::test]
    async fn delete_agent_preset_route_retires_once_and_returns_canonical_not_found_afterward() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("owner-1");
        let preset_id = AgentPresetId::from("preset-1");
        store
            .insert_preset(StoredPreset {
                session_only: false,
                preset: AgentPreset {
                    preset_id: preset_id.clone(),
                    owner_user_id: Some(owner.clone()),
                    source: AgentPresetSource::User,
                    display_name: "Preset".to_owned(),
                    description: None,
                    current_stable_revision: None,
                },
            })
            .await
            .unwrap();
        let router = control_plane_router(test_control_plane(store.clone()))
            .layer(Extension(AuthenticatedOwner(owner)));

        let retired = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/agent-presets/preset-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retired.status(), StatusCode::OK);
        assert!(store.get_preset(&preset_id).await.unwrap().is_none());

        let repeated = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/agent-presets/preset-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(repeated.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(repeated.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let error: nomifun_api_types::ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.code, "AGENT_PRESET_NOT_FOUND");
    }
}
