use nomifun_common::AppError;

/// Authentication-layer errors.
///
/// Converts to `AppError` for HTTP response mapping.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Password validation failed: {0}")]
    WeakPassword(String),

    #[error("Username validation failed: {0}")]
    InvalidUsername(String),

    #[error("Token expired")]
    TokenExpired,

    #[error("Token invalid: {0}")]
    TokenInvalid(String),

    #[error("Token blacklisted")]
    TokenBlacklisted,

    #[error("Password hash error: {0}")]
    HashError(String),
}

impl From<AuthError> for AppError {
    fn from(err: AuthError) -> Self {
        match err {
            AuthError::InvalidCredentials => AppError::Unauthorized("Invalid username or password".into()),
            AuthError::WeakPassword(msg) => AppError::BadRequest(msg),
            AuthError::InvalidUsername(msg) => AppError::BadRequest(msg),
            AuthError::TokenExpired => AppError::Unauthorized("Token expired".into()),
            AuthError::TokenInvalid(msg) => AppError::Unauthorized(msg),
            AuthError::TokenBlacklisted => AppError::Unauthorized("Token has been revoked".into()),
            AuthError::HashError(msg) => AppError::Internal(format!("Password hash error: {msg}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn invalid_credentials_maps_to_unauthorized() {
        let app_err: AppError = AuthError::InvalidCredentials.into();
        assert_eq!(app_err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn weak_password_maps_to_bad_request() {
        let app_err: AppError = AuthError::WeakPassword("too short".into()).into();
        assert_eq!(app_err.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn invalid_username_maps_to_bad_request() {
        let app_err: AppError = AuthError::InvalidUsername("bad chars".into()).into();
        assert_eq!(app_err.status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn token_expired_maps_to_unauthorized() {
        let app_err: AppError = AuthError::TokenExpired.into();
        assert_eq!(app_err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn token_invalid_maps_to_unauthorized() {
        let app_err: AppError = AuthError::TokenInvalid("bad".into()).into();
        assert_eq!(app_err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn token_blacklisted_maps_to_unauthorized() {
        let app_err: AppError = AuthError::TokenBlacklisted.into();
        assert_eq!(app_err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn hash_error_maps_to_internal() {
        let app_err: AppError = AuthError::HashError("failed".into()).into();
        assert_eq!(app_err.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
