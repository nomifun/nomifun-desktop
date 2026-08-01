/// Stable business ID of the built-in Nomi agent.
///
/// Catalog/source identity belongs in `agent_metadata.source_key`; all
/// persisted agent references use this bare UUIDv7 business ID.
pub const NOMI_AGENT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000114";

// --- Authentication ---

pub const COOKIE_NAME: &str = "nomifun-session";
pub const COOKIE_MAX_AGE_DAYS: u32 = 30;
pub const SESSION_MAX_AGE_SECONDS: u64 = COOKIE_MAX_AGE_DAYS as u64 * 24 * 60 * 60;
pub const CSRF_COOKIE_NAME: &str = "nomifun-csrf-token";
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

// --- Server ---

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 25808;
/// Request body size limit (10 MB).
pub const BODY_LIMIT: usize = 10 * 1024 * 1024;
/// File upload size limit (30 MB).
pub const UPLOAD_MAX_SIZE: usize = 30 * 1024 * 1024;
