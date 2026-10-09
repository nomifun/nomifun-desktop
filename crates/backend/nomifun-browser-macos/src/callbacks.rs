//! WebKit delegates have exact native ownership; late navigation callbacks are
//! checked against the current WKNavigation before changing product state.
use crate::{
    engine::{Page, PopupCandidate, create_native_page, native_page, navigation_allowed},
    interactions::NativeInteractions,
};
use block2::DynBlock;
use nomifun_browser_platform::runtime::BrowserTabLifecycle;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::Bool,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_web_kit::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Weak, atomic::Ordering},
};

pub(crate) struct DelegateState {
    page: Weak<Page>,
    interactions: Rc<NativeInteractions>,
    navigation: Cell<usize>,
    announced: Cell<bool>,
    requested: RefCell<Option<Retained<WKNavigation>>>,
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
                    .is_some_and(|p| !p.input_locked() && p.is_visible())
                && (url.starts_with("blob:") || url.starts_with("data:"));
            let allowed = live
                && (navigation_allowed(&url)
                    || url == "about:blank"
                    || (!main && (url.starts_with("data:") || url.starts_with("blob:")))
                    || generated_file);
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
            let policy = if self.page().is_none() {
                WKNavigationResponsePolicy::Cancel
            } else if unsafe { response.canShowMIMEType() } {
                WKNavigationResponsePolicy::Allow
            } else {
                WKNavigationResponsePolicy::Download
            };
            decision.call((policy,));
        }
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            let Some(page) = self.page() else {
                return;
            };
            if self.ivars().announced.get() && !self.current(navigation) {
                return;
            }
            self.ivars().navigation.set(navigation_id(navigation));
            if !self.ivars().announced.replace(false) {
                page.changed(|s| {
                    s.document_generation = s.document_generation.saturating_add(1);
                    s.lifecycle = BrowserTabLifecycle::Loading;
                    s.dialog = None;
                });
            }
            self.ivars().interactions.drain_dialogs();
            self.refresh(view, None);
        }
        #[unsafe(method(webView:didReceiveServerRedirectForProvisionalNavigation:))]
        fn redirected(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            if self.current(navigation) {
                self.refresh(view, None);
            }
        }
        #[unsafe(method(webView:didCommitNavigation:))]
        fn committed(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            if self.current(navigation) {
                self.refresh(view, None);
            }
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, view: &WKWebView, navigation: Option<&WKNavigation>) {
            if self.current(navigation) {
                self.refresh(view, Some(BrowserTabLifecycle::Ready));
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
            self.ivars().navigation.set(usize::MAX);
            self.ivars().announced.set(false);
            self.ivars().requested.borrow_mut().take();
            if let Some(page) = self.page() {
                page.changed(|s| {
                    s.document_generation = s.document_generation.saturating_add(1);
                    s.lifecycle = BrowserTabLifecycle::Crashed;
                    s.dialog = None;
                });
                self.ivars().interactions.drain_dialogs();
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
            navigation: Cell::new(0),
            announced: Cell::new(false),
            requested: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }
    pub(crate) fn prepare_navigation(&self) {
        self.ivars().announced.set(true);
        // Reject completion/cancellation from the preceding request even if
        // WebKit delivers it before didStart for the replacement navigation.
        self.ivars().navigation.set(usize::MAX);
        self.ivars().requested.borrow_mut().take();
        if let Some(page) = self.page() {
            page.changed(|s| {
                s.document_generation = s.document_generation.saturating_add(1);
                s.lifecycle = BrowserTabLifecycle::Loading;
                s.dialog = None;
            });
        }
        self.ivars().interactions.drain_dialogs();
    }
    pub(crate) fn track_navigation(
        &self,
        navigation: Option<Retained<WKNavigation>>,
        view: &WKWebView,
    ) {
        self.ivars()
            .navigation
            .set(navigation_id(navigation.as_deref()));
        let none = navigation.is_none();
        *self.ivars().requested.borrow_mut() = navigation;
        if none {
            self.ivars().announced.set(false);
            // A nil native navigation identifies a no-op / same-document
            // transition. Only publish Ready when WebKit also reports idle.
            if !unsafe { view.isLoading() } {
                self.refresh(view, Some(BrowserTabLifecycle::Ready));
            }
        }
    }
    fn page(&self) -> Option<Arc<Page>> {
        self.ivars()
            .page
            .upgrade()
            .filter(|p| !p.close_requested.load(Ordering::Acquire))
    }
    fn current(&self, navigation: Option<&WKNavigation>) -> bool {
        self.ivars().navigation.get() == navigation_id(navigation)
    }
    pub(crate) fn refresh(&self, view: &WKWebView, lifecycle: Option<BrowserTabLifecycle>) {
        let Some(page) = self.page() else {
            return;
        };
        let url = unsafe { view.URL() }
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "about:blank".into());
        let title = unsafe { view.title() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let back = unsafe { view.canGoBack() };
        let forward = unsafe { view.canGoForward() };
        let old = page.snapshot();
        if old.url == url
            && old.title == title
            && old.can_go_back == back
            && old.can_go_forward == forward
            && lifecycle.is_none_or(|l| l == old.lifecycle)
        {
            return;
        }
        page.changed(|s| {
            if lifecycle.is_none() && s.lifecycle == BrowserTabLifecycle::Ready && s.url != url {
                s.document_generation = s.document_generation.saturating_add(1);
            }
            s.url = url;
            s.title = title;
            s.can_go_back = back;
            s.can_go_forward = forward;
            if let Some(l) = lifecycle {
                s.lifecycle = l;
            }
        });
    }
    fn failed(&self, view: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
        if !self.current(navigation) {
            return;
        }
        // NSURLErrorCancelled is the explicit Stop/new-navigation path, not a
        // load failure. This callback still belongs to the current navigation.
        self.refresh(
            view,
            Some(if error.code() == -999 {
                BrowserTabLifecycle::Ready
            } else {
                BrowserTabLifecycle::Failed
            }),
        );
    }
}
fn navigation_id(value: Option<&WKNavigation>) -> usize {
    value.map_or(0, |v| v as *const WKNavigation as usize)
}
