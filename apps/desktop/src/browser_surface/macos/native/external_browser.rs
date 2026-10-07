//! Human-only handoff to macOS. URLs come from the exact active native tab and
//! the Downloads path comes from the application path resolver.

use nomifun_browser_platform::{
    run_guard::RunAdmissionError,
    runtime::WorkspaceError,
};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

pub(crate) async fn open_url(
    url: String,
    cancel: CancellationToken,
) -> Result<(), WorkspaceError> {
    let parsed = url::Url::parse(&url).map_err(|_| WorkspaceError::InvalidUrl)?;
    if cancel.is_cancelled() {
        return Err(RunAdmissionError::Cancelled.into());
    }
    if url.len() > 8192
        || !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(WorkspaceError::InvalidUrl);
    }
    tokio::task::spawn_blocking(move || open::that_detached(parsed.as_str()))
        .await
        .map_err(|_| WorkspaceError::NativeCommandFailed)?
        .map_err(|_| WorkspaceError::NativeCommandFailed)
}

pub(crate) async fn open_downloads(
    path: PathBuf,
    cancel: CancellationToken,
) -> Result<(), WorkspaceError> {
    if cancel.is_cancelled() {
        return Err(RunAdmissionError::Cancelled.into());
    }
    if !path.is_absolute() {
        return Err(WorkspaceError::NativeCommandFailed);
    }
    tokio::task::spawn_blocking(move || open::that_detached(path))
        .await
        .map_err(|_| WorkspaceError::NativeCommandFailed)?
        .map_err(|_| WorkspaceError::NativeCommandFailed)
}
