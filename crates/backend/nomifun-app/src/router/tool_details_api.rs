//! Owner-scoped invocation inspection, without a runtime or process operation.
use super::*;

pub(super) async fn get(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<nomifun_api_types::ToolDetailQuery>,
) -> Result<Json<ApiResponse<nomifun_api_types::ToolDetailPage>>, NomiCoreApiError> {
    let session = parse_agent_session_id(&agent_session_id)?;
    let principal = authenticated_principal(&owner);
    state.session_owner.canonical().session_record(&principal, &session).await?;
    let page = state.session_owner.canonical().store().tool_detail_page(&principal, &session, &query).await
        .map_err(detail_error)?;
    Ok(Json(ApiResponse::ok(page)))
}

fn detail_error(error: nomifun_agent_session::SessionStoreError) -> NomiCoreApiError {
    use nomifun_agent_session::SessionStoreError as Error;
    let (status, code) = match &error {
        Error::NotFound(_) => (StatusCode::NOT_FOUND, "TOOL_DETAIL_NOT_FOUND"),
        Error::Deleted(_) => (StatusCode::CONFLICT, "SESSION_DELETED"),
        Error::InvalidSession(_) => (StatusCode::BAD_REQUEST, "TOOL_DETAIL_QUERY_INVALID"),
        Error::Conflict(_) => (StatusCode::CONFLICT, "TOOL_DETAIL_SOURCE_CHANGED"),
        Error::InvalidEvent(_) | Error::InvalidPayload(_) | Error::Json(_) | Error::Digest(_) =>
            (StatusCode::CONFLICT, "TOOL_DETAIL_SOURCE_INVALID"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "TOOL_DETAIL_READ_FAILED"),
    };
    NomiCoreApiError::new(status, code, error.to_string())
}
