//! Owner-scoped, side-effect-free reads of committed collaboration records.
use super::*;

pub(super) async fn get(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<nomifun_api_types::DelegationRecordsQuery>,
) -> Result<Json<ApiResponse<nomifun_api_types::DelegationRecordsResponse>>, NomiCoreApiError> {
    let session = parse_agent_session_id(&agent_session_id)?;
    state.session_owner.canonical().session_record(&authenticated_principal(&owner), &session).await?;
    if query.after_session_seq.is_some_and(|seq|seq<0) || query.through_session_seq.is_some_and(|seq|seq<0)
        || query.cursor.as_ref().is_some_and(|cursor|cursor.len()>131072) {
        return Err(AppError::BadRequest("invalid delegation history cursor".into()).into());
    }
    let roots=query.source_message_ids.as_deref().map(|ids| {
        if ids.len()>37*100 {return Err(AppError::BadRequest("at most 100 source request identities may be requested".into()));}
        let mut roots=Vec::new();
        for id in ids.split(',') {
            nomifun_common::validate_uuidv7(id).map_err(|error|AppError::BadRequest(error.to_string()))?;
            if !roots.iter().any(|root|root==id) {roots.push(id.to_owned());}
        }
        Ok(roots)
    }).transpose()?;
    let (records,through_session_seq,next_cursor)=nomifun_db::read_delegation_records(
        &state.session_owner.pool,owner.as_ref(),session.as_ref(),roots.as_deref(),query.after_session_seq.unwrap_or(0),
        query.through_session_seq,query.cursor.as_deref(),query.limit.unwrap_or(100),
    ).await.map_err(|error|AppError::Internal(error.to_string()))?;
    Ok(Json(ApiResponse::ok(nomifun_api_types::DelegationRecordsResponse {records,through_session_seq,next_cursor})))
}
