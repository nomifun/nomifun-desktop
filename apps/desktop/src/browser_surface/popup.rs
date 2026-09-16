//! Native new-window deferrals. COM objects never leave their owning UI thread.
//! The host must admit a request and register its child before completing it.

use std::{cell::RefCell, collections::BTreeMap};
use tauri::Manager;
use tokio::sync::{mpsc, oneshot};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2_13, ICoreWebView2Deferral, ICoreWebView2Environment,
        ICoreWebView2NewWindowRequestedEventArgs,
    },
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    WindowCloseRequestedEventHandler,
};
use windows::Win32::{Foundation::HWND, UI::Input::KeyboardAndMouse::IsWindowEnabled};
use windows::core::{BOOL, IUnknown, Interface, PWSTR};

const LIMIT: usize = 8;
const UNAVAILABLE: &str = "Native popup request is stale or unavailable.";

struct Pending {
    owner: uuid::Uuid,
    opener_label: String,
    args: ICoreWebView2NewWindowRequestedEventArgs,
    deferral: Option<ICoreWebView2Deferral>,
    environment: ICoreWebView2Environment,
    private: bool,
    child_label: Option<String>,
    child: Option<tauri::Webview>,
    child_native_closed: bool,
    creating: bool,
    denied: bool,
}

impl Pending {
    fn settle_denied(&mut self) -> Result<(), String> {
        // Both obligations retain their exact native objects until acknowledged.
        // A failed child Close must not discard the deferral or cleanup owner.
        settle_owned(&mut self.child, |child| {
            close_unclaimed(child, &mut self.child_native_closed)
        })?;
        settle_owned(&mut self.deferral, |deferral| unsafe {
            self.args.SetHandled(true).map_err(|_| UNAVAILABLE.to_owned())?;
            deferral.Complete().map_err(|_| UNAVAILABLE.to_owned())
        })
    }
}

fn settle_owned<T>(owned: &mut Option<T>, settle: impl FnOnce(&T) -> Result<(), String>) -> Result<(), String> {
    if let Some(value) = owned.as_ref() {
        settle(value)?;
        owned.take();
    }
    Ok(())
}

struct Registration {
    label: String,
    core: ICoreWebView2,
    popup_token: i64,
    navigation_token: i64,
    close_token: i64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = unsafe { self.core.remove_NewWindowRequested(self.popup_token) };
        let _ = unsafe { self.core.remove_NavigationStarting(self.navigation_token) };
        let _ = unsafe { self.core.remove_WindowCloseRequested(self.close_token) };
    }
}

thread_local! {
    static REGISTRATIONS: RefCell<BTreeMap<uuid::Uuid, Registration>> = RefCell::default();
    static PENDING: RefCell<BTreeMap<uuid::Uuid, Pending>> = RefCell::default();
}

fn deny(id: uuid::Uuid) -> Result<(), String> {
    if PENDING.with(|entries| {
        let mut entries = entries.borrow_mut();
        entries.get_mut(&id).is_some_and(|pending| {
            pending.denied = true;
            pending.creating
        })
    }) {
        // add_child can dispatch reentrant native callbacks. Keep the request
        // until the in-flight creation has returned its exact controller.
        return Err("Native popup creation has not settled.".into());
    }
    let pending = PENDING.with(|entries| entries.borrow_mut().remove(&id));
    // Complete outside the RefCell borrow: completing a native request may
    // resume browser work which produces more callbacks.
    if let Some(mut pending) = pending {
        if let Err(error) = pending.settle_denied() {
            PENDING.with(|entries| entries.borrow_mut().insert(id, pending));
            return Err(error);
        }
    }
    Ok(())
}

fn deny_owner(owner: uuid::Uuid) {
    let ids: Vec<_> = PENDING.with(|entries| {
        entries
            .borrow()
            .iter()
            .filter_map(|(id, pending)| (pending.owner == owner).then_some(*id))
            .collect()
    });
    for id in ids {
        let _ = deny(id);
    }
}

