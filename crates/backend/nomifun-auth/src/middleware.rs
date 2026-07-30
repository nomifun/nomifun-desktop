use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

use nomifun_common::constants::COOKIE_NAME;
use nomifun_common::{AppError, UserId};
use nomifun_db::IUserRepository;

use crate::JwtService;
use crate::cookie::CookieConfig;
use crate::extract::{extract_bearer_token, extract_cookie_value, extract_token_from_headers};
use crate::jwt::is_past_half_life;

/// Authenticated user injected into request extensions by the auth middleware.
///
/// Route handlers extract this from `request.extensions()` to identify
/// the current user.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    /// User ID from the database.
    pub id: UserId,
    /// Username.
    pub username: String,
}

/// Shared state for the authentication middleware.
#[derive(Clone)]
pub struct AuthState {
    pub jwt_service: Arc<JwtService>,
    pub user_repo: Arc<dyn IUserRepository>,
    /// Cookie attributes for the sliding session renewal `Set-Cookie`.
    pub cookie_config: Arc<CookieConfig>,
}

/// Stable authorization state for installation-scoped control planes.
///
/// This is deliberately an immutable user id, not a username or an `admin`
/// flag. The application resolves it once through `installation_identity`
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

    pub fn permits(&self, user_id: &UserId) -> bool {
        user_id.as_str() == self.authoritative_user_id.as_ref()
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
/// 6. Sliding renewal: when a **cookie** session is past half of its
///    lifetime, re-sign it and attach a fresh `Set-Cookie` — an actively
///    used browser session never hits the hard 30-day expiry mid-use
///    (audit 2026-07-30, finding E). Bearer-token clients manage their own
///    token (`/api/auth/refresh`) and are left untouched.
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
    // which injects the installation owner. Honor that and skip JWT verification.
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
        .find_by_id(payload.user_id.as_str())
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {e}")))?
        .ok_or_else(|| AppError::Forbidden("User not found".into()))?;

    // Renew only sessions that actually ride the cookie: a bearer token also
    // present in the headers wins during extraction, so a renewed cookie
    // would be ignored by that client — skip those.
    let renew_cookie = is_past_half_life(&payload)
        && extract_bearer_token(request.headers()).is_none()
        && extract_cookie_value(request.headers(), COOKIE_NAME).as_deref() == Some(token.as_str());
    let renewed = if renew_cookie {
        match state.jwt_service.sign(user.user_id.as_str(), &user.username) {
            Ok(fresh) => Some(state.cookie_config.build_session_cookie(&fresh)),
            Err(error) => {
                // Renewal is opportunistic: the current token is still valid,
                // so serve the request rather than failing it.
                tracing::warn!(%error, "session sliding renewal failed; keeping current token");
                None
            }
        }
    } else {
        None
    };

    request.extensions_mut().insert(CurrentUser {
        id: user.user_id,
        username: user.username,
    });

    let mut response = next.run(request).await;
    if let Some(cookie) = renewed
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    Ok(response)
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
    use super::{InstanceOwnerState, UserId};
    use std::sync::Arc;

    const TEST_OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    #[test]
    fn owner_identity_is_exact_and_username_independent() {
        let state = InstanceOwnerState::new(Arc::from(TEST_OWNER_ID));
        let owner = UserId::parse(TEST_OWNER_ID).unwrap();
        let other = UserId::new();
        assert!(state.permits(&owner));
        assert!(!state.permits(&other));
        for invalid in ["admin", "SYSTEM_DEFAULT_USER", ""] {
            assert!(UserId::parse(invalid).is_err());
        }
    }
}
