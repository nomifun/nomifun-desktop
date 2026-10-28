//! Native four-key subtitle board. No agent session, webview or model credentials
//! live in the transport thread. The reducer is shared by every desktop platform.
pub mod document;
pub mod desktop_control;
pub mod maintenance;
pub mod ipc;
pub mod legacy_service;
pub mod protocol;
pub mod service;
