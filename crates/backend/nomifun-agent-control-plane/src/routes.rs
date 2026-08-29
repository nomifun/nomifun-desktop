use std::sync::Arc;
use std::ops::Deref;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use nomifun_agent_contracts::UserId;
use nomifun_api_types::{
    AgentBindingRecordDto, AgentPresetEditorResponse, AgentPresetLibraryResponse, ApiResponse,
    CapabilityCatalogItemDto, CreateAgentPresetFromTemplateRequest, CreateAgentPresetRequest,
    CreateRemoteBindingRequest, McpToolCatalogItemDto, PutAgentBindingRequest, RemoteBindingDto,
    ResolveAgentPresetPreviewRequest, ResolveAgentPresetPreviewResponse,
    ResolveSavedRevisionPreviewRequest, SaveAgentPresetRevisionRequest,
    SaveAgentPresetRevisionResponse, SkillCatalogItemDto, UpdateRemoteBindingRequest,
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
    Router::new()
        .route("/api/agent-preset-templates", get(list_official_templates))
        .route("/api/capabilities", get(list_capabilities))
        .route("/api/skills", get(list_skills))
        .route("/api/mcp-tool-mappings", get(list_mcp_tools))
        .route("/api/agent-presets", post(create_preset))
        .route(
            "/api/agent-presets/from-template/{template_id}",
            post(create_from_template),
        )
        .route(
            "/api/agent-presets/{preset_id}/editor",
            get(get_editor),
        )
        .route(
            "/api/agent-presets/{preset_id}/resolve-preview",
            post(resolve_preview),
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
            "/api/agent-presets/{preset_id}/revisions/{revision}/resolve-preview",
            post(resolve_saved_revision_preview),
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
        )
        .with_state(control_plane)
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

async fn resolve_preview(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(preset_id): Path<String>,
    Json(request): Json<ResolveAgentPresetPreviewRequest>,
) -> Result<Json<ApiResponse<ResolveAgentPresetPreviewResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane.preview(&owner, &preset_id, request).await?,
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

async fn resolve_saved_revision_preview(
    State(control_plane): State<Arc<AgentControlPlane>>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((preset_id, revision)): Path<(String, u64)>,
    Json(request): Json<ResolveSavedRevisionPreviewRequest>,
) -> Result<Json<ApiResponse<ResolveAgentPresetPreviewResponse>>, ControlPlaneError> {
    Ok(Json(ApiResponse::ok(
        control_plane
            .preview_saved_revision(&owner, &preset_id, revision, request)
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
    #[test]
    fn router_source_has_no_editor_test_endpoint() {
        let source = include_str!("routes.rs");
        assert!(!source.contains(&("/api/".to_owned() + "test")));
        assert!(!source.contains(&("/test-".to_owned() + "sessions")));
        assert!(!source.contains(&("/api/agent-".to_owned() + "sessions")));
    }
}
