//! WebKit delegates have exact native ownership; late navigation callbacks are
//! checked against the current WKNavigation before changing product state.
use crate::{
    engine::{Page, PopupCandidate, create_native_page, native_page, navigation_allowed},
    interactions::NativeInteractions,
    navigation::{self, Update},
};
use block2::DynBlock;
use nomifun_browser_platform::runtime::{
    BrowserCancellationReason, BrowserNavigationSource, BrowserNavigationEvent,
    BrowserNavigationEventKind, BrowserNavigationActionSummary, BrowserNavigationActionType,
    BrowserNavigationHttpMethod, BrowserNavigationPolicy,
};
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
    history_navigation: RefCell<Option<Retained<WKNavigation>>>,
    history_dispatch: Cell<Option<(BrowserNavigationSource, u64)>>,
    announced: Cell<bool>,
    start_seen: Cell<bool>,
    // Replaced requests without a native start acknowledgement must retain
    // their token until that acknowledgement or a terminal callback arrives.
    awaiting_retired_start: RefCell<Vec<Retained<WKNavigation>>>,
    // Keep a bounded set of retired tokens so their late start, finish and
    // failure callbacks remain distinguishable from unexplained routing loss.
    // Retaining the native objects also prevents pointer identity reuse.
    retired_navigations: RefCell<Vec<Retained<WKNavigation>>>,
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
            let page = self.page();
            let live = page.is_some();
            let request = unsafe { action.request() };
            let url = request.URL()
                .and_then(|u| u.absoluteString())
                .map(|u| u.to_string())
                .unwrap_or_default();
            let main = unsafe { action.targetFrame() }.is_none_or(|f| unsafe { f.isMainFrame() });
            let paused = main && page.as_ref().is_some_and(|page|
                page.snapshot().load.navigation_control.as_ref().is_some_and(|control| control.paused));
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
            let policy = if paused { BrowserNavigationPolicy::Paused }
                else if !allowed { BrowserNavigationPolicy::Cancel }
                else if download { BrowserNavigationPolicy::Download }
                else { BrowserNavigationPolicy::Allow };
            // This trace promises main-document navigation coverage. An iframe
            // may be its source, but unrelated subframe loads must not flood it.
            if main && let Some(page) = &page {
                let source_frame = unsafe { action.sourceFrame() };
                let source_origin = source_frame.as_ref().and_then(|frame| unsafe { frame.request() }.URL())
                    .and_then(|url| url.absoluteString())
                    .and_then(|url| url::Url::parse(&url.to_string()).ok())
                    .filter(|url| matches!(url.scheme(), "http" | "https"))
                    .map(|url| url.origin().ascii_serialization());
                let mut event = BrowserNavigationEvent::new(BrowserNavigationEventKind::Action,
                    None, None, BrowserNavigationSource::Unknown, Some(url.clone()));
                event.action = Some(BrowserNavigationActionSummary {
                    navigation_type: navigation_type(unsafe { action.navigationType() }),
                    main_frame: main,
                    source_main_frame: source_frame.as_ref().map(|frame| unsafe { frame.isMainFrame() }),
                    http_method: http_method(request.HTTPMethod().as_ref().map(|method| method.to_string()).as_deref()),
                    policy, source_origin,
                    // WKNavigationType is evidence of an action category, not
                    // proof of physical user input. Do not infer a gesture.
                    user_initiated: None,
                });
                page.navigation_action(event);
                if main && (paused || download || !allowed) {
                    page.transition(Update::PolicyCancelled { url: Some(url.clone()), reason:
                        if paused { BrowserCancellationReason::AutomaticNavigationPaused }
                        else if download { BrowserCancellationReason::Download }
                        else { BrowserCancellationReason::NavigationRejected } });
                }
            }
            decision.call((if paused || !allowed {
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
            if self.retired(navigation) { return; }
            if navigation.is_none() { page.mark_navigation_routing_lost(); return; }
            let history = self.ivars().history_navigation.borrow().as_deref()
                .is_some_and(|pending| navigation_id(Some(pending)) == navigation_id(navigation));
            if history {
                self.retire_current();
                self.ivars().history_navigation.borrow_mut().take();
                let Some((source, dispatch_sequence)) = self.ivars().history_dispatch.get() else {
                    page.mark_navigation_routing_lost();
                    return;
                };
                self.ivars().announced.set(false);
                self.ivars().start_seen.set(true);
                *self.ivars().navigation.borrow_mut() = navigation.map(|value| unsafe { Retained::retain(value as *const WKNavigation as *mut WKNavigation) }.unwrap());
                // Only this exact native start turns a history command into
                // a new document attempt. Same-document history has no public
                // full-load callbacks and keeps its already settled state.
                page.transition(Update::HistoryStarted { url: view_url(view), source, dispatch_sequence });
                page.transition(Update::Started { url: view_url(view) });
                self.ivars().interactions.drain_dialogs();
                self.refresh(view);
                if page.snapshot().load.navigation_control.as_ref().is_some_and(|control| control.paused) {
                    self.stop_loading_with_reason(view, BrowserCancellationReason::AutomaticNavigationPaused);
                }
                return;
            }
            if self.ivars().announced.get() && !self.current(navigation) {
                page.mark_navigation_routing_lost();
                return;
            }
            if !self.ivars().announced.get() && self.current(navigation) { return; }
            if !self.current(navigation) { self.retire_history(); self.retire_current(); }
            self.ivars().start_seen.set(true);
            *self.ivars().navigation.borrow_mut() = navigation.map(|value| unsafe { Retained::retain(value as *const WKNavigation as *mut WKNavigation) }.unwrap());
            if !self.ivars().announced.replace(false) {
                page.transition(Update::Begin { url: Some(view_url(view)), source: BrowserNavigationSource::PageNavigation, bootstrap: false });
            }
            page.transition(Update::Started { url: view_url(view) });
            self.ivars().interactions.drain_dialogs();
            self.refresh(view);
            // The action policy may have allowed this navigation just before
            // a user pause. A queued didStart must not escape the new hold.
            if page.snapshot().load.navigation_control.as_ref().is_some_and(|control| control.paused) {
                self.stop_loading_with_reason(view, BrowserCancellationReason::AutomaticNavigationPaused);
            }
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
            if self.retired(navigation) { return; }
            if self.active(navigation) {
                if let Some(page) = self.page() {
                    page.transition(Update::Finished { url: view_url(view) });
                    page.measure_identity();
                }
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
            self.retire_history();
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
            origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            kind: WKMediaCaptureType,
            decision: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            self.ivars().interactions.permissions.offer(origin, kind, decision);
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
        if opener.snapshot().load.navigation_control.as_ref().is_some_and(|control| control.paused) {
            return None;
        }
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
        let ticket = opener.reserve_popup(&page)?;
        page.inherit_input_lock(&opener);
        let native = match create_native_page(&page, parent, Some(configuration)) {
            Ok(native) => native,
            Err(_) => { page.close_native(); return None; }
        };
        let (tx, ready) = tokio::sync::oneshot::channel();
        if output
            .try_send(PopupCandidate::new(page.clone(), url, &opener, ticket, ready))
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
            history_navigation: RefCell::new(None),
            history_dispatch: Cell::new(None),
            announced: Cell::new(false),
            start_seen: Cell::new(true),
            awaiting_retired_start: RefCell::new(Vec::new()),
            retired_navigations: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }
    pub(crate) fn prepare_bootstrap(&self) {
        self.prepare(None, BrowserNavigationSource::Unknown, true);
    }
    pub(crate) fn prepare_navigation(&self, url: Option<String>, source: BrowserNavigationSource) {
        self.prepare(url, source, false);
    }
    pub(crate) fn prepare_history_navigation(&self, source: BrowserNavigationSource) {
        self.retire_history();
        if let Some(page) = self.page() {
            page.transition(Update::AutomaticNavigationPaused(false));
            page.transition(Update::HistoryRequested { source });
            self.ivars().history_dispatch.set(Some((source, page.snapshot().navigation_dispatch_sequence)));
        }
        self.ivars().interactions.drain_dialogs();
    }
    pub(crate) fn track_history_navigation(&self, navigation: Option<Retained<WKNavigation>>, view: &WKWebView) {
        if navigation.is_none() { self.ivars().history_dispatch.set(None); }
        *self.ivars().history_navigation.borrow_mut() = navigation;
        // Sampling after ACK updates metadata only; neither an ACK nor
        // isLoading=false is sufficient evidence to synthesize Finished.
        self.refresh(view);
    }
    fn prepare(&self, url: Option<String>, source: BrowserNavigationSource, bootstrap: bool) {
        self.retire_history();
        self.ivars().announced.set(true);
        // Drop the prior retained navigation before dispatch. Completion from
        // its native token is inert even before replacement didStart arrives.
        self.retire_current();
        if let Some(page) = self.page() {
            if !bootstrap { page.transition(Update::AutomaticNavigationPaused(false)); }
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
        self.stop_loading_with_reason(view, BrowserCancellationReason::UserStop);
    }
    pub(crate) fn stop_history_dispatch(&self, view: &WKWebView, receipt: crate::engine::NativeNavigationReceipt) -> Result<(), String> {
        if !receipt.matches_history_dispatch(self.ivars().history_dispatch.get().map(|(_, sequence)| sequence)) {
            return Err("BROWSER_STALE_OBSERVATION".into());
        }
        self.stop_loading_with_reason(view, BrowserCancellationReason::UserStop);
        Ok(())
    }
    pub(crate) fn pause_navigation(&self, view: &WKWebView) {
        let tracked = self.page().is_some_and(|page| navigation::in_flight(page.snapshot().load.phase));
        self.stop_loading_with_reason(view, BrowserCancellationReason::AutomaticNavigationPaused);
        // WebKit can have accepted a policy action before its didStart facts
        // reach the Page. Cancel that native work too, while leaving a finished
        // retained document's lifecycle intact.
        if !tracked { unsafe { view.stopLoading(); } }
    }
    fn stop_loading_with_reason(&self, view: &WKWebView, reason: BrowserCancellationReason) {
        let pending_history = self.ivars().history_navigation.borrow().is_some();
        self.retire_history();
        if let Some(page) = self.page() {
            if navigation::in_flight(page.snapshot().load.phase) {
                // Publish the terminal result and invalidate the token before
                // stopLoading can synchronously deliver cancellation/finish.
                self.ivars().announced.set(false);
                page.transition(Update::Cancelled(reason));
                unsafe { view.stopLoading(); }
            } else if pending_history {
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
    fn retired(&self, navigation: Option<&WKNavigation>) -> bool {
        let id = navigation_id(navigation);
        let awaiting = {
            let mut retired = self.ivars().awaiting_retired_start.borrow_mut();
            retired.iter().position(|value| navigation_id(Some(value)) == id).map(|index| retired.remove(index))
        };
        if let Some(previous) = awaiting {
            self.remember_retired(previous);
            return true;
        }
        self.ivars().retired_navigations.borrow().iter().any(|value| navigation_id(Some(value)) == id)
    }
    fn retire_current(&self) {
        if let Some(previous) = self.ivars().navigation.borrow_mut().take() {
            if !self.ivars().start_seen.get() { self.ivars().awaiting_retired_start.borrow_mut().push(previous); }
            else { self.remember_retired(previous); }
        }
    }
    fn retire_history(&self) {
        self.ivars().history_dispatch.set(None);
        if let Some(previous) = self.ivars().history_navigation.borrow_mut().take() {
            self.remember_retired(previous);
        }
    }
    fn remember_retired(&self, previous: Retained<WKNavigation>) {
        let mut retired = self.ivars().retired_navigations.borrow_mut();
        retired.push(previous);
        if retired.len() > nomifun_browser_platform::navigation::MAX_NAVIGATION_ATTEMPTS {
            retired.remove(0);
        }
    }
    fn active(&self, navigation: Option<&WKNavigation>) -> bool {
        if self.retired(navigation) { return false; }
        if navigation.is_none() || !self.current(navigation) {
            if let Some(page) = self.page() { page.mark_navigation_routing_lost(); }
            return false;
        }
        self.page().is_some_and(|page| {
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
        if self.retired(navigation) { return; }
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
fn navigation_type(value: WKNavigationType) -> BrowserNavigationActionType {
    match value {
        WKNavigationType::LinkActivated => BrowserNavigationActionType::LinkActivated,
        WKNavigationType::FormSubmitted => BrowserNavigationActionType::FormSubmitted,
        WKNavigationType::BackForward => BrowserNavigationActionType::BackForward,
        WKNavigationType::Reload => BrowserNavigationActionType::Reload,
        WKNavigationType::FormResubmitted => BrowserNavigationActionType::FormResubmitted,
        WKNavigationType::Other => BrowserNavigationActionType::Other,
        _ => BrowserNavigationActionType::Unknown,
    }
}
fn http_method(value: Option<&str>) -> BrowserNavigationHttpMethod {
    match value.map(str::to_ascii_uppercase).as_deref() {
        Some("GET") => BrowserNavigationHttpMethod::Get,
        Some("POST") => BrowserNavigationHttpMethod::Post,
        Some("HEAD") => BrowserNavigationHttpMethod::Head,
        Some("PUT") => BrowserNavigationHttpMethod::Put,
        Some("PATCH") => BrowserNavigationHttpMethod::Patch,
        Some("DELETE") => BrowserNavigationHttpMethod::Delete,
        Some("OPTIONS") => BrowserNavigationHttpMethod::Options,
        Some("CONNECT") => BrowserNavigationHttpMethod::Connect,
        Some("TRACE") => BrowserNavigationHttpMethod::Trace,
        Some(_) => BrowserNavigationHttpMethod::Other,
        None => BrowserNavigationHttpMethod::Unknown,
    }
}
fn view_url(view: &WKWebView) -> String {
    unsafe { view.URL() }.and_then(|url| url.absoluteString()).map(|url| url.to_string()).unwrap_or_else(|| "about:blank".into())
}
