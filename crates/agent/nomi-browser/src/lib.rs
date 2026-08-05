//! nomi-browser —— `BrowserTool` facade，包裹进程内自研 CDP 引擎
//! （`nomi-browser-engine`）对外暴露浏览器自动化工具。P0 暴露三动作：
//! `navigate` / `screenshot` / `capabilities`；observe/aria 在 P1+。

pub mod approval;
pub mod extract;
pub mod managed;
pub mod platform_adapter;
pub mod redline;
pub mod site_memory;
pub mod takeover;
pub mod tool;
pub mod visual_fallback;

pub use approval::{ApprovalAsk, ApprovalDecision, ApprovalKind, BrowserApprovalGate, GateEgressApprover};
pub use extract::{ExtractModel, ExtractSchema};
pub use managed::{ManagedBrowserFacade, managed_result_envelope, public_lane_json};
pub use platform_adapter::{
    EngineConfigResolver, ManagedEngineHostFactory, ManagedEngineLaneDriver,
    ManagedLanePolicyDecorator,
};
pub use nomifun_browser_platform::BrowserLaneClient;
pub use redline::{accname_is_irreversible, classify_action, enforce_redline, ActionContext, ApprovalTier};
pub use tool::{BrowserTool, StandaloneBrowserTask, OUT_OF_BAND_CONFIRMED_KEY};

/// Fields whose authority belongs to the main-process runtime/host registry.
/// A caller may select an owner-scoped `lane_id`/`lane_name`, but it may never
/// construct or override identity, surface routing, target ownership, epochs,
/// cancellation, or resource routing. They are never accepted from — nor
/// forwarded out of — model input across the platform boundary.
///
/// This is the ONE shared list for every managed browser surface (the native
/// [`BrowserTool`], the shared [`ManagedBrowserFacade`], and the gateway
/// registry): rejecting/stripping different sets would make identical requests
/// behave differently across supposedly-equivalent surfaces.
pub const TRUSTED_OWNER_INPUT_FIELDS: &[&str] = &[
    "caller",
    "caller_identity",
    "user_id",
    "conversation_id",
    "runtime_instance_id",
    "agent_id",
    "companion_id",
    "execution_id",
    "step_id",
    "attempt_id",
    "remote_connection_id",
    "owner_lease_id",
    "capability_expires_at_ms",
    "allowed_operations",
    "identity_generation",
    "browser_epoch",
    "target_id",
    "frame_id",
    "ref_generation",
    "cancellation_id",
    "workspace_hint",
    "surface",
    "browser_surface",
    "lane_key",
    "task_resource_key",
    "runtime_cleanup_key",
    "task_family_resource_key",
    "task_resource_family_key",
];
