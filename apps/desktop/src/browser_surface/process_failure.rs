//! Native process failures invalidate page identity; they never replay actions.
use nomifun_browser_platform::{
    revision::BrowserRevision,
    runtime::{BrowserTabLifecycle, BrowserTabSnapshot},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Mutex},
};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, ProcessFailedEventHandler};

struct Registration {
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    core: ICoreWebView2,
    token: i64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = unsafe { self.core.remove_ProcessFailed(self.token) };
    }
}
thread_local! { static REGISTRATIONS: RefCell<HashMap<String,Registration>>=RefCell::default(); }
pub(super) fn document_exited(label:&str)->bool {
    REGISTRATIONS.with(|entries|entries.borrow().get(label).is_some_and(|registration|registration.metadata.lock().unwrap_or_else(|error|error.into_inner()).lifecycle==BrowserTabLifecycle::Crashed))
}
pub(super) fn close_view(label: &str) {
    drop(REGISTRATIONS.with(|entries| entries.borrow_mut().remove(label)));
}

fn invalidates_document(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> bool {
    matches!(
        kind,
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
            | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
    )
}
fn apply(tab: &mut BrowserTabSnapshot, kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> bool {
    if !invalidates_document(kind) {
        return false;
    }
    tab.target.document_generation = tab.target.document_generation.saturating_add(1);
    tab.lifecycle = BrowserTabLifecycle::Crashed;
    tab.blocked_permissions.clear();
    tab.script_dialog = None;
    tab.diagnostics.clear_page();
    true
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
            let event_label = label.clone();
            let retained_metadata=metadata.clone();
            let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                unsafe {
                    args.ProcessFailedKind(&mut kind)?;
                }
                let changed = apply(
                    &mut metadata.lock().unwrap_or_else(|error| error.into_inner()),
                    kind,
                );
                if changed {
                    let _=super::permissions::deny_pending(&event_label);
                    super::fail_pending_commands(&event_label);
                    revision.bump();
                }
                Ok(())
            }));
            let mut token = 0;
            unsafe {
                core.add_ProcessFailed(&handler, &mut token)?;
            }
            REGISTRATIONS.with(|entries| {
                entries
                    .borrow_mut()
                    .insert(label, Registration { core, token, metadata:retained_metadata })
            });
            Ok(())
        })();
        let _ = tx.send(
            result.map_err(|_| "Native browser process monitoring is unavailable.".to_owned()),
        );
    })
    .map_err(|error| error.to_string())?;
    rx.await.map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_browser_platform::runtime::BrowserTabTarget;
    #[test]
    fn document_failure_invalidates_refs_but_retains_the_tab() {
        let mut tab = BrowserTabSnapshot {
            target: BrowserTabTarget {
                tab_id: "same-tab".into(),
                runtime_generation: 3,
                document_generation: 7,
            },
            title: "Page".into(),
            url: "http://localhost/".into(),
            lifecycle: BrowserTabLifecycle::Ready,
            can_go_back: true,
            can_go_forward: false,
            blocked_permissions: vec!["camera".into()],
            permission_requests: vec![],
            script_dialog: None,
            diagnostics: Default::default(),
        };
        tab.script_dialog = Some(nomifun_browser_platform::runtime::BrowserDialog {
            request_id: "exited-dialog".into(), target: tab.target.clone(),
            kind: nomifun_browser_platform::runtime::BrowserDialogKind::Confirm,
            message: "Pending".into(), default_text: String::new(), origin: "http://localhost".into(), text_truncated: false,
        });
        assert!(apply(
            &mut tab,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
        ));
        assert_eq!(tab.target.document_generation, 8);
        assert_eq!(tab.target.tab_id, "same-tab");
        assert_eq!(tab.target.runtime_generation, 3);
        assert_eq!(tab.lifecycle, BrowserTabLifecycle::Crashed);
        assert!(tab.blocked_permissions.is_empty());
        assert!(tab.script_dialog.is_none());
        assert!(tab.can_go_back);
    }
    #[test]
    fn recoverable_auxiliary_failures_do_not_claim_document_destruction() {
        assert!(!invalidates_document(
            COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED
        ));
        assert!(!invalidates_document(
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE
        ));
        // A child-process exit does not prove that root-page pressed input is
        // gone. Its frame routing must settle without retiring the whole page.
        assert!(!invalidates_document(
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED
        ));
    }
}