/// Run/close barriers must observe actual cleanup, not just a queued Drop.
/// Use the retained opener identity even after its subscription is removed.
pub(crate) async fn settle_view(view: &tauri::Webview) -> Result<(), String> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.app_handle().run_on_main_thread(move || {
        let ids: Vec<_> = PENDING.with(|entries| entries.borrow().iter()
            .filter_map(|(id, pending)| (pending.opener_label == label).then_some(*id)).collect());
        let mut result = Ok(());
        for id in ids {
            if let Err(error) = deny(id) { result = Err(error); }
        }
        let _ = tx.send(result);
    }).map_err(|_| UNAVAILABLE.to_owned())?;
    rx.await.map_err(|_| UNAVAILABLE.to_owned())?
}

fn remove_owner(owner: uuid::Uuid) {
    let registration = REGISTRATIONS.with(|entries| entries.borrow_mut().remove(&owner));
    drop(registration);
    deny_owner(owner);
}

pub(super) fn close_view(label: &str) {
    let owners: Vec<_> = REGISTRATIONS.with(|entries| {
        entries
            .borrow()
            .iter()
            .filter_map(|(id, registration)| (registration.label == label).then_some(*id))
            .collect()
    });
    for owner in owners {
        remove_owner(owner);
    }
}

/// The Runtime may close a registered candidate before dropping its rejected
/// request. Record that exact COM acknowledgement for the pending owner too.
pub(super) fn acknowledge_child_close(label: &str) {
    PENDING.with(|entries| {
        for pending in entries.borrow_mut().values_mut() {
            if pending.child_label.as_deref() == Some(label) {
                pending.child_native_closed = true;
            }
        }
    });
}

