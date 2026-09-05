//! User-scoped sales workspace persistence and installation-owner account admin.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Json, Path, State};
use axum::routing::{get, post};
use axum::{Extension, Router};
use nomifun_auth::{CurrentUser, hash_password, validate_password, validate_username};
use nomifun_common::AppError;
use nomifun_db::{IUserRepository, SqlitePool, models::User, sqlx};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_WORKSPACE_BODY_BYTES: usize = 6 * 1024 * 1024;
const MAX_TASKS: usize = 10_000;
const MAX_COMPANIES: usize = 50_000;
const MAX_RESULTS: usize = 100_000;

#[derive(Clone)]
pub struct SalesTenantRouterState {
    pool: SqlitePool,
    user_repo: Arc<dyn IUserRepository>,
    owner_user_id: Arc<str>,
}

impl SalesTenantRouterState {
    pub fn new(
        pool: SqlitePool,
        user_repo: Arc<dyn IUserRepository>,
        owner_user_id: Arc<str>,
    ) -> Self {
        Self {
            pool,
            user_repo,
            owner_user_id,
        }
    }
}

/// Routes available to every authenticated sales-workspace user.
pub fn sales_tenant_user_routes(state: SalesTenantRouterState) -> Router {
    Router::new()
        .route(
            "/api/sales/workspace",
            get(get_workspace).put(put_workspace),
        )
        .route("/api/sales/access", get(get_access))
        .layer(DefaultBodyLimit::max(MAX_WORKSPACE_BODY_BYTES))
        .with_state(state)
}

/// Account administration routes. The application composition root wraps
/// these in both authentication and the installation-owner gate.
pub fn sales_tenant_owner_routes(state: SalesTenantRouterState) -> Router {
    Router::new()
        .route("/api/sales/admin/users", get(list_users).post(create_user))
        .route(
            "/api/sales/admin/users/{user_id}/password",
            post(reset_user_password),
        )
        .layer(DefaultBodyLimit::max(32 * 1024))
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct SalesWorkspaceResponse {
    success: bool,
    workspace: Value,
    updated_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct PutSalesWorkspaceRequest {
    expected_user_id: String,
    workspace: Value,
}

fn empty_workspace() -> Value {
    json!({
        "version": 1,
        "companyProfile": {
            "companyName": "",
            "website": "",
            "businessSummary": "",
            "valueProposition": "",
            "targetCustomer": "",
            "senderName": "",
            "senderEmail": ""
        },
        "tasks": [],
        "companies": [],
        "results": []
    })
}

async fn get_workspace(
    State(state): State<SalesTenantRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<SalesWorkspaceResponse>, AppError> {
    let row = sqlx::query_as::<_, (String, i64)>(
        "SELECT workspace_json, updated_at FROM sales_workspaces WHERE user_id = ?",
    )
    .bind(user.id.as_str())
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| AppError::Internal(format!("Database error: {error}")))?;

    let (workspace, updated_at) = match row {
        Some((raw, updated_at)) => {
            let workspace = serde_json::from_str(&raw).map_err(|error| {
                AppError::Internal(format!("Stored sales workspace is invalid: {error}"))
            })?;
            (workspace, Some(updated_at))
        }
        None => (empty_workspace(), None),
    };

    Ok(Json(SalesWorkspaceResponse {
        success: true,
        workspace,
        updated_at,
    }))
}

async fn put_workspace(
    State(state): State<SalesTenantRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<PutSalesWorkspaceRequest>,
) -> Result<Json<SalesWorkspaceResponse>, AppError> {
    if request.expected_user_id != user.id.as_str() {
        return Err(AppError::Conflict(
            "The authenticated sales account changed before this workspace save".into(),
        ));
    }
    validate_workspace_shape(&request.workspace)?;
    validate_linked_conversations(&state.pool, user.id.as_str(), &request.workspace).await?;

    let serialized = serde_json::to_string(&request.workspace)
        .map_err(|error| AppError::BadRequest(format!("Invalid sales workspace: {error}")))?;
    if serialized.len() > MAX_WORKSPACE_BODY_BYTES {
        return Err(AppError::BadRequest("Sales workspace is too large".into()));
    }

    let now = nomifun_common::now_ms();
    sqlx::query(
        "INSERT INTO sales_workspaces (user_id, workspace_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(user_id) DO UPDATE SET \
           workspace_json = excluded.workspace_json, updated_at = excluded.updated_at",
    )
    .bind(user.id.as_str())
    .bind(serialized)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(|error| AppError::Internal(format!("Database error: {error}")))?;

    Ok(Json(SalesWorkspaceResponse {
        success: true,
        workspace: request.workspace,
        updated_at: Some(now),
    }))
}

fn validate_workspace_shape(workspace: &Value) -> Result<(), AppError> {
    let object = workspace
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Sales workspace must be an object".into()))?;
    if object.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(AppError::BadRequest(
            "Unsupported sales workspace version".into(),
        ));
    }
    if !object.get("companyProfile").is_some_and(Value::is_object) {
        return Err(AppError::BadRequest(
            "Sales workspace companyProfile must be an object".into(),
        ));
    }

    for (field, limit) in [
        ("tasks", MAX_TASKS),
        ("companies", MAX_COMPANIES),
        ("results", MAX_RESULTS),
    ] {
        let entries = object
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::BadRequest(format!("Sales workspace {field} must be an array")))?;
        if entries.len() > limit {
            return Err(AppError::BadRequest(format!(
                "Sales workspace {field} exceeds the {limit} item limit"
            )));
        }
    }

    Ok(())
}

