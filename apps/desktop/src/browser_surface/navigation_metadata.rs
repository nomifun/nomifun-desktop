//! Native history metadata. Navigation owns all document and URL state.

use nomifun_browser_platform::{revision::BrowserRevision, runtime::BrowserTabSnapshot};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Mutex},
};
use webview2_com::{
    HistoryChangedEventHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2,
};
use windows::core::BOOL;

struct Registration {
    core: ICoreWebView2,
    history: i64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = unsafe { self.core.remove_HistoryChanged(self.history) };
    }
}
thread_local! {static REGISTRATIONS:RefCell<HashMap<String,Registration>>=RefCell::default();}

pub(super) fn close_view(label: &str) {
    let registration = REGISTRATIONS.with(|entries| entries.borrow_mut().remove(label));
    drop(registration);
}

fn history(
    core: &ICoreWebView2,
    metadata: &Mutex<BrowserTabSnapshot>,
    revision: &BrowserRevision,
) -> windows::core::Result<()> {
    let mut back = BOOL::default();
    let mut forward = BOOL::default();
    unsafe {
        core.CanGoBack(&mut back)?;
        core.CanGoForward(&mut forward)?;
    }
    let changed = {
        let mut data = metadata.lock().unwrap_or_else(|error| error.into_inner());
        let changed =
            data.can_go_back != back.as_bool() || data.can_go_forward != forward.as_bool();
        data.can_go_back = back.as_bool();
        data.can_go_forward = forward.as_bool();
        changed
    };
    if changed {
        revision.bump();
    }
    Ok(())
}

pub(crate) async fn install(
    view: &tauri::Webview,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    revision: Arc<BrowserRevision>,
) -> Result<(), String> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            if REGISTRATIONS.with(|entries| entries.borrow().contains_key(&label)) {
                return Ok(());
            }
            let core = unsafe { platform.controller().CoreWebView2()? };
            history(&core, &metadata, &revision)?;
            let history_metadata = metadata.clone();
            let history_revision = revision.clone();
            let handler = HistoryChangedEventHandler::create(Box::new(move |core, _| {
                if let Some(core) = core {
                    history(&core, &history_metadata, &history_revision)?;
                }
                Ok(())
            }));
            let mut history_token = 0;
            unsafe {
                core.add_HistoryChanged(&handler, &mut history_token)?;
            }
            REGISTRATIONS.with(|entries| {
                entries.borrow_mut().insert(
                    label,
                    Registration {
                        core,
                        history: history_token,
                    },
                )
            });
            Ok(())
        })();
        let _ =
            tx.send(result.map_err(|_| "Native navigation metadata is unavailable.".to_owned()));
    })
    .map_err(|error| error.to_string())?;
    rx.await.map_err(|error| error.to_string())?
}
