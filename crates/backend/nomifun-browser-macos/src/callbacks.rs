//! WebKit delegates have exact native ownership; late navigation callbacks are
//! checked against the current WKNavigation before changing product state.
use crate::{
    engine::{Page, PopupCandidate, create_native_page, native_page, navigation_allowed},
    interactions::NativeInteractions,
    navigation::{self, Update},
};
use block2::DynBlock;
use nomifun_browser_platform::runtime::{BrowserCancellationReason, BrowserNavigationSource};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::Bool,
};
use objc2_foundation::{NSArray, NSError, NSHTTPURLResponse, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_web_kit::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Weak, atomic::Ordering},
};

pub(crate) struct DelegateState {
    page: Weak<Page>,
    interactions: Rc<NativeInteractions>,
    navigation: RefCell<Option<Retained<WKNavigation>>>,
    announced: Cell<bool>,
    start_seen: Cell<bool>,
    // Native requests replaced before their didStart acknowledgement retain
    // identity until that start or terminal callback arrives. No page facts
    // or persistent history are stored here.
    awaiting_retired_start: RefCell<Vec<Retained<WKNavigation>>>,
}
define_class!(
    #[unsafe(super = NSObject)]
    #[name = "NomiFunWKNavigationDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateState]
    pub(crate) struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(
            &self,
            _view: &WKWebView,
            action: &WKNavigationAction,
            decision: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let live = self.page().is_some();
            let url = unsafe { action.request().URL() }
                .and_then(|u| u.absoluteString())
                .map(|u| u.to_string())
                .unwrap_or_default();
            let main = unsafe { action.targetFrame() }.is_none_or(|f| unsafe { f.isMainFrame() });
            // Non-network documents are allowed only as subframes. No local
            // file or host IPC scheme is ever admitted into an untrusted page.
            let download = unsafe { action.shouldPerformDownload() };
            let generated_file = download
                && self
                    .page()
                    .is_some_and(|p| p.allows_user_download())
                && (url.starts_with("blob:") || url.starts_with("data:"));
            let allowed = live
                && (navigation_allowed(&url)
                    || url == "about:blank"
                    || (!main && (url.starts_with("data:") || url.starts_with("blob:")))
                    || generated_file);
            if main && (download || !allowed) {
                if let Some(page) = self.page() {
                    page.transition(Update::PolicyCancelled { url: Some(url.clone()), reason: if download { BrowserCancellationReason::Download } else { BrowserCancellationReason::NavigationRejected } });
                }
            }
            decision.call((if !allowed {
                WKNavigationActionPolicy::Cancel
            } else if download {
                WKNavigationActionPolicy::Download
            } else {
                WKNavigationActionPolicy::Allow
            },));
        }
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            _view: &WKWebView,
            response: &WKNavigationResponse,
            decision: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            if unsafe { response.isForMainFrame() } {
                if let Some(page) = self.page() {
                    let native_response = unsafe { response.response() };
                    let url = native_response.URL().and_then(|url| url.absoluteString()).map(|url| url.to_string());
                    let status = native_response.downcast_ref::<NSHTTPURLResponse>().and_then(|response| u16::try_from(response.statusCode()).ok());
                    page.transition(Update::Response { url, status });
                }
            }
            let policy = if self.page().is_none() {
                WKNavigationResponsePolicy::Cancel
            } else if unsafe { response.canShowMIMEType() } {
                WKNavigationResponsePolicy::Allow
            } else {
                if let Some(page) = self.page() {
                    page.transition(Update::PolicyCancelled { url: None, reason: BrowserCancellationReason::Download });
                }
                WKNavigationResponsePolicy::Download
            };
            decision.call((policy,));
        }
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            let Some(page) = self.page() else {
                return;
            };
            if self.take_retired(navigation) { return; }
            if self.ivars().announced.get() && !self.current(navigation) {
                return;
            }
            if !self.ivars().announced.get() && self.current(navigation) { return; }
            self.ivars().start_seen.set(true);
            *self.ivars().navigation.borrow_mut() = navigation.map(|value| unsafe { Retained::retain(value as *const WKNavigation as *mut WKNavigation) }.unwrap());
            if !self.ivars().announced.replace(false) {
                page.transition(Update::Begin { url: Some(view_url(view)), source: BrowserNavigationSource::PageNavigation, bootstrap: false });
            }
            page.transition(Update::Started { url: view_url(view) });
            self.ivars().interactions.drain_dialogs();
            self.refresh(view);
        }
        #[unsafe(method(webView:didReceiveServerRedirectForProvisionalNavigation:))]
        fn redirected(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            if self.active(navigation) {
                if let Some(page) = self.page() { page.transition(Update::Redirect { url: view_url(view) }); }
                self.refresh(view);
            }
        }
        #[unsafe(method(webView:didCommitNavigation:))]
        fn committed(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            if self.active(navigation) {
                if let Some(page) = self.page() { page.transition(Update::Committed { url: view_url(view) }); }
                self.refresh(view);
            }
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            if self.take_retired(navigation) { return; }
            if self.active(navigation) {
                if let Some(page) = self.page() { page.transition(Update::Finished { url: view_url(view) }); }
                self.refresh(view);
            }
        }
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn provisional_failed(
            &self,
            view: &WKWebView,
            navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.failed(view, navigation, error);
        }
        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn navigation_failed(
            &self,
            view: &WKWebView,
            navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.failed(view, navigation, error);
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn crashed(&self, _view: &WKWebView) {
            self.ivars().announced.set(false);
            if let Some(page) = self.page() {
                page.transition(Update::Crashed);
                self.ivars().interactions.drain_dialogs();
                self.ivars().interactions.cancel_user_panels();
                page.interrupt_pending();
            }
        }
        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_download(
            &self,
            _view: &WKWebView,
            _action: &WKNavigationAction,
            download: &WKDownload,
        ) {
            self.ivars().interactions.attach_download(download);
        }
        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_download(
            &self,
            _view: &WKWebView,
            _response: &WKNavigationResponse,
            download: &WKDownload,
        ) {
            self.ivars().interactions.attach_download(download);
        }
    }
    unsafe impl WKUIDelegate for Delegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn popup(
            &self,
            _view: &WKWebView,
            configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            self.create_popup(configuration, action)
        }
        #[unsafe(method(webViewDidClose:))]
        fn closed(&self, _view: &WKWebView) {
            if let Some(page) = self.page() {
                page.close_native();
            }
        }
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(
            &self,
            view: &WKWebView,
            message: &NSString,
            frame: &WKFrameInfo,
            completion: &DynBlock<dyn Fn()>,
        ) {
            self.ivars()
                .interactions
                .alert(view, frame, message, completion);
        }
        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(
            &self,
            view: &WKWebView,
            message: &NSString,
            frame: &WKFrameInfo,
            completion: &DynBlock<dyn Fn(Bool)>,
        ) {
            self.ivars()
                .interactions
                .confirm(view, frame, message, completion);
        }
        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(
            &self,
            view: &WKWebView,
            message: &NSString,
            text: Option<&NSString>,
            frame: &WKFrameInfo,
            completion: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            self.ivars()
                .interactions
                .prompt(view, frame, message, text, completion);
        }
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(
            &self,
            view: &WKWebView,
            parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            completion: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            self.ivars()
                .interactions
                .open_panel(view, parameters, completion);
        }
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media_permission(
            &self,
            _view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            _kind: WKMediaCaptureType,
            decision: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            let allowed = self
                .page()
                .is_some_and(|p| !p.input_locked() && p.is_visible());
            if !allowed {
                if let Some(page) = self.page() {
                    page.changed(|s| {
                        if !s.blocked_permissions.iter().any(|p| p == "media_capture") {
                            s.blocked_permissions.push("media_capture".into());
                        }
                    });
                }
            }
            decision.call((if allowed {
                WKPermissionDecision::Prompt
            } else {
                WKPermissionDecision::Deny
            },));
        }
    }
);
impl Delegate {
    fn create_popup(
        &self,
        configuration: &WKWebViewConfiguration,
        action: &WKNavigationAction,
    ) -> Option<Retained<WKWebView>> {
        let opener = self.page()?;
        let url = unsafe { action.request().URL() }
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default();
        if !navigation_allowed(&url) && url != "about:blank" {
            return None;
        }
        let output = opener.popup_sender.lock().unwrap().clone()?;
        let parent = native_page(opener.id)?.parent.clone();
        let page = opener.engine.allocate_page(opener.context.clone());
        page.inherit_input_lock(&opener);
        let native = create_native_page(&page, parent, Some(configuration)).ok()?;
        let (tx, ready) = tokio::sync::oneshot::channel();
        if output
            .try_send(PopupCandidate {
                page: page.clone(),
                target_url: url,
                ready,
            })
            .is_err()
        {
            page.close_native();
            return None;
        }
        let _ = tx.send(Ok(()));
        Some(native.view.clone())
    }
    pub(crate) fn new(
        mtm: MainThreadMarker,
        page: Weak<Page>,
        interactions: Rc<NativeInteractions>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateState {
            page,
            interactions,
            navigation: RefCell::new(None),
            announced: Cell::new(false),
            start_seen: Cell::new(true),
            awaiting_retired_start: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }
    pub(crate) fn prepare_bootstrap(&self) {
        self.prepare(None, BrowserNavigationSource::Unknown, true);
    }
    pub(crate) fn prepare_navigation(&self, url: Option<String>, source: BrowserNavigationSource) {
        self.prepare(url, source, false);
    }
    fn prepare(&self, url: Option<String>, source: BrowserNavigationSource, bootstrap: bool) {
        let awaiting_start = !self.ivars().start_seen.get();
        self.ivars().announced.set(true);
        // Drop the prior retained navigation before dispatch. Completion from
        // its native token is inert even before replacement didStart arrives.
        if let Some(previous) = self.ivars().navigation.borrow_mut().take() {
            if awaiting_start { self.ivars().awaiting_retired_start.borrow_mut().push(previous); }
        }
        if let Some(page) = self.page() {
            page.transition(Update::Begin { url, source, bootstrap });
        }
        self.ivars().interactions.drain_dialogs();
    }
    pub(crate) fn track_navigation(&self, navigation: Option<Retained<WKNavigation>>, view: &WKWebView) {
        let none = navigation.is_none();
        self.ivars().start_seen.set(none);
        *self.ivars().navigation.borrow_mut() = navigation;
        if none {
            self.ivars().announced.set(false);
            if !unsafe { view.isLoading() } {
                if let Some(page) = self.page() { page.transition(Update::NoNavigation { url: view_url(view) }); }
                self.refresh(view);
            }
        }
    }
    pub(crate) fn stop_loading(&self, view: &WKWebView) {
        if let Some(page) = self.page() {
            if navigation::in_flight(page.snapshot().load.phase) {
                // Publish the terminal result and invalidate the token before
                // stopLoading can synchronously deliver cancellation/finish.
                self.ivars().announced.set(false);
                page.transition(Update::Cancelled(BrowserCancellationReason::UserStop));
                unsafe { view.stopLoading(); }
            }
        }
        self.ivars().interactions.drain_dialogs();
        self.refresh(view);
    }
    fn page(&self) -> Option<Arc<Page>> {
        self.ivars()
            .page
            .upgrade()
            .filter(|p| !p.close_requested.load(Ordering::Acquire))
    }
    fn current(&self, navigation: Option<&WKNavigation>) -> bool {
        navigation_id(self.ivars().navigation.borrow().as_deref()) == navigation_id(navigation)
    }
    fn take_retired(&self, navigation: Option<&WKNavigation>) -> bool {
        let id = navigation_id(navigation);
        let mut retired = self.ivars().awaiting_retired_start.borrow_mut();
        if let Some(index) = retired.iter().position(|value| navigation_id(Some(value)) == id) {
            retired.remove(index);
            true
        } else { false }
    }
    fn active(&self, navigation: Option<&WKNavigation>) -> bool {
        self.current(navigation) && self.page().is_some_and(|page| {
            let state = page.snapshot();
            state.bootstrap_loading || navigation::in_flight(state.load.phase)
        })
    }
    pub(crate) fn refresh(&self, view: &WKWebView) {
        let Some(page) = self.page() else { return; };
        let progress = unsafe { view.estimatedProgress() };
        page.transition(Update::Metadata {
            url: view_url(view),
            title: unsafe { view.title() }.map(|title| title.to_string()).unwrap_or_default(),
            back: unsafe { view.canGoBack() }, forward: unsafe { view.canGoForward() },
            progress: if progress.is_finite() { (progress.clamp(0.0, 1.0) * 100.0).round() as u16 } else { 0 },
        });
    }
    fn failed(&self, view: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
        if self.take_retired(navigation) { return; }
        if !self.active(navigation) { return; }
        if let Some(page) = self.page() {
            page.transition(Update::Failed { domain: error.domain().to_string(), code: error.code() as i64 });
        }
        self.refresh(view);
    }
}
fn navigation_id(value: Option<&WKNavigation>) -> usize {
    value.map_or(0, |value| value as *const WKNavigation as usize)
}
fn view_url(view: &WKWebView) -> String {
    unsafe { view.URL() }.and_then(|url| url.absoluteString()).map(|url| url.to_string()).unwrap_or_else(|| "about:blank".into())
}
