//! JWT authentication, password hashing, CSRF protection, rate limiting, and auth middleware.
mod companion_token;
mod cookie;
mod csrf;
mod error;
mod extract;
mod jwt;
pub mod middleware;
mod password;
pub mod qr_token;
mod rate_limit;
mod routes;
mod security;
pub mod trust;
mod validation;

// Error type
pub use error::AuthError;

// JWT service
pub use jwt::{JwtService, TokenPayload, generate_random_hex_secret, generate_random_secret_string, resolve_jwt_secret};

// Per-companion API token (Remote front door)
pub use companion_token::{CompanionTokenValidator, token_sha256_hex};

// Password service
pub use password::{
    dummy_password_hash, generate_password, generate_user_credentials, hash_password, verify_password,
    verify_password_timed,
};

// Validation
pub use validation::{validate_password, validate_username};

// Rate limiting
pub use rate_limit::{
    RateLimiter, api_rate_limit_middleware, auth_rate_limit_middleware, authenticated_action_rate_limit_middleware,
};

// Token / IP extraction
pub use extract::{
    extract_client_ip, extract_client_ip_from_headers, extract_cookie_value, extract_token_from_headers,
    extract_token_from_ws_headers,
};

// Cookie configuration
pub use cookie::CookieConfig;

// Security headers
pub use security::security_headers_middleware;

// CSRF protection
pub use csrf::csrf_middleware;

// Auth middleware
pub use middleware::{
    AuthState, CurrentUser, InstanceOwnerState, auth_middleware,
    require_instance_owner_middleware,
};

// Trust resolution (local-trust secret, auth policy)
pub use trust::{
    AuthPolicy, LOCAL_TRUST_HEADER, LocalTrusted, SYSTEM_USER_ID, TrustState, is_locally_trusted,
    require_local_trust_middleware, trust_resolve_middleware,
};

// QR token store
pub use qr_token::QrTokenStore;

// Routes
pub use routes::{AuthRouterState, auth_routes};
