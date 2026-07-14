use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use nomifun_common::AppError;
use nomifun_db::IUserRepository;

use crate::JwtService;
use crate::extract::extract_token_from_headers;

/// Authenticated user injected into request extensions by the auth middleware.
///
/// Route handlers extract this from `request.extensions()` to identify
/// the current user.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    /// User ID from the database.
    pub id: String,
    /// Username.
    pub username: String,
}

/// Shared state for the authentication middleware.
#[derive(Clone)]
pub struct AuthState {
    pub jwt_service: Arc<JwtService>,
    pub user_repo: Arc<dyn IUserRepository>,
}

/// Stable authorization state for installation-scoped control planes.
///
/// This is deliberately an immutable user id, not a username or an `admin`
/// flag. The application resolves it once from the canonical system-user row
/// during boot and shares the same value with every transport boundary.
#[derive(Clone, Debug)]
pub struct InstanceOwnerState {
    pub authoritative_user_id: Arc<str>,
}

impl InstanceOwnerState {
    pub fn new(authoritative_user_id: Arc<str>) -> Self {
        Self {
            authoritative_user_id,
        }
    }

    pub fn permits(&self, user_id: &str) -> bool {
        user_id == self.authoritative_user_id.as_ref()
    }
}

/// Authentication middleware that verifies JWT tokens and injects `CurrentUser`.
///
/// Flow:
/// 1. If the global trust middleware already resolved this request as
///    locally-trusted (NoAuth, or a valid local-trust secret), it has already
///    injected [`CurrentUser`] — pass through unchanged.
/// 2. Otherwise extract bearer token from `Authorization` header or
///    `nomifun-session` cookie
/// 3. Verify JWT signature, expiration, and blacklist
/// 4. Look up user in the database to ensure they still exist
/// 5. Insert [`CurrentUser`] into request extensions
///
/// Returns HTTP 403 for any authentication failure (per API spec).
///
/// Use with `axum::middleware::from_fn_with_state`.
pub async fn auth_middleware(
    State(state): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Locally-trusted requests are resolved upstream by `trust_resolve_middleware`,
    // which injects the system user. Honor that and skip JWT verification.
    if request.extensions().get::<CurrentUser>().is_some() {
        return Ok(next.run(request).await);
    }

    let token = extract_token_from_headers(request.headers())
        .ok_or_else(|| AppError::Forbidden("Authentication required".into()))?;

    let payload = state.jwt_service.verify(&token).map_err(|e| {
        tracing::debug!("Token verification failed: {e}");
        AppError::Forbidden("Invalid or expired token".into())
    })?;

    let user = state
        .user_repo
        .find_by_id(&payload.user_id)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {e}")))?
        .ok_or_else(|| AppError::Forbidden("User not found".into()))?;

    request.extensions_mut().insert(CurrentUser {
        id: user.id,
        username: user.username,
    });

    Ok(next.run(request).await)
}

/// Require the already-authenticated caller to be the installation owner.
///
/// Layer this *inside* [`auth_middleware`] so [`CurrentUser`] is present. A
/// missing identity fails closed; this middleware never falls back to a
/// username, local-mode guess, or a hard-coded caller supplied by the route.
pub async fn require_instance_owner_middleware(
    State(state): State<InstanceOwnerState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let current = request
        .extensions()
        .get::<CurrentUser>()
        .ok_or_else(|| AppError::Forbidden("Authentication required".into()))?;

    if !state.permits(&current.id) {
        return Err(AppError::Forbidden(
            "Installation owner access required".into(),
        ));
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod instance_owner_tests {
    use super::InstanceOwnerState;
    use std::sync::Arc;

    #[test]
    fn owner_identity_is_exact_and_username_independent() {
        let state = InstanceOwnerState::new(Arc::from("system_default_user"));
        assert!(state.permits("system_default_user"));
        assert!(!state.permits("admin"));
        assert!(!state.permits("SYSTEM_DEFAULT_USER"));
        assert!(!state.permits(""));
    }
}
