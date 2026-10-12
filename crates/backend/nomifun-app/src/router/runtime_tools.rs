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
use nomifun_runtime::{ToolError, ToolId, ToolPreparationSnapshot, ToolSource, VerifiedTool};
use serde::Serialize;

#[derive(Serialize)]
struct ToolAvailability {
    tool: ToolId,
    available: bool,
    source: Option<ToolSource>,
}

pub(crate) fn runtime_tool_routes() -> Router {
    Router::new()
        .route("/api/runtime-tools/{tool}", get(status))
        .route("/api/runtime-tools/{tool}/availability", get(availability))
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
/// Observe a system executable or an already downloaded, verified component.
/// This endpoint does not start or wait for a component download.
async fn availability(
    Extension(_user): Extension<CurrentUser>,
    Path(tool): Path<String>,
) -> Result<Json<ApiResponse<ToolAvailability>>, AppError> {
    let tool = tool_id(&tool)?;
    let resolved = nomifun_runtime::resolve_existing_tool(tool).await;
    Ok(Json(ApiResponse::ok(classify_availability(tool, resolved)?)))
}

fn classify_availability(
    tool: ToolId,
    resolved: Result<VerifiedTool, ToolError>,
) -> Result<ToolAvailability, AppError> {
    match resolved {
        Ok(resolved) => Ok(ToolAvailability { tool, available: true, source: Some(resolved.source) }),
        Err(error) if is_missing_component(&error) => Ok(ToolAvailability { tool, available: false, source: None }),
        Err(error) => Err(AppError::ComponentUnavailable {
            component: tool.as_str().into(), reason: error.to_string(),
        }),
    }
}

fn is_missing_component(error: &ToolError) -> bool {
    match error {
        ToolError::NotPrepared | ToolError::Unavailable { .. } => true,
        ToolError::Shared(error) => is_missing_component(error),
        _ => false,
    }
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

    #[tokio::test]
    async fn availability_observation_never_starts_component_preparation() {
        let before = nomifun_runtime::tool_preparation_snapshot(ToolId::Bun);
        let user = CurrentUser {
            id: nomifun_common::UserId::new(),
            username: "availability-observer".into(),
        };
        let response = availability(Extension(user), Path("bun".into())).await.unwrap();
        let wire = serde_json::to_value(response.0).unwrap();
        assert_eq!(wire["data"]["tool"], "bun");
        assert!(wire["data"]["available"].is_boolean());
        assert_eq!(
            wire["data"]["source"].is_null(),
            !wire["data"]["available"].as_bool().unwrap()
        );
        assert_eq!(nomifun_runtime::tool_preparation_snapshot(ToolId::Bun), before);
    }

    #[test]
    fn availability_only_offers_download_for_an_actually_missing_component() {
        for error in [
            ToolError::NotPrepared,
            ToolError::Shared(std::sync::Arc::new(ToolError::NotPrepared)),
            ToolError::Unavailable { tool: "bun", override_variable: "NOMIFUN_BUN_PATH" },
        ] {
            let availability = classify_availability(ToolId::Bun, Err(error)).unwrap();
            assert!(!availability.available);
            assert!(availability.source.is_none());
        }
        for error in [
            ToolError::Verification { tool: "bun", reason: "invalid system version".into() },
            ToolError::Changed { tool: "bun" },
            ToolError::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "cannot read runtime")),
            ToolError::Shared(std::sync::Arc::new(ToolError::Changed { tool: "bun" })),
        ] {
            let error = classify_availability(ToolId::Bun, Err(error)).err().unwrap();
            assert_eq!(error.error_code(), "COMPONENT_UNAVAILABLE");
            assert_eq!(error.error_details().unwrap()["component"], "bun");
        }
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
