//! UI-thread presentation ownership, separate from document load state.
//! Native transitions reapply this mask without a renderer layout replay.

use nomifun_browser_platform::navigation::{BrowserContentState, BrowserNavigationPhase};
use nomifun_browser_platform::runtime::{BrowserLoadSummary, BrowserTabLifecycle, BrowserTabSnapshot, BrowserTabTarget};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{GetParent, IsWindowVisible, SW_HIDE, SW_SHOW, ShowWindow},
};
use windows::core::BOOL;

#[derive(Clone)]
struct PresentationRequest {
    wanted: bool,
    surface_lease: CancellationToken,
    closing_lease: CancellationToken,
    rendering_capture: Arc<AtomicBool>,
    popup_hold: Option<PopupCommitHold>,
}
#[derive(Clone)]
pub(crate) struct PopupCommitHold {
    pub(crate) opener: BrowserTabTarget,
    pub(crate) child: BrowserTabTarget,
    pub(crate) metadata: Arc<Mutex<BrowserTabSnapshot>>,
    pub(crate) closing_lease: CancellationToken,
}
impl PopupCommitHold {
    fn pending(&self, opener: &BrowserTabTarget) -> bool {
        if self.opener != *opener || self.child.runtime_generation != opener.runtime_generation
            || self.closing_lease.is_cancelled() {
            return false;
        }
        let registered = OWNERS.with(|owners| owners.borrow().get(&self.child.tab_id)
            .is_some_and(|owner| Arc::ptr_eq(&owner.metadata, &self.metadata)));
        if !registered { return false; }
        let child = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
        child.target == self.child && first_document_pending(&child)
    }
}

fn first_document_pending(tab: &BrowserTabSnapshot) -> bool {
    tab.target.document_generation == 0 && tab.lifecycle == BrowserTabLifecycle::Loading
        && tab.load.as_ref().map_or(true, |load| {
            load.navigation_sequence <= 1
                && load.content_state == BrowserContentState::None
                && !load.navigation_control.as_ref().is_some_and(|control| control.paused)
                && matches!(load.phase, BrowserNavigationPhase::Idle | BrowserNavigationPhase::Requested | BrowserNavigationPhase::Provisional)
        })
}
impl PresentationRequest {
    fn panel_presented(&self) -> bool {
        self.wanted && !self.surface_lease.is_cancelled() && !self.closing_lease.is_cancelled()
    }
}
struct Owner {
    controller: ICoreWebView2Controller,
    container: HWND,
    parent: HWND,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    request: RefCell<Option<PresentationRequest>>,
}
thread_local! { static OWNERS: RefCell<HashMap<String, Rc<Owner>>> = RefCell::default(); }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeMask {
    Show,
    Hide,
    HideWindowKeepCompositor,
    HideWindowRunController,
}

fn empty_document(load: Option<&BrowserLoadSummary>) -> bool {
    // A fresh request does not replace the pixels held by the controller.
    // Keep a previous native error page hidden until a real non-error commit
    // establishes usable content. Retained documents stay visible throughout.
    load.map_or(true, |load| load.content_state == BrowserContentState::None)
}
fn mask(panel_presented: bool, empty_document: bool, capturing: bool, closing: bool, popup_hold: bool) -> NativeMask {
    if panel_presented && !empty_document {
        NativeMask::Show
    } else if popup_hold && !closing {
        NativeMask::HideWindowRunController
    } else if capturing && !empty_document && !closing {
        NativeMask::HideWindowKeepCompositor
    } else {
        NativeMask::Hide
    }
}

