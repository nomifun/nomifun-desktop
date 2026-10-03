//! macOS-only native CEF adapter. Windows retains its independent WebView2 host.
#![cfg(target_os = "macos")]

pub mod protocol;
pub mod engine;
pub mod guardian;
pub mod guardian_client;
mod application;
mod text;
mod profile;
mod downloads;
