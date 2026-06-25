//! nomi-browser —— `BrowserTool` facade，包裹进程内自研 CDP 引擎
//! （`nomi-browser-engine`）对外暴露浏览器自动化工具。P0 暴露三动作：
//! `navigate` / `screenshot` / `capabilities`；observe/aria 在 P1+。

pub mod approval;
pub mod extract;
pub mod recording;
pub mod redline;
pub mod replay;
pub mod site_memory;
pub mod takeover;
pub mod tool;
pub mod visual_fallback;

pub use approval::{ApprovalAsk, ApprovalDecision, ApprovalKind, BrowserApprovalGate, GateEgressApprover};
pub use extract::{ExtractModel, ExtractSchema};
pub use recording::{RecordedStep, Recording};
pub use redline::{accname_is_irreversible, classify_action, enforce_redline, ActionContext, ApprovalTier};
pub use tool::{BrowserSecretSource, BrowserTool, OUT_OF_BAND_CONFIRMED_KEY};
