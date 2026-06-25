// --- File processing ---

pub const NOMIFUN_TIMESTAMP_SEPARATOR: &str = "_nomifun_";
pub const NOMIFUN_FILES_MARKER: &str = "[[NOMI_FILES]]";

// --- WebSocket ---

pub const HEARTBEAT_INTERVAL_MS: u64 = 30_000;
pub const HEARTBEAT_TIMEOUT_MS: u64 = 60_000;
pub const WS_CLOSE_NORMAL: u16 = 1000;
pub const WS_CLOSE_POLICY_VIOLATION: u16 = 1008;

// --- Authentication ---

pub const SESSION_EXPIRY: &str = "24h";
pub const COOKIE_NAME: &str = "nomifun-session";
pub const COOKIE_MAX_AGE_DAYS: u32 = 30;
pub const CSRF_COOKIE_NAME: &str = "nomifun-csrf-token";
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

// --- Server ---

pub const DEFAULT_HOST: &str = "127.0.0.1";
pub const REMOTE_HOST: &str = "0.0.0.0";
pub const DEFAULT_PORT: u16 = 25808;
/// Request body size limit (10 MB).
pub const BODY_LIMIT: usize = 10 * 1024 * 1024;
/// File upload size limit (30 MB).
pub const UPLOAD_MAX_SIZE: usize = 30 * 1024 * 1024;

// --- Team mode ---

/// Hard-coded backends that always support team mode, regardless of ACP capability detection.
pub const TEAM_CAPABLE_BACKENDS: &[&str] = &["claude", "codex", "gemini", "nomi", "codebuddy"];

/// Determine if an agent supports team mode based on its persisted `agent_capabilities` JSON.
///
/// Returns `true` if:
/// 1. The backend is in the hard whitelist, OR
/// 2. The `agent_capabilities` JSON contains an `mcp_capabilities` / `mcpCapabilities` / `mcp`
///    field (per ACP spec, presence of any MCP transport implies stdio support).
pub fn is_team_capable(backend: &str, agent_capabilities: Option<&serde_json::Value>) -> bool {
    if TEAM_CAPABLE_BACKENDS.contains(&backend) {
        return true;
    }
    has_mcp_capability(agent_capabilities)
}

/// Check whether `agent_capabilities` JSON declares any MCP transport.
/// Per ACP spec: stdio is the baseline; if any transport is declared, the agent supports MCP.
pub fn has_mcp_capability(agent_capabilities: Option<&serde_json::Value>) -> bool {
    let Some(caps) = agent_capabilities else {
        return false;
    };
    caps.get("mcp_capabilities")
        .or_else(|| caps.get("mcpCapabilities"))
        .or_else(|| caps.get("mcp"))
        .is_some()
}

// --- Image processing ---

pub const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &[".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".tiff", ".svg"];
/// Remote image download size limit (5 MB).
pub const REMOTE_IMAGE_MAX_SIZE: usize = 5 * 1024 * 1024;
pub const REMOTE_IMAGE_MAX_REDIRECTS: u32 = 5;