fn admitted_uri(uri: &str) -> Option<String> {
    if matches!(uri, "" | "about:blank") {
        return Some("about:blank".into());
    }
    let url = url::Url::parse(uri).ok()?;
    (uri.len() <= 8192
        && matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

pub(crate) struct PopupRequest {
    id: uuid::Uuid,
    app: tauri::AppHandle,
    pub(crate) opener_label: String,
    pub(crate) url: String,
    pub(crate) admission: Option<PopupAdmission>,
}

#[derive(Clone)]
pub(crate) struct PopupAdmission {
    pub(crate) download: Option<std::sync::Arc<super::user_downloads::AgentRequest>>,
    pub(crate) opener: nomifun_browser_platform::runtime::BrowserTabTarget,
    pub(crate) cancel: tokio_util::sync::CancellationToken,
    pub(crate) closed: tokio_util::sync::CancellationToken,
}

impl Drop for PopupRequest {
    fn drop(&mut self) {
        let id = self.id;
        let _ = self.app.run_on_main_thread(move || { let _ = deny(id); });
    }
}

impl PopupRequest {
    /// Transfer cleanup responsibility to the Runtime's retained candidate
    /// before it enters the tab registry. Cancellation may still deny the
    /// WindowProxy binding, but may no longer destroy a Runtime-owned child.
    pub(crate) async fn claim_child(&self, child: &tauri::Webview) -> Result<(), String> {
        let id = self.id;
        let label = child.label().to_owned();
        let admission = self.admission.clone();
        let (tx, rx) = oneshot::channel();
        self.app.run_on_main_thread(move || {
            let result = PENDING.with(|entries| {
                let mut entries = entries.borrow_mut();
                let pending = entries.get_mut(&id).ok_or(UNAVAILABLE)?;
                if pending.creating || pending.denied || pending.child_native_closed
                    || pending.child_label.as_deref() != Some(label.as_str())
                    || admission.as_ref().is_some_and(|context| context.cancel.is_cancelled() || context.closed.is_cancelled()) {
                    return Err(UNAVAILABLE);
                }
                pending.child.take().ok_or(UNAVAILABLE)?;
                Ok(())
            }).map_err(str::to_owned);
            let _ = tx.send(result);
        }).map_err(|_| UNAVAILABLE.to_owned())?;
        rx.await.map_err(|_| UNAVAILABLE.to_owned())?
    }

    /// Create an un-navigated hidden child in the exact opener environment and
    /// profile. This does not bind/complete the request or make the view usable.
    /// The caller retains the view and applies the current input policy first.
    pub(crate) async fn create_child(
        &self,
        configure: impl FnOnce(
            tauri::webview::WebviewBuilder<tauri::Wry>,
        ) -> tauri::webview::WebviewBuilder<tauri::Wry>
        + Send
        + 'static,
    ) -> Result<tauri::Webview, String> {
        let id = self.id;
        let app = self.app.clone();
        let opener_label = self.opener_label.clone();
        let admission = self.admission.clone();
        let (tx, rx) = oneshot::channel();
        self.app
            .run_on_main_thread(move || {
                let result = (|| -> Result<tauri::Webview, String> {
                    if admission.as_ref().is_some_and(|context| {
                        context.cancel.is_cancelled() || context.closed.is_cancelled()
                    }) {
                        return Err(UNAVAILABLE.into());
                    }
                    let (environment, private) = PENDING.with(|entries| {
                        let mut entries = entries.borrow_mut();
                        let pending = entries.get_mut(&id).ok_or(UNAVAILABLE)?;
                        if pending.child_label.is_some() || pending.creating || pending.denied {
                            return Err(UNAVAILABLE);
                        }
                        pending.creating = true;
                        Ok((pending.environment.clone(), pending.private))
                    })?;
                    let label = format!("browser-popup-{}", uuid::Uuid::now_v7());
                    let created = app.get_webview(&opener_label).ok_or_else(|| UNAVAILABLE.to_owned()).and_then(|opener| opener
                        .window()
                        .add_child(
                            configure(
                                tauri::webview::WebviewBuilder::new(
                                    &label,
                                    tauri::WebviewUrl::External("about:blank".parse().unwrap()),
                                )
                                .with_environment(environment)
                                .incognito(private)
                                .focused(false)
                                .disable_drag_drop_handler()
                                .on_navigation(|url| admitted_uri(url.as_str()).is_some()),
                            ),
                            tauri::LogicalPosition::new(-32_000.0, -32_000.0),
                            tauri::LogicalSize::new(1024.0, 768.0),
                        )
                        .map_err(|_| "Native popup child could not be created.".to_owned()));
                    PENDING.with(|entries| {
                        let mut entries = entries.borrow_mut();
                        // deny and complete both retain a creating request.
                        let pending = entries.get_mut(&id).expect("in-flight popup creation retains its request");
                        pending.creating = false;
                        if let Ok(view) = &created {
                            pending.child_label = Some(label);
                            pending.child = Some(view.clone());
                        }
                    });
                    created
                })();
                if let Err(Ok(_)) = tx.send(result) {
                    let _ = deny(id);
                }
            })
            .map_err(|_| UNAVAILABLE.to_owned())?;
        rx.await.map_err(|_| UNAVAILABLE.to_owned())?
    }

    /// Bind the real WindowProxy to this exact, not-yet-navigated native child.
    /// The host must not issue DOM/CDP navigation before this point.
    /// Keep the request on error until the host finishes candidate cleanup;
    /// implicit early Drop would race that cleanup against unclaimed teardown.
    pub(crate) async fn complete(&self, child: &tauri::Webview) -> Result<(), String> {
        let id = self.id;
        let label = child.label().to_owned();
        let admission = self.admission.clone();
        let (tx, rx) = oneshot::channel();
        child
            .with_webview(move |platform| {
                if PENDING.with(|entries| entries.borrow().get(&id).is_some_and(|pending| pending.creating)) {
                    let _ = tx.send(Err(UNAVAILABLE.to_owned()));
                    return;
                }
                let mut pending = PENDING.with(|entries| entries.borrow_mut().remove(&id));
                let result = (|| -> Result<(), String> {
                    if admission.as_ref().is_some_and(|context| {
                        context.cancel.is_cancelled() || context.closed.is_cancelled()
                    }) {
                        return Err(UNAVAILABLE.into());
                    }
                    let pending = pending.as_mut().ok_or(UNAVAILABLE)?;
                    if pending.denied { return Err(UNAVAILABLE.into()); }
                    if pending.child_label.as_deref() != Some(label.as_str()) {
                        return Err(UNAVAILABLE.into());
                    }
                    // SAFETY: identity checks and all COM operations run on this UI thread.
                    unsafe {
                        let mut container = HWND::default();
                        platform
                            .controller()
                            .ParentWindow(&mut container)
                            .map_err(|_| UNAVAILABLE)?;
                        if container.0.is_null() || IsWindowEnabled(container).as_bool() {
                            return Err("Native popup input must be locked before binding.".into());
                        }
                        let expected = pending
                            .environment
                            .cast::<IUnknown>()
                            .map_err(|_| UNAVAILABLE)?;
                        let actual = platform
                            .environment()
                            .cast::<IUnknown>()
                            .map_err(|_| UNAVAILABLE)?;
                        if expected != actual {
                            return Err("Native popup environment identity did not match.".into());
                        }
                        let core = platform
                            .controller()
                            .CoreWebView2()
                            .map_err(|_| UNAVAILABLE)?;
                        pending
                            .args
                            .SetNewWindow(&core)
                            .map_err(|_| "Native popup binding failed.")?;
                        pending.args.SetHandled(true).map_err(|_| UNAVAILABLE)?;
                        pending
                            .deferral
                            .as_ref()
                            .ok_or(UNAVAILABLE)?
                            .Complete()
                            .map_err(|_| UNAVAILABLE)?;
                        pending.deferral.take();
                        pending.child.take();
                    }
                    Ok(())
                })();
                if result.is_err() {
                    if let Some(pending) = pending.take() {
                        // Request Drop settles/cleans this on a fresh UI task,
                        // not inside a borrowed Tauri with_webview callback.
                        PENDING.with(|entries| entries.borrow_mut().insert(id, pending));
                    }
                }
                let _ = tx.send(result);
            })
            .map_err(|_| UNAVAILABLE.to_owned())?;
        rx.await.map_err(|_| UNAVAILABLE.to_owned())?
    }
}

fn close_unclaimed(view: &tauri::Webview, native_closed: &mut bool) -> Result<(), String> {
    // A retained request may be dropped after the Runtime already completed
    // candidate teardown on a binding error.
    if view.app_handle().get_webview(view.label()).is_none() {
        return if *native_closed { super::forget_closed_if_unregistered(view); Ok(()) } else { Err(UNAVAILABLE.into()) };
    }
    // Called only on the UI thread, outside with_webview. Its dispatch is
    // synchronous here, so remove the Tauri registration after its borrow ends.
    if !*native_closed {
        let (tx,mut rx)=oneshot::channel();
        let dispatched=view.with_webview(move |platform| {
            let _=tx.send(unsafe {platform.controller().Close()}.is_ok());
        });
        if dispatched.is_err() || !matches!(rx.try_recv(),Ok(true)) {
            return Err("Unclaimed popup native close was not confirmed.".into());
        }
        *native_closed = true;
        super::finalize_closed_view(view.label());
    }
    view.close().map_err(|_| "Unclaimed popup registration did not close.".to_owned())?;
    if !super::forget_closed_if_unregistered(view) {return Err("Unclaimed popup registration remains owned.".into());}
    Ok(())
}

pub(crate) struct PopupSubscription {
    owner: uuid::Uuid,
    app: tauri::AppHandle,
    receiver: mpsc::Receiver<PopupRequest>,
    page_closed: tokio_util::sync::CancellationToken,
}

impl Drop for PopupSubscription {
    fn drop(&mut self) {
        let owner = self.owner;
        let _ = self.app.run_on_main_thread(move || remove_owner(owner));
    }
}

impl PopupSubscription {
    pub(crate) async fn listen(view: &tauri::Webview) -> Result<Self, String> {
        Self::listen_inner(view, None).await
    }

    pub(crate) async fn listen_guarded(
        view: &tauri::Webview,
        admission: std::sync::Arc<dyn Fn() -> Option<PopupAdmission> + Send + Sync>,
    ) -> Result<Self, String> {
        Self::listen_inner(view, Some(admission)).await
    }

    async fn listen_inner(
        view: &tauri::Webview,
        admission: Option<std::sync::Arc<dyn Fn() -> Option<PopupAdmission> + Send + Sync>>,
    ) -> Result<Self, String> {
        let owner = uuid::Uuid::now_v7();
        let app = view.app_handle().clone();
        let callback_app = app.clone();
        let page_closed = tokio_util::sync::CancellationToken::new();
        let callback_closed = page_closed.clone();
        let label = view.label().to_owned();
        let (sender, receiver) = mpsc::channel(LIMIT);
        let (tx, rx) = oneshot::channel();
        view.with_webview(move |platform| {
            let result = (|| -> windows::core::Result<()> {
                if REGISTRATIONS
                    .with(|entries| entries.borrow().values().any(|entry| entry.label == label))
                {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::E_UNEXPECTED,
                    ));
                }
                let core = unsafe { platform.controller().CoreWebView2()? };
                let environment = platform.environment();
                let mut private = BOOL::default();
                unsafe {
                    core.cast::<ICoreWebView2_13>()?
                        .Profile()?
                        .IsInPrivateModeEnabled(&mut private)?;
                }
                let opener_label = label.clone();
                let handler = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    // Never permit WebView2's unmanaged top-level fallback.
                    unsafe {
                        args.SetHandled(true)?;
                    }
                    if !REGISTRATIONS.with(|entries| entries.borrow().contains_key(&owner)) {
                        return Ok(());
                    }
                    let mut gesture = BOOL::default();
                    unsafe {
                        args.IsUserInitiated(&mut gesture)?;
                    }
                    if !gesture.as_bool() {
                        return Ok(());
                    }
                    let context = match &admission {
                        Some(admit) => match admit() {
                            Some(context) => Some(context),
                            None => return Ok(()),
                        },
                        None => None,
                    };
                    if context.as_ref().is_some_and(|context| {
                        context.cancel.is_cancelled() || context.closed.is_cancelled()
                    }) {
                        return Ok(());
                    }
                    let mut raw = PWSTR::null();
                    let status = unsafe { args.Uri(&mut raw) };
                    let uri = super::event_string(raw, 8192);
                    status?;
                    let Some(url) = uri.as_deref().and_then(admitted_uri) else {
                        return Ok(());
                    };
                    if PENDING.with(|entries| {
                        entries
                            .borrow()
                            .values()
                            .filter(|pending| pending.owner == owner)
                            .count()
                            >= LIMIT
                    }) {
                        return Ok(());
                    }
                    let id = uuid::Uuid::now_v7();
                    let deferral = unsafe { args.GetDeferral()? };
                    PENDING.with(|entries| {
                        entries.borrow_mut().insert(
                            id,
                            Pending {
                                owner,
                                opener_label: opener_label.clone(),
                                args,
                                deferral: Some(deferral),
                                environment: environment.clone(),
                                private: private.as_bool(),
                                child_label: None,
                                child: None,
                                child_native_closed: false,
                                creating: false,
                                denied: false,
                            },
                        )
                    });
                    let request = PopupRequest {
                        id,
                        app: callback_app.clone(),
                        opener_label: opener_label.clone(),
                        url,
                        admission: context.clone(),
                    };
                    if sender.try_send(request).is_err() {
                        let _ = deny(id);
                        return Ok(());
                    }
                    let expiry_app = callback_app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(context) = context {
                            tokio::select! {
                                _=context.cancel.cancelled()=>{},
                                _=context.closed.cancelled()=>{},
                                _=tokio::time::sleep(std::time::Duration::from_secs(10))=>{},
                            }
                        } else {
                            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        }
                        let _ = expiry_app.run_on_main_thread(move || { let _ = deny(id); });
                    });
                    Ok(())
                }));
                let mut popup_token = 0;
                unsafe {
                    core.add_NewWindowRequested(&handler, &mut popup_token)?;
                }
                let navigation = NavigationStartingEventHandler::create(Box::new(move |_, _| {
                    deny_owner(owner);
                    Ok(())
                }));
                let mut navigation_token = 0;
                if let Err(error) =
                    unsafe { core.add_NavigationStarting(&navigation, &mut navigation_token) }
                {
                    let _ = unsafe { core.remove_NewWindowRequested(popup_token) };
                    return Err(error);
                }
                let closed = WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                    callback_closed.cancel();
                    deny_owner(owner);
                    Ok(())
                }));
                let mut close_token = 0;
                if let Err(error) =
                    unsafe { core.add_WindowCloseRequested(&closed, &mut close_token) }
                {
                    let _ = unsafe { core.remove_NavigationStarting(navigation_token) };
                    let _ = unsafe { core.remove_NewWindowRequested(popup_token) };
                    return Err(error);
                }
                REGISTRATIONS.with(|entries| {
                    entries.borrow_mut().insert(
                        owner,
                        Registration {
                            label,
                            core,
                            popup_token,
                            navigation_token,
                            close_token,
                        },
                    )
                });
                Ok(())
            })();
            let _ = tx.send(result.map_err(|_| "Native popup subscription failed.".to_owned()));
        })
        .map_err(|_| UNAVAILABLE.to_owned())?;
        let subscription = Self {
            owner,
            app,
            receiver,
            page_closed,
        };
        rx.await.map_err(|_| UNAVAILABLE.to_owned())??;
        Ok(subscription)
    }

    pub(crate) async fn next(&mut self) -> Option<PopupRequest> {
        self.receiver.recv().await
    }

    pub(crate) fn page_closed(&self) -> tokio_util::sync::CancellationToken {
        self.page_closed.clone()
    }

    pub(crate) async fn pending_count(&self) -> Result<usize, String> {
        let owner = self.owner;
        let (tx, rx) = oneshot::channel();
        self.app
            .run_on_main_thread(move || {
                let count = PENDING.with(|entries| {
                    entries
                        .borrow()
                        .values()
                        .filter(|pending| pending.owner == owner)
                        .count()
                });
                let _ = tx.send(count);
            })
            .map_err(|_| UNAVAILABLE.to_owned())?;
        rx.await.map_err(|_| UNAVAILABLE.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_cleanup_retains_exact_authority_until_acknowledged() {
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        struct Proof(std::rc::Rc<std::cell::Cell<u32>>);
        impl Drop for Proof {
            fn drop(&mut self) { self.0.set(self.0.get() + 1); }
        }
        let mut owned = Some(Proof(drops.clone()));
        let identity = owned.as_ref().unwrap() as *const Proof;
        assert!(settle_owned(&mut owned, |_| Err("native close failed".into())).is_err());
        assert_eq!(owned.as_ref().unwrap() as *const Proof, identity);
        assert_eq!(drops.get(), 0);
        settle_owned(&mut owned, |_| Ok(())).unwrap();
        assert!(owned.is_none());
        assert_eq!(drops.get(), 1);
        settle_owned(&mut owned, |_| panic!("acknowledged cleanup must not be repeated")).unwrap();
    }

    #[test]
    fn later_failure_does_not_repeat_an_acknowledged_cleanup_step() {
        let mut child = Some("native child");
        let mut deferral = Some("native deferral");
        settle_owned(&mut child, |_| Ok(())).unwrap();
        assert!(settle_owned(&mut deferral, |_| Err("deferral did not complete".into())).is_err());
        assert!(child.is_none());
        assert_eq!(deferral, Some("native deferral"));
        settle_owned(&mut child, |_| panic!("do not close a destroyed controller twice")).unwrap();
        settle_owned(&mut deferral, |_| Ok(())).unwrap();
    }

    #[test]
    fn native_popup_uri_gate_allows_frontend_and_blank_but_not_external_protocols() {
        for uri in [
            "about:blank",
            "",
            "http://localhost:5173/app",
            "https://example.com/a",
        ] {
            assert!(admitted_uri(uri).is_some(), "{uri}")
        }
        for uri in [
            "javascript:alert(1)",
            "file:///private/file",
            "data:text/html,hello",
            "mailto:a@example.com",
            "https://name:secret@example.com",
            "about:config",
        ] {
            assert!(admitted_uri(uri).is_none(), "{uri}")
        }
        assert!(admitted_uri(&format!("https://example.com/{}", "a".repeat(8192))).is_none());
    }
}
