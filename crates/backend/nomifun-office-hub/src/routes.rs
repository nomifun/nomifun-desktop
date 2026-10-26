use std::sync::Arc;
use axum::{Json, Router, extract::{DefaultBodyLimit, Extension, Path, State},
    routing::{get, post, put}};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use crate::*;

pub fn office_hub_routes(service: Arc<OfficeHubService>) -> Router {
    Router::new()
        .route("/api/office-hub/status", get(status))
        .route("/api/office-hub/profiles", get(profiles))
        .route("/api/office-hub/runtime/prepare", post(prepare))
        .route("/api/office-hub/runtime/select", post(select))
        .route("/api/office-hub/runtime/check-update", post(check_update))
        .route("/api/office-hub/connection", put(connection))
        .route("/api/office-hub/auth", post(start_auth))
        .route("/api/office-hub/auth/logout", post(logout))
        .route("/api/office-hub/auth/{id}", get(auth).delete(cancel_auth))
        .route("/api/office-hub/query", post(query))
        .route("/api/office-hub/operations", post(execute))
        .route("/api/office-hub/operations/request/{request_id}", get(operation_by_request))
        .route("/api/office-hub/operations/{id}", get(operation))
        .route("/api/office-hub/operations/{id}/reconcile", post(reconcile))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(service)
}
type HubState = State<Arc<OfficeHubService>>;
type User = Extension<CurrentUser>;
type Reply<T> = Result<Json<ApiResponse<T>>, HubError>;
async fn status(State(service): HubState, _:User) -> Reply<OfficeStatus> {
    Ok(Json(ApiResponse::ok(service.status().await?)))
}
async fn profiles(State(service):HubState,_:User)->Reply<Vec<OfficeProfile>>{
    Ok(Json(ApiResponse::ok(service.profiles().await?)))
}
async fn prepare(State(service):HubState,_:User)->Reply<RuntimeInfo>{
    Ok(Json(ApiResponse::ok(service.prepare_runtime().await?)))
}
async fn select(State(service):HubState,_:User,Json(request):Json<RuntimeSelection>)->Reply<RuntimeInfo>{
    Ok(Json(ApiResponse::ok(service.select_runtime(request).await?)))
}
async fn check_update(State(service):HubState,_:User)->Reply<RuntimeUpdateInfo>{
    Ok(Json(ApiResponse::ok(service.check_update().await?)))
}
async fn connection(State(service):HubState,_:User,Json(request):Json<ConnectionUpdate>)->Reply<OfficeConnection>{
    Ok(Json(ApiResponse::ok(service.update_connection(request).await?)))
}
async fn start_auth(State(service):HubState,_:User,Json(request):Json<AuthStartRequest>)->Reply<AuthorizationFlow>{
    Ok(Json(ApiResponse::ok(service.start_authorization(request).await?)))
}
async fn logout(State(service):HubState,_:User,Json(request):Json<LogoutRequest>)->Reply<()>{
    service.logout(request).await?;
    Ok(Json(ApiResponse::ok(())))
}
async fn auth(State(service):HubState,_:User,Path(id):Path<String>)->Reply<AuthorizationFlow>{
    Ok(Json(ApiResponse::ok(service.authorization(&id)?)))
}
async fn cancel_auth(State(service):HubState,_:User,Path(id):Path<String>)->Reply<AuthorizationFlow>{
    Ok(Json(ApiResponse::ok(service.cancel_authorization(&id).await?)))
}
async fn query(State(service):HubState,_:User,Json(request):Json<OfficeQuery>)->Reply<OfficeQueryResult>{
    Ok(Json(ApiResponse::ok(service.query(request).await?)))
}
async fn execute(State(service):HubState,_:User,Json(request):Json<ExecuteRequest>)->Reply<OfficeOperation>{
    Ok(Json(ApiResponse::ok(service.execute(request).await?)))
}
async fn operation(State(service):HubState,_:User,Path(id):Path<String>)->Reply<OfficeOperation>{
    Ok(Json(ApiResponse::ok(service.operation(&id).await?)))
}
async fn operation_by_request(State(service):HubState,_:User,Path(request_id):Path<String>)->Reply<OfficeOperation>{
    Ok(Json(ApiResponse::ok(service.operation_by_request(&request_id).await?)))
}
async fn reconcile(State(service):HubState,_:User,Path(id):Path<String>)->Reply<OfficeOperation>{
    Ok(Json(ApiResponse::ok(service.reconcile(&id).await?)))
}