fn linked_conversation_ids(workspace: &Value) -> HashSet<&str> {
    workspace
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|task| task.get("conversationId").and_then(Value::as_str))
        .filter(|conversation_id| !conversation_id.trim().is_empty())
        .collect()
}

async fn validate_linked_conversations(
    pool: &SqlitePool,
    user_id: &str,
    workspace: &Value,
) -> Result<(), AppError> {
    for conversation_id in linked_conversation_ids(workspace) {
        let owner: Option<String> =
            sqlx::query_scalar("SELECT user_id FROM conversations WHERE conversation_id = ?")
                .bind(conversation_id)
                .fetch_optional(pool)
                .await
                .map_err(|error| AppError::Internal(format!("Database error: {error}")))?;
        if owner.is_some_and(|owner| owner != user_id) {
            return Err(AppError::Forbidden(
                "A linked Agent conversation belongs to another user".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct SalesAccessResponse {
    success: bool,
    is_instance_owner: bool,
}

async fn get_access(
    State(state): State<SalesTenantRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Json<SalesAccessResponse> {
    Json(SalesAccessResponse {
        success: true,
        is_instance_owner: user.id.as_str() == state.owner_user_id.as_ref(),
    })
}

#[derive(Debug, Serialize)]
struct ManagedUser {
    user_id: String,
    username: String,
    is_owner: bool,
    created_at: i64,
    last_login: Option<i64>,
}

impl ManagedUser {
    fn from_user(user: User, owner_user_id: &str) -> Self {
        Self {
            is_owner: user.user_id.as_str() == owner_user_id,
            user_id: user.user_id.to_string(),
            username: user.username,
            created_at: user.created_at,
            last_login: user.last_login,
        }
    }
}

#[derive(Debug, Serialize)]
struct ManagedUsersResponse {
    success: bool,
    users: Vec<ManagedUser>,
}

async fn list_users(
    State(state): State<SalesTenantRouterState>,
) -> Result<Json<ManagedUsersResponse>, AppError> {
    let mut users = state.user_repo.list_users().await?;
    users.sort_by_key(|user| user.created_at);
    Ok(Json(ManagedUsersResponse {
        success: true,
        users: users
            .into_iter()
            .map(|user| ManagedUser::from_user(user, state.owner_user_id.as_ref()))
            .collect(),
    }))
}

#[derive(Debug, Deserialize)]
struct CreateManagedUserRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct ManagedUserResponse {
    success: bool,
    user: ManagedUser,
}

async fn create_user(
    State(state): State<SalesTenantRouterState>,
    Json(request): Json<CreateManagedUserRequest>,
) -> Result<Json<ManagedUserResponse>, AppError> {
    let username = request.username.trim().to_string();
    validate_username(&username)?;
    validate_password(&request.password)?;
    let password = request.password;
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|error| AppError::Internal(format!("Password hash task failed: {error}")))??;
    let user = state.user_repo.create_user(&username, &password_hash).await?;

    Ok(Json(ManagedUserResponse {
        success: true,
        user: ManagedUser::from_user(user, state.owner_user_id.as_ref()),
    }))
}

#[derive(Debug, Deserialize)]
struct ResetManagedUserPasswordRequest {
    password: String,
}

#[derive(Debug, Serialize)]
struct SuccessResponse {
    success: bool,
}

async fn reset_user_password(
    State(state): State<SalesTenantRouterState>,
    Path(user_id): Path<String>,
    Json(request): Json<ResetManagedUserPasswordRequest>,
) -> Result<Json<SuccessResponse>, AppError> {
    if state.user_repo.find_by_id(&user_id).await?.is_none() {
        return Err(AppError::NotFound("User not found".into()));
    }
    validate_password(&request.password)?;
    let password = request.password;
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|error| AppError::Internal(format!("Password hash task failed: {error}")))??;
    state.user_repo.update_password(&user_id, &password_hash).await?;
    Ok(Json(SuccessResponse { success: true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::UserId;
    use nomifun_db::{SqliteUserRepository, init_database_memory};

    #[test]
    fn workspace_shape_requires_the_versioned_sales_contract() {
        assert!(validate_workspace_shape(&empty_workspace()).is_ok());
        assert!(validate_workspace_shape(&json!({"version": 2})).is_err());
        assert!(validate_workspace_shape(&json!({
            "version": 1,
            "companyProfile": {},
            "tasks": {},
            "companies": [],
            "results": []
        })).is_err());
    }

    #[test]
    fn linked_conversations_are_deduplicated() {
        let workspace = json!({
            "tasks": [
                {"conversationId": "a"},
                {"conversationId": "a"},
                {"conversationId": ""},
                {}
            ]
        });
        assert_eq!(linked_conversation_ids(&workspace), HashSet::from(["a"]));
    }

    fn current_user(user: &User) -> CurrentUser {
        CurrentUser {
            id: user.user_id.clone(),
            username: user.username.clone(),
        }
    }

    #[tokio::test]
    async fn workspaces_are_read_and_written_only_for_the_current_user() {
        let database = init_database_memory().await.unwrap();
        let repository = Arc::new(SqliteUserRepository::new(database.pool().clone()));
        let owner = repository.get_system_user().await.unwrap().unwrap();
        let other = repository.create_user("sales-other", "hash").await.unwrap();
        let state = SalesTenantRouterState::new(
            database.pool().clone(),
            repository,
            Arc::from(owner.user_id.as_str()),
        );

        let mut owner_workspace = empty_workspace();
        owner_workspace["companyProfile"]["companyName"] = json!("Owner Company");
        let _ = put_workspace(
            State(state.clone()),
            Extension(current_user(&owner)),
            Json(PutSalesWorkspaceRequest {
                expected_user_id: owner.user_id.to_string(),
                workspace: owner_workspace.clone(),
            }),
        )
        .await
        .unwrap();

        let mut other_workspace = empty_workspace();
        other_workspace["companyProfile"]["companyName"] = json!("Other Company");
        let _ = put_workspace(
            State(state.clone()),
            Extension(current_user(&other)),
            Json(PutSalesWorkspaceRequest {
                expected_user_id: other.user_id.to_string(),
                workspace: other_workspace.clone(),
            }),
        )
        .await
        .unwrap();

        let Json(owner_response) = get_workspace(
            State(state.clone()),
            Extension(current_user(&owner)),
        )
        .await
        .unwrap();
        let Json(other_response) = get_workspace(
            State(state),
            Extension(current_user(&other)),
        )
        .await
        .unwrap();

        assert_eq!(owner_response.workspace, owner_workspace);
        assert_eq!(other_response.workspace, other_workspace);
        assert_ne!(owner_response.workspace, other_response.workspace);
    }

    #[tokio::test]
    async fn workspace_rejects_another_users_agent_conversation() {
        let database = init_database_memory().await.unwrap();
        let repository = Arc::new(SqliteUserRepository::new(database.pool().clone()));
        let owner = repository.get_system_user().await.unwrap().unwrap();
        let other = repository.create_user("sales-other", "hash").await.unwrap();
        let conversation_id = UserId::new();
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO conversations \
             (conversation_id, user_id, name, type, created_at, updated_at) \
             VALUES (?, ?, 'private sales run', 'nomi', ?, ?)",
        )
        .bind(conversation_id.as_str())
        .bind(owner.user_id.as_str())
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await
        .unwrap();

        let state = SalesTenantRouterState::new(
            database.pool().clone(),
            repository,
            Arc::from(owner.user_id.as_str()),
        );
        let mut workspace = empty_workspace();
        workspace["tasks"] = json!([{
            "id": "task-other",
            "name": "not mine",
            "conversationId": conversation_id.as_str()
        }]);

        let error = put_workspace(
            State(state),
            Extension(current_user(&other)),
            Json(PutSalesWorkspaceRequest {
                expected_user_id: other.user_id.to_string(),
                workspace,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn queued_save_is_rejected_after_the_authenticated_account_changes() {
        let database = init_database_memory().await.unwrap();
        let repository = Arc::new(SqliteUserRepository::new(database.pool().clone()));
        let owner = repository.get_system_user().await.unwrap().unwrap();
        let other = repository.create_user("sales-other", "hash").await.unwrap();
        let state = SalesTenantRouterState::new(
            database.pool().clone(),
            repository,
            Arc::from(owner.user_id.as_str()),
        );

        let error = put_workspace(
            State(state),
            Extension(current_user(&other)),
            Json(PutSalesWorkspaceRequest {
                expected_user_id: owner.user_id.to_string(),
                workspace: empty_workspace(),
            }),
        )
        .await
        .unwrap_err();

        assert!(matches!(error, AppError::Conflict(_)));
    }
}
