//! Public WK/AppKit ports used by the desktop runtime. There is no CDP adapter.
use std::sync::Arc;
use nomifun_browser_macos::engine::Page;
#[derive(Clone)]
pub(crate) struct View { pub page: Arc<Page> }
impl View { pub fn new(page: Arc<Page>) -> Self { Self { page } } }
#[path = "native/external_browser.rs"]
pub(crate) mod external_browser;
pub(crate) async fn hide(view: &View) -> Result<(), String> { view.page.hide().await }
pub(crate) async fn set_native_user_input_enabled(view: &View, enabled: bool) -> Result<(), String> {
    view.page.set_input_locked(!enabled).await
}
pub(crate) mod script_dialogs {
    use super::*;
    use nomifun_browser_platform::runtime::{BrowserTabTarget, WorkspaceError};
    pub async fn drain(view: &View) -> Result<(), WorkspaceError> {
        view.page.set_dialog_draining(true).await.map_err(|_| WorkspaceError::NativeCommandFailed)
    }
    pub async fn resume(view: &View) -> Result<(), WorkspaceError> {
        view.page.set_dialog_draining(false).await.map_err(|_| WorkspaceError::NativeCommandFailed)
    }
    pub async fn respond(view: &View, target: BrowserTabTarget, id: String, accept: bool, text: Option<String>, cancel: tokio_util::sync::CancellationToken) -> Result<(), WorkspaceError> {
        if target.tab_id != format!("browser-{}", view.page.id()) { return Err(WorkspaceError::StaleTarget); }
        view.page.reply_dialog(id, target.document_generation, accept, text.unwrap_or_default(), cancel).await.map_err(|error| match error.as_str() {
            "BROWSER_CANCELLED" => nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled.into(),
            "BROWSER_ACTION_INTERRUPTED" => WorkspaceError::ActionInterrupted,
            _ => WorkspaceError::StaleTarget,
        })
    }
}
