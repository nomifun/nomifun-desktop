//! Wire declarations of private worker protocol v1 at Cua commit 4d959bbf.
//! These are transport types, never Session storage or a transcript.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PRIVATE_WORKER_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCompletion {
    NotStarted,
    Completed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRequest {
    pub protocol_version: u32,
    pub request_id: u64,
    pub generation: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_handle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelResponse {
    pub protocol_version: u32,
    pub request_id: u64,
    pub generation: String,
    pub ok: bool,
    pub completion: ActionCompletion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPermissionMode {
    Standard,
    Bounded,
    Unrestricted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAuthorizationOptions {
    pub allowed_modes: Vec<SessionPermissionMode>,
    pub compatibility_mode: SessionPermissionMode,
    pub compatibility_capability_manifest_path: Option<String>,
    pub compatibility_bounded_manifest_path: Option<String>,
    pub unrestricted_acknowledged: bool,
    pub max_session_ttl_seconds: u64,
    pub max_idle_ttl_seconds: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfiguredDriverOptions {
    pub claude_code_compatibility: bool,
    pub authorization: RuntimeAuthorizationOptions,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInitialization {
    pub configured_driver: ConfiguredDriverOptions,
    pub host_bundle_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedSessionOptions {
    pub public_session: String,
    pub mode: SessionPermissionMode,
    pub ttl_seconds: u64,
    pub idle_ttl_seconds: u64,
    pub capability_manifest_path: Option<String>,
    pub bounded_manifest_path: Option<String>,
}

impl WorkerInitialization {
    pub fn standard(host_bundle_id: String) -> Self {
        Self {
            host_bundle_id,
            configured_driver: ConfiguredDriverOptions {
                claude_code_compatibility: false,
                authorization: RuntimeAuthorizationOptions {
                    allowed_modes: vec![SessionPermissionMode::Standard],
                    compatibility_mode: SessionPermissionMode::Standard,
                    compatibility_capability_manifest_path: None,
                    compatibility_bounded_manifest_path: None,
                    unrestricted_acknowledged: false,
                    max_session_ttl_seconds: 86_400,
                    max_idle_ttl_seconds: 3_600,
                },
            },
        }
    }
}
impl TrustedSessionOptions {
    pub fn standard(public_session: String) -> Self {
        Self {
            public_session,
            mode: SessionPermissionMode::Standard,
            ttl_seconds: 86_400,
            idle_ttl_seconds: 3_600,
            capability_manifest_path: None,
            bounded_manifest_path: None,
        }
    }
}
