//! Native four-key subtitle board. No agent session, webview or model credentials
//! live in the device process. The reducer is shared by every desktop platform.
pub mod document;
pub mod desktop_control;
pub mod protocol;
pub mod service;