fn apply(label: &str, owner: &Owner) -> Result<(), String> {
    // Copy live ownership first; native visibility changes may deliver other
    // callbacks, so no registry, request or metadata lock crosses a COM call.
    let request = owner.request.borrow().clone();
    let panel = request
        .as_ref()
        .is_some_and(PresentationRequest::panel_presented);
    let capturing = request
        .as_ref()
        .is_some_and(|request| request.rendering_capture.load(Ordering::Acquire));
    let closing = request
        .as_ref()
        .is_some_and(|request| request.closing_lease.is_cancelled());
    let (empty, target) = {
        let metadata = owner.metadata.lock().unwrap_or_else(|e| e.into_inner());
        (empty_document(metadata.load.as_ref()), metadata.target.clone())
    };
    let popup_hold = request.as_ref().is_some_and(|request| {
        !request.wanted && !request.surface_lease.is_cancelled() && !closing
            && request.popup_hold.as_ref().is_some_and(|hold| hold.pending(&target))
    });
    let native_mask = mask(panel, empty, capturing, closing, popup_hold);
    if owner.container.0.is_null()
        || unsafe { GetParent(owner.container) }.ok() != Some(owner.parent)
    {
        return Err("Browser presentation container is no longer owned.".into());
    }
    let visibility = unsafe {
        let show = native_mask == NativeMask::Show;
        // Load progress is not a focus/layout request. Reconcile actual native
        // state, avoiding repeated ShowWindow activation on an already-shown
        // page and respecting any temporary screenshot compositor state.
        if native_mask == NativeMask::HideWindowRunController
            || IsWindowVisible(owner.container).as_bool() != show {
            let _ = ShowWindow(owner.container, if show { SW_SHOW } else { SW_HIDE });
        }
        if native_mask != NativeMask::HideWindowKeepCompositor {
            let controller_visible = show || native_mask == NativeMask::HideWindowRunController;
            let mut actual = BOOL::default();
            if owner.controller.IsVisible(&mut actual).is_ok() && actual.as_bool() == controller_visible {
                Ok(())
            } else {
                owner
                    .controller
                    .SetIsVisible(controller_visible)
                    .map_err(|_| "Browser document presentation could not be applied.".to_owned())
            }
        } else {
            Ok(())
        }
    };
    // DOM permission/upload choices require the document. A first attachment
    // download has no document and still needs its visible panel's save choice.
    super::user_file_chooser::set_visible(label, native_mask == NativeMask::Show);
    super::user_downloads::set_visible(label, panel && !closing);
    let permissions = super::permissions::set_visible(label, native_mask == NativeMask::Show);
    visibility.and(permissions)
}

pub(crate) fn is_registered(label: &str) -> bool {
    OWNERS.with(|owners| owners.borrow().contains_key(label))
}
pub(crate) fn refresh(label: &str) -> Result<(), String> {
    let owner = OWNERS.with(|owners| owners.borrow().get(label).cloned());
    // A usable commit is shown before its opener loses the temporary hold.
    // Snapshot owners first: COM visibility changes may deliver more callbacks.
    let dependents: Vec<_> = OWNERS.with(|owners| owners.borrow().iter()
        .filter(|(_, owner)| owner.request.borrow().as_ref().is_some_and(|request| {
            request.popup_hold.as_ref().is_some_and(|hold| hold.child.tab_id == label)
        }))
        .map(|(label, owner)| (label.clone(), owner.clone())).collect());
    let mut result = owner.map_or(Ok(()), |owner| apply(label, &owner));
    for (label, owner) in dependents {
        let applied = apply(&label, &owner);
        if result.is_ok() { result = applied; }
    }
    result
}
pub(crate) fn request(
    label: &str,
    wanted: bool,
    surface_lease: CancellationToken,
    closing_lease: CancellationToken,
    rendering_capture: Arc<AtomicBool>,
    popup_hold: Option<PopupCommitHold>,
) -> Result<(), String> {
    let owner = OWNERS
        .with(|owners| owners.borrow().get(label).cloned())
        .ok_or("Browser presentation owner is unavailable.")?;
    *owner.request.borrow_mut() = Some(PresentationRequest {
        wanted,
        surface_lease,
        closing_lease,
        rendering_capture,
        popup_hold,
    });
    apply(label, &owner)
}
pub(crate) fn close_view(label: &str) {
    // Called only after native Close acknowledgement. Late load callbacks find
    // no owner and cannot resurrect a controller or reuse a closed HWND.
    drop(OWNERS.with(|owners| owners.borrow_mut().remove(label)));
    let _ = refresh(label);
}

