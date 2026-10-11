//! Authenticated preparation of the three manifest-approved host components.
//! No caller path, URL, version or global cancellation is accepted here.
use axum::{
    Json, Router,
    extract::{Extension, Path},
    routing::{get, post},
};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_runtime::{ToolId, ToolPreparationSnapshot};

pub(crate) fn runtime_tool_routes() -> Router {
    Router::new()
        .route("/api/runtime-tools/{tool}", get(status))
        .route("/api/runtime-tools/{tool}/prepare", post(begin))
}
fn tool_id(value: &str) -> Result<ToolId, AppError> {
    match value {
        "bun" => Ok(ToolId::Bun),
        "node" => Ok(ToolId::Node),
        "officecli" => Ok(ToolId::OfficeCli),
        _ => Err(AppError::BadRequest(
            "Unknown managed component. Expected bun, node or officecli.".into(),
        )),
    }
}
async fn status(
    Extension(_user): Extension<CurrentUser>,
    Path(tool): Path<String>,
) -> Result<Json<ApiResponse<ToolPreparationSnapshot>>, AppError> {
    Ok(Json(ApiResponse::ok(
        nomifun_runtime::tool_preparation_snapshot(tool_id(&tool)?),
    )))
}
async fn begin(
    Extension(_user): Extension<CurrentUser>,
    Path(tool): Path<String>,
) -> Result<Json<ApiResponse<ToolPreparationSnapshot>>, AppError> {
    Ok(Json(ApiResponse::ok(
        nomifun_runtime::begin_tool_preparation(tool_id(&tool)?),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn status_observation_never_starts_component_preparation() {
        let before = nomifun_runtime::tool_preparation_snapshot(ToolId::Bun);
        let user = CurrentUser {
            id: nomifun_common::UserId::new(),
            username: "status-observer".into(),
        };
        let response = status(Extension(user), Path("bun".into())).await.unwrap();
        let wire = serde_json::to_value(response.0).unwrap();
        assert_eq!(wire["data"]["tool"], "bun");
        assert_eq!(wire["data"]["revision"], before.revision);
        assert_eq!(nomifun_runtime::tool_preparation_snapshot(ToolId::Bun), before);
    }

    #[test]
    fn preparation_accepts_only_fixed_manifest_components() {
        for name in ["bun", "node", "officecli"] {
            assert!(tool_id(name).is_ok());
        }
        for name in [
            "https://unapproved.invalid/tool",
            "/usr/bin/node",
            "../bun",
            "node@latest",
            "Bun",
        ] {
            assert!(tool_id(name).is_err());
        }
    }
}
