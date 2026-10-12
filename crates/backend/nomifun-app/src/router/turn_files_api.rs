//! Authenticated presentation reads, independent of transcript paging.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TurnFilesQuery { turn_ids: String }

pub(super) async fn get(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<TurnFilesQuery>,
) -> Result<Json<ApiResponse<Vec<nomifun_api_types::TurnFileSummary>>>, NomiCoreApiError> {
    let session = parse_agent_session_id(&agent_session_id)?;
    if query.turn_ids.len() > 37 * 100 {
        return Err(AppError::BadRequest("at most 100 Turn identities may be requested".into()).into());
    }
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    for root in query.turn_ids.split(',') {
        nomifun_common::validate_uuidv7(root).map_err(|error| AppError::BadRequest(error.to_string()))?;
        if seen.insert(root.to_owned()) { roots.push(root.to_owned()); }
    }
    let record = state.session_owner.canonical().session_record(&authenticated_principal(&owner), &session).await?;
    let workspace = frozen_workspace_root(&state.session_owner.managed_workspace_root,
        owner.as_ref(), &session, &record.agent_binding)?;
    let summaries = crate::router::turn_files::query(
        state.session_owner.canonical().store(), &session, &roots,
        workspace.as_deref(), &state.session_owner.managed_workspace_root,
    ).await?;
    Ok(Json(ApiResponse::ok(summaries)))
}
