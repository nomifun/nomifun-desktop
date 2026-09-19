//! macOS uses CEF. Only semantic algorithms and platform contracts are shared
//! with Windows; no COM/WebView2 host or lifecycle implementation is reused.
pub(crate) mod native;
pub(crate) mod host;
pub(crate) mod lifecycle;
pub(crate) mod user_file_chooser;
