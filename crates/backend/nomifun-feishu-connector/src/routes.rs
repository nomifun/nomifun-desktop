use std::sync::Arc;
use axum::{Json, Router, extract::{DefaultBodyLimit, Extension, Path, State},
    routing::{get, post, put}};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use crate::*;

pub fn feishu_connector_routes(service: Arc<FeishuConnectorService>) -> Router {
    Router::new()
        .route("/api/feishu-connector/status", get(status))
        .route("/api/feishu-connector/profiles", get(profiles))
        .route("/api/feishu-connector/runtime/prepare", post(prepare))
        .route("/api/feishu-connector/runtime/repair", post(repair))
        .route("/api/feishu-connector/runtime/select", post(select))
        .route("/api/feishu-connector/runtime/check-update", post(check_update))
        .route("/api/feishu-connector/connection", put(connection))
        .route("/api/feishu-connector/auth", post(start_auth))
        .route("/api/feishu-connector/auth/logout", post(logout))
        .route("/api/feishu-connector/auth/{id}", get(auth).delete(cancel_auth))
        .route("/api/feishu-connector/query", post(query))
        .route("/api/feishu-connector/operations", post(execute))
        .route("/api/feishu-connector/operations/request/{request_id}", get(operation_by_request))
        .route("/api/feishu-connector/operations/{id}", get(operation))
        .route("/api/feishu-connector/operations/{id}/reconcile", post(reconcile))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(service)
}
type ConnectorState = State<Arc<FeishuConnectorService>>;
type User = Extension<CurrentUser>;
type Reply<T> = Result<Json<ApiResponse<T>>, FeishuConnectorError>;
async fn status(State(service): ConnectorState, _:User) -> Reply<FeishuConnectorStatus> {
    Ok(Json(ApiResponse::ok(service.status().await?)))
}
async fn profiles(State(service):ConnectorState,_:User)->Reply<Vec<FeishuProfile>>{
    Ok(Json(ApiResponse::ok(service.profiles().await?)))
}
async fn prepare(State(service):ConnectorState,_:User)->Reply<RuntimeInfo>{
    Ok(Json(ApiResponse::ok(service.prepare_runtime(false).await?)))
}
async fn repair(State(service):ConnectorState,_:User)->Reply<RuntimeInfo>{
    Ok(Json(ApiResponse::ok(service.prepare_runtime(true).await?)))
}
async fn select(State(service):ConnectorState,_:User,Json(request):Json<RuntimeSelection>)->Reply<RuntimeInfo>{
    Ok(Json(ApiResponse::ok(service.select_runtime(request).await?)))
}
async fn check_update(State(service):ConnectorState,_:User)->Reply<RuntimeUpdateInfo>{
    Ok(Json(ApiResponse::ok(service.check_update().await?)))
}
async fn connection(State(service):ConnectorState,_:User,Json(request):Json<ConnectionUpdate>)->Reply<FeishuConnection>{
    Ok(Json(ApiResponse::ok(service.update_connection(request).await?)))
}
async fn start_auth(State(service):ConnectorState,_:User,Json(request):Json<AuthStartRequest>)->Reply<AuthorizationFlow>{
    Ok(Json(ApiResponse::ok(service.start_authorization(request).await?)))
}
async fn logout(State(service):ConnectorState,_:User,Json(request):Json<LogoutRequest>)->Reply<()>{
    service.logout(request).await?;
    Ok(Json(ApiResponse::ok(())))
}
async fn auth(State(service):ConnectorState,_:User,Path(id):Path<String>)->Reply<AuthorizationFlow>{
    Ok(Json(ApiResponse::ok(service.authorization(&id)?)))
}
async fn cancel_auth(State(service):ConnectorState,_:User,Path(id):Path<String>)->Reply<AuthorizationFlow>{
    Ok(Json(ApiResponse::ok(service.cancel_authorization(&id).await?)))
}
async fn query(State(service):ConnectorState,_:User,Json(request):Json<FeishuQuery>)->Reply<FeishuQueryResult>{
    Ok(Json(ApiResponse::ok(service.query(request).await?)))
}
async fn execute(State(service):ConnectorState,_:User,Json(request):Json<ExecuteRequest>)->Reply<FeishuOperation>{
    Ok(Json(ApiResponse::ok(service.execute(request).await?)))
}
async fn operation(State(service):ConnectorState,_:User,Path(id):Path<String>)->Reply<FeishuOperation>{
    Ok(Json(ApiResponse::ok(service.operation(&id).await?)))
}
async fn operation_by_request(State(service):ConnectorState,_:User,Path(request_id):Path<String>)->Reply<FeishuOperation>{
    Ok(Json(ApiResponse::ok(service.operation_by_request(&request_id).await?)))
}
async fn reconcile(State(service):ConnectorState,_:User,Path(id):Path<String>)->Reply<FeishuOperation>{
    Ok(Json(ApiResponse::ok(service.reconcile(&id).await?)))
}