pub(crate) async fn install(
    view: &tauri::Webview,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
) -> Result<(), String> {
    let label = view.label().to_owned();
    let parent = view
        .window()
        .hwnd()
        .map_err(|_| "Browser parent window is unavailable.")?
        .0 as usize;
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> Result<(), String> {
            if is_registered(&label) {
                return Ok(());
            }
            let parent = HWND(parent as *mut std::ffi::c_void);
            let controller = platform.controller().clone();
            let mut container = HWND::default();
            unsafe { controller.ParentWindow(&mut container) }
                .map_err(|_| "Browser presentation container is unavailable.")?;
            if container.0.is_null()
                || container == parent
                || unsafe { GetParent(container) }.ok() != Some(parent)
            {
                return Err("Browser presentation container ownership is invalid.".into());
            }
            let owner = Rc::new(Owner {
                controller,
                container,
                parent,
                metadata,
                request: RefCell::new(None),
            });
            apply(&label, &owner)?;
            OWNERS.with(|owners| owners.borrow_mut().insert(label, owner));
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| "Browser presentation could not be installed.".to_owned())?;
    rx.await
        .map_err(|_| "Browser presentation installation acknowledgement was lost.".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_browser_platform::navigation::BrowserNavigationControl;
    #[test]
    fn terminal_empty_failure_hides_and_same_slot_recovery_restores() {
        let mut load = BrowserLoadSummary::default();
        for phase in [
            BrowserNavigationPhase::Failed,
            BrowserNavigationPhase::Cancelled,
            BrowserNavigationPhase::Crashed,
        ] {
            load.phase = phase;
            assert_eq!(
                mask(true, empty_document(Some(&load)), false, false, false),
                NativeMask::Hide
            );
        }
        load.phase = BrowserNavigationPhase::Requested;
        assert_eq!(
            mask(true, empty_document(Some(&load)), false, false, false),
            NativeMask::Hide
        );
        load.phase = BrowserNavigationPhase::Finished;
        load.content_state = BrowserContentState::CurrentDocument;
        assert_eq!(
            mask(true, empty_document(Some(&load)), false, false, false),
            NativeMask::Show
        );
        load.phase = BrowserNavigationPhase::Failed;
        load.content_state = BrowserContentState::RetainedDocument;
        assert_eq!(
            mask(true, empty_document(Some(&load)), false, false, false),
            NativeMask::Show
        );
    }
    #[test]
    fn missing_document_stays_masked_during_requests_commits_and_capture() {
        for phase in [
            BrowserNavigationPhase::Idle,
            BrowserNavigationPhase::Requested,
            BrowserNavigationPhase::Provisional,
            BrowserNavigationPhase::Committed,
            BrowserNavigationPhase::Finished,
            BrowserNavigationPhase::Failed,
            BrowserNavigationPhase::Cancelled,
            BrowserNavigationPhase::Crashed,
        ] {
            let load = BrowserLoadSummary { phase, ..Default::default() };
            for presented in [false, true] {
                for capturing in [false, true] {
                    assert_eq!(
                        mask(presented, empty_document(Some(&load)), capturing, false, false),
                        NativeMask::Hide,
                    );
                }
            }
        }
        assert_eq!(mask(true, empty_document(None), true, false, false), NativeMask::Hide);
    }
    #[test]
    fn retained_content_is_visible_and_fresh_usable_commit_restores_without_layout() {
        let mut load = BrowserLoadSummary {
            phase: BrowserNavigationPhase::Requested,
            content_state: BrowserContentState::RetainedDocument,
            ..Default::default()
        };
        for phase in [
            BrowserNavigationPhase::Requested,
            BrowserNavigationPhase::Provisional,
            BrowserNavigationPhase::Failed,
            BrowserNavigationPhase::Cancelled,
        ] {
            load.phase = phase;
            assert_eq!(mask(true, empty_document(Some(&load)), false, false, false), NativeMask::Show);
        }
        load.phase = BrowserNavigationPhase::Committed;
        load.content_state = BrowserContentState::None;
        assert_eq!(mask(true, empty_document(Some(&load)), false, false, false), NativeMask::Hide);
        load.content_state = BrowserContentState::CurrentDocument;
        assert_eq!(mask(true, empty_document(Some(&load)), false, false, false), NativeMask::Show);
        // A real HTTP failure document is still usable content. Its status is
        // not permission to expose a native network error document.
        load.response_status = Some(403);
        load.phase = BrowserNavigationPhase::Failed;
        assert_eq!(mask(true, empty_document(Some(&load)), false, false, false), NativeMask::Show);
    }
    #[test]
    fn superseded_or_closed_surface_cannot_be_shown_by_recovery() {
        let request = PresentationRequest {
            wanted: true,
            surface_lease: CancellationToken::new(),
            closing_lease: CancellationToken::new(),
            rendering_capture: Arc::new(AtomicBool::new(false)),
            popup_hold: None,
        };
        assert!(request.panel_presented());
        request.surface_lease.cancel();
        assert_eq!(
            mask(request.panel_presented(), false, false, false, false),
            NativeMask::Hide
        );
        let request = PresentationRequest {
            surface_lease: CancellationToken::new(),
            ..request
        };
        request.closing_lease.cancel();
        assert_eq!(
            mask(request.panel_presented(), false, true, true, false),
            NativeMask::Hide
        );
    }
    #[test]
    fn hidden_capture_preserves_compositor_but_empty_document_does_not() {
        assert_eq!(
            mask(false, false, true, false, false),
            NativeMask::HideWindowKeepCompositor
        );
        assert_eq!(mask(false, true, true, false, false), NativeMask::Hide);
        assert_eq!(mask(false, false, true, true, false), NativeMask::Hide);
        // The page is hidden, but the presented panel still admits a first
        // download's native destination choice.
        assert_eq!(mask(true, true, false, false, false), NativeMask::Hide);
    }
    #[test]
    fn popup_opener_hold_never_exposes_its_window_or_changes_child_mask() {
        for empty in [false, true] {
            for capturing in [false, true] {
                assert_eq!(mask(false, empty, capturing, false, true), NativeMask::HideWindowRunController);
                assert_eq!(mask(false, empty, capturing, true, true), NativeMask::Hide);
                assert_eq!(mask(true, true, capturing, false, false), NativeMask::Hide);
            }
        }
    }
    #[test]
    fn popup_hold_ends_on_commit_terminal_state_pause_or_later_attempt() {
        let mut child = BrowserTabSnapshot {
            target: BrowserTabTarget { tab_id: "child".into(), runtime_generation: 7, document_generation: 0 },
            title: String::new(), url: String::new(), lifecycle: BrowserTabLifecycle::Loading,
            load: Some(BrowserLoadSummary::default()), can_go_back: false, can_go_forward: false,
            zoom_percent: 100, blocked_permissions: vec![], permission_requests: vec![],
            script_dialog: None, diagnostics: Default::default(),
        };
        for phase in [BrowserNavigationPhase::Idle, BrowserNavigationPhase::Requested, BrowserNavigationPhase::Provisional] {
            let load = child.load.as_mut().unwrap();
            load.phase = phase;
            load.navigation_sequence = u64::from(phase != BrowserNavigationPhase::Idle);
            assert!(first_document_pending(&child));
        }
        for phase in [BrowserNavigationPhase::Committed, BrowserNavigationPhase::Finished, BrowserNavigationPhase::Failed,
            BrowserNavigationPhase::Cancelled, BrowserNavigationPhase::Crashed] {
            child.load.as_mut().unwrap().phase = phase;
            assert!(!first_document_pending(&child));
        }
        child.load.as_mut().unwrap().phase = BrowserNavigationPhase::Provisional;
        child.load.as_mut().unwrap().navigation_control = Some(BrowserNavigationControl { paused: true, ..Default::default() });
        assert!(!first_document_pending(&child));
        child.load.as_mut().unwrap().navigation_control = None;
        child.load.as_mut().unwrap().navigation_sequence = 2;
        assert!(!first_document_pending(&child));
        child.load.as_mut().unwrap().navigation_sequence = 1;
        child.target.document_generation = 1;
        assert!(!first_document_pending(&child));
        child.target.document_generation = 0;
        child.load.as_mut().unwrap().content_state = BrowserContentState::CurrentDocument;
        assert!(!first_document_pending(&child));
    }
}
