//! Ordinary application command hosting. This crate is the only owner of the
//! managed copy, settings and process tree; MCP consumers never start programs.
mod environment;
mod files;
mod model;
mod owner;
mod relocation;

pub use environment::{ExecutableReport, EnvironmentReport, detect_environment};
pub use model::{AppDetails, AppError, AppMetadata, AppOperation, AppRecord, AppSettings, AppSettingsPatch, AppState, ImportKind, LocalConnection};
pub use owner::HostingOwner;
pub use relocation::{OfflineHostedAppInventory, inspect_offline_hosted_apps, rewrite_offline_hosted_app_registries};

/// Reuse the shared process decoder, including Windows native code pages and
/// incremental UTF-8 handling, rather than guessing encoding in the renderer.
fn decode_output(bytes: &[u8]) -> String {
    let output = nomi_process_runtime::OutputBuffer::new(bytes.len());
    output.push(nomi_process_runtime::OutputStream::Stdout, bytes);
    output.snapshot_from(nomi_process_runtime::OutputCursor::START).text()
}
