//! Thread-safe handles to native objects retained solely on AppKit's main thread.
//! No browser subprocess owner, protocol server, IPC bridge or second event loop.
use crate::{callbacks::Delegate, interactions::NativeInteractions};
use block2::RcBlock;
use nomifun_browser_platform::runtime::{
    BrowserDialogKind, BrowserDownloadSnapshot, BrowserPermissionRequest, BrowserSurfaceBounds,
    BrowserTabLifecycle,
};
use objc2::{
    AnyThread, MainThreadMarker, Message, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSEvent, NSEventMask, NSEventModifierFlags,
    NSEventType, NSImage, NSView,
};
use objc2_foundation::{
    NSArray, NSDate, NSDictionary, NSError, NSNumber, NSPoint, NSProcessInfo, NSRect, NSSize,
    NSString, NSURL, NSURLRequest, NSUUID,
};
use objc2_web_kit::{
    WKContentWorld, WKSnapshotConfiguration, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub type UiWork = Box<dyn FnOnce() + Send>;
pub type ParentView = dyn Fn() -> Result<Retained<NSView>, String> + Send + Sync;
pub type DispatchGuard = dyn Fn() -> bool + Send + Sync;
pub enum NavigationCommand {
    Navigate(String),
    Back,
    Forward,
    Reload,
}
const INTERRUPTED: &str = "BROWSER_ACTION_INTERRUPTED";
const UNCONFIRMED: &str = "BROWSER_EXECUTION_UNCONFIRMED";
const STALE: &str = "BROWSER_STALE_OBSERVATION";

#[derive(Clone, Debug)]
pub struct NativeDialog {
    pub request_id: String,
    pub document_generation: u64,
    pub kind: BrowserDialogKind,
    pub message: String,
    pub default_text: String,
    pub origin: String,
    pub text_truncated: bool,
}
#[derive(Clone, Debug)]
pub struct PageSnapshot {
    pub document_generation: u64,
    pub url: String,
    pub title: String,
    pub lifecycle: BrowserTabLifecycle,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub dialog: Option<NativeDialog>,
    pub blocked_permissions: Vec<String>,
    pub permission_requests: Vec<BrowserPermissionRequest>,
}
impl Default for PageSnapshot {
    fn default() -> Self {
        Self {
            document_generation: 0,
            url: "about:blank".into(),
            title: String::new(),
            lifecycle: BrowserTabLifecycle::Loading,
            can_go_back: false,
            can_go_forward: false,
            dialog: None,
            blocked_permissions: vec![],
            permission_requests: vec![],
        }
    }
}

pub(crate) struct NativePage {
    pub view: Retained<WKWebView>,
    // WebKit's named-world registry is weak. Retain the world for this native
    // page's entire lifetime so observe/action calls share the same opaque DOM
    // reference store; recreating only its name creates a new world identifier.
    content_world: Retained<WKContentWorld>,
    pub parent: Retained<NSView>,
    pub delegate: Retained<Delegate>,
    pub interactions: Rc<NativeInteractions>,
    pub owner: Weak<Page>,
    context: Arc<Context>,
}
struct NativeContext {
    store: Retained<WKWebsiteDataStore>,
    identifier: Option<Uuid>,
}
#[derive(Default)]
struct Registry {
    pages: BTreeMap<Uuid, Rc<NativePage>>,
    contexts: BTreeMap<Uuid, NativeContext>,
    monitors: BTreeMap<Uuid, Retained<AnyObject>>,
    removing: BTreeSet<Uuid>,
}
thread_local! { static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default()); }
pub(crate) fn native_page(id: Uuid) -> Option<Rc<NativePage>> {
    REGISTRY.with(|r| r.borrow().pages.get(&id).cloned())
}

pub struct Engine {
    id: Uuid,
    closing: AtomicBool,
    pages: Mutex<BTreeMap<Uuid, Weak<Page>>>,
    pointer_owners: Mutex<BTreeMap<isize, Weak<Page>>>,
}
impl Engine {
    pub fn initialize() -> Result<Arc<Self>, String> {
        let _mtm = MainThreadMarker::new().ok_or("WK initialization requires the main thread")?;
        if NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            < 14
        {
            return Err("WK persistent session stores require macOS 14 or newer".into());
        }
        let owner = Arc::new(Self {
            id: Uuid::now_v7(),
            closing: AtomicBool::new(false),
            pages: Mutex::new(BTreeMap::new()),
            pointer_owners: Mutex::new(BTreeMap::new()),
        });
        let weak = Arc::downgrade(&owner);
        // App-local public event monitor; never observes other apps or uses a
        // global event tap, accessibility permission, swizzling or WebKit SPI.
        let monitor = RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            let event_ref = unsafe { event.as_ref() };
            if weak
                .upgrade()
                .is_some_and(|engine| engine.blocks_user_event(event_ref))
            {
                std::ptr::null_mut()
            } else {
                event.as_ptr()
            }
        });
        let token = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::Any, &monitor)
        }
        .ok_or("WK input gate installation failed")?;
        REGISTRY.with(|r| r.borrow_mut().monitors.insert(owner.id, token));
        Ok(owner)
    }
    pub fn post(self: &Arc<Self>, work: UiWork) -> Result<(), String> {
        if self.closing.load(Ordering::Acquire) {
            return Err("WK engine is closing".into());
        }
        dispatch2::DispatchQueue::main().exec_async(work);
        Ok(())
    }
    pub async fn wait_ready(&self) -> Result<(), String> {
        if self.closing.load(Ordering::Acquire) {
            Err("WK engine is closing".into())
        } else {
            Ok(())
        }
    }
    pub async fn create_context(
        self: &Arc<Self>,
        identifier: Option<Uuid>,
    ) -> Result<Arc<Context>, String> {
        self.wait_ready().await?;
        if identifier.is_some_and(|id| id.is_nil()) {
            return Err("WK store identifier cannot be nil".into());
        }
        let context = Arc::new(Context {
            id: Uuid::now_v7(),
            identifier,
            closed: AtomicBool::new(false),
        });
        let result = context.clone();
        let engine = self.clone();
        on_main(move || {
            if engine.closing.load(Ordering::Acquire) {
                return Err("WK engine is closing".into());
            }
            if identifier.is_some_and(|id| REGISTRY.with(|r| r.borrow().removing.contains(&id))) {
                return Err("WK store removal is in progress".into());
            }
            let mtm = MainThreadMarker::new().unwrap();
            let store = unsafe {
                match identifier {
                    Some(id) => WKWebsiteDataStore::dataStoreForIdentifier(&native_uuid(id), mtm),
                    None => WKWebsiteDataStore::nonPersistentDataStore(mtm),
                }
            };
            REGISTRY.with(|r| {
                r.borrow_mut()
                    .contexts
                    .insert(result.id, NativeContext { store, identifier })
            });
            Ok(())
        })
        .await?;
        Ok(context)
    }
    pub async fn create_page(
        self: &Arc<Self>,
        parent: Arc<ParentView>,
        context: Arc<Context>,
    ) -> Result<Arc<Page>, String> {
        self.wait_ready().await?;
        let page = self.allocate_page(context);
        let pending = page.clone();
        on_main(move || {
            let parent = parent()?;
            let native = create_native_page(&pending, parent, None)?;
            unsafe {
                native.view.loadHTMLString_baseURL(
                    &NSString::from_str("<!doctype html><meta charset=utf-8><title></title>"),
                    None,
                );
            }
            Ok(())
        })
        .await?;
        Ok(page)
    }
    pub(crate) fn allocate_page(self: &Arc<Self>, context: Arc<Context>) -> Arc<Page> {
        let (metadata, _) = watch::channel(PageSnapshot::default());
        let (closed, _) = watch::channel(false);
        let page = Arc::new(Page {
            id: Uuid::now_v7(),
            engine: self.clone(),
            context,
            metadata,
            closed,
            close_requested: AtomicBool::new(false),
            visible: AtomicBool::new(false),
            input_locked: AtomicBool::new(true),
            blocked_inputs: AtomicUsize::new(0),
            dialog_draining: AtomicBool::new(false),
            change_listener: Mutex::new(None),
            popup_sender: Mutex::new(None),
            pending: Mutex::new(BTreeMap::new()),
            downloads: Mutex::new(vec![]),
        });
        self.pages
            .lock()
            .unwrap()
            .insert(page.id, Arc::downgrade(&page));
        page
    }
    fn blocks_user_event(&self, event: &NSEvent) -> bool {
        let kind = event.r#type();
        let keyboard = matches!(
            kind,
            NSEventType::KeyDown | NSEventType::KeyUp | NSEventType::FlagsChanged
        );
        let pointer = matches!(
            kind,
            NSEventType::LeftMouseDown
                | NSEventType::LeftMouseUp
                | NSEventType::RightMouseDown
                | NSEventType::RightMouseUp
                | NSEventType::OtherMouseDown
                | NSEventType::OtherMouseUp
                | NSEventType::MouseMoved
                | NSEventType::LeftMouseDragged
                | NSEventType::RightMouseDragged
                | NSEventType::OtherMouseDragged
                | NSEventType::ScrollWheel
                | NSEventType::Magnify
                | NSEventType::Rotate
                | NSEventType::Swipe
                | NSEventType::BeginGesture
                | NSEventType::EndGesture
                | NSEventType::Pressure
        );
        if !keyboard && !pointer {
            return false;
        }
        let down = matches!(
            kind,
            NSEventType::LeftMouseDown | NSEventType::RightMouseDown | NSEventType::OtherMouseDown
        );
        let up = matches!(
            kind,
            NSEventType::LeftMouseUp | NSEventType::RightMouseUp | NSEventType::OtherMouseUp
        );
        let drag = matches!(
            kind,
            NSEventType::LeftMouseDragged
                | NSEventType::RightMouseDragged
                | NSEventType::OtherMouseDragged
        );
        if up || drag {
            let owner = if up {
                self.pointer_owners
                    .lock()
                    .unwrap()
                    .remove(&event.buttonNumber())
            } else {
                self.pointer_owners
                    .lock()
                    .unwrap()
                    .get(&event.buttonNumber())
                    .cloned()
            };
            if let Some(page) = owner.and_then(|p| p.upgrade()).filter(|p| p.input_locked()) {
                page.blocked_inputs.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        // Modifier transitions are not key-character events. Asking AppKit
        // for characters on FlagsChanged raises an Objective-C exception,
        // which aborts when it crosses Tao's non-unwinding event callback.
        if has_key_characters(kind)
            && event
                .modifierFlags()
                .contains(NSEventModifierFlags::Command)
            && event
                .charactersIgnoringModifiers()
                .is_some_and(|s| s.to_string().eq_ignore_ascii_case("q"))
        {
            return false;
        }
        let pages: Vec<_> = self
            .pages
            .lock()
            .unwrap()
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        // Semantic element.focus() can restore an editable native responder.
        // Re-establish the gate before every local input event, including a
        // menu click with no WK window; menu actions resolve the responder later.
        for page in pages.iter().filter(|page| page.input_locked()) {
            if let Some(native) = native_page(page.id) {
                if release_browser_responder(&native).is_err() {
                    return true;
                }
            }
        }
        let Some(window) = event.window(MainThreadMarker::new().unwrap()) else {
            return false;
        };
        for page in pages {
            if !page.is_visible() {
                continue;
            }
            let Some(native) = native_page(page.id) else {
                continue;
            };
            if native.view.window().as_ref().is_none_or(|w| **w != *window) {
                continue;
            }
            let hit = if keyboard {
                window.firstResponder().is_some_and(|r| {
                    let is_view: bool =
                        unsafe { msg_send![&r, isKindOfClass: objc2::class!(NSView)] };
                    is_view && unsafe { msg_send![&r, isDescendantOf: &*native.view] }
                })
            } else {
                let p = native
                    .view
                    .convertPoint_fromView(event.locationInWindow(), None);
                let b = native.view.bounds();
                p.x >= 0.0 && p.y >= 0.0 && p.x < b.size.width && p.y < b.size.height
            };
            if hit {
                if down {
                    self.pointer_owners
                        .lock()
                        .unwrap()
                        .insert(event.buttonNumber(), Arc::downgrade(&page));
                }
                if page.input_locked() {
                    page.blocked_inputs.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
                return false;
            }
        }
        false
    }
    pub async fn shutdown(self: &Arc<Self>) -> Result<(), String> {
        self.closing.store(true, Ordering::Release);
        let pages: Vec<_> = self
            .pages
            .lock()
            .unwrap()
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for page in pages {
            page.force_close().await?;
        }
        let id = self.id;
        on_main(move || {
            if let Some(token) = REGISTRY.with(|r| r.borrow_mut().monitors.remove(&id)) {
                unsafe {
                    NSEvent::removeMonitor(&token);
                }
            }
            Ok(())
        })
        .await
    }
    pub async fn remove_data_store(identifier: Uuid) -> Result<(), String> {
        if identifier.is_nil() {
            return Err("WK store identifier cannot be nil".into());
        }
        let (tx, rx) = oneshot::channel();
        dispatch2::DispatchQueue::main().exec_async(move || {
            if NSProcessInfo::processInfo()
                .operatingSystemVersion()
                .majorVersion
                < 14
            {
                let _ = tx.send(Err(
                    "WK persistent session stores require macOS 14 or newer".into(),
                ));
                return;
            }
            // This entry point also runs before any browser/renderer exists
            // (for example startup dataset cleanup). On supported macOS,
            // enumerating first can dispatch through WebKit's uninitialized
            // main RunLoop. The public configuration initializer establishes
            // WebKit's process state without opening a view, starting our own
            // event loop, or creating a persistent/default website data store.
            let _configuration =
                unsafe { WKWebViewConfiguration::new(MainThreadMarker::new().unwrap()) };
            if REGISTRY.with(|r| {
                let r = r.borrow();
                r.contexts
                    .values()
                    .any(|c| c.identifier == Some(identifier))
                    || r.removing.contains(&identifier)
            }) {
                let _ = tx.send(Err("WK store is still in use".into()));
                return;
            }
            REGISTRY.with(|r| r.borrow_mut().removing.insert(identifier));
            let tx = Arc::new(Mutex::new(Some(tx)));
            let fetched = RcBlock::new(move |identifiers: std::ptr::NonNull<NSArray<NSUUID>>| {
                let target = native_uuid(identifier);
                let exists = unsafe { identifiers.as_ref() }
                    .iter()
                    .any(|id| *id == *target);
                if !exists {
                    finish_store_removal(identifier, &tx, Ok(()));
                    return;
                }
                remove_store_attempt(identifier, tx.clone(), 1);
            });
            unsafe {
                WKWebsiteDataStore::fetchAllDataStoreIdentifiers(
                    &fetched,
                    MainThreadMarker::new().unwrap(),
                );
            }
        });
        rx.await
            .map_err(|_| "WK persistent store removal acknowledgement lost".to_owned())?
    }
}

pub struct Context {
    id: Uuid,
    pub identifier: Option<Uuid>,
    closed: AtomicBool,
}
impl Context {
    pub async fn clear_site_data(self: &Arc<Self>) -> Result<(), String> {
        let id = self.id;
        let (tx, rx) = oneshot::channel();
        dispatch2::DispatchQueue::main().exec_async(move || {
            let store = REGISTRY.with(|r| r.borrow().contexts.get(&id).map(|c| c.store.clone()));
            let Some(store) = store else {
                let _ = tx.send(Err("WK context is closed".into()));
                return;
            };
            if REGISTRY.with(|r| r.borrow().pages.values().any(|p| p.context.id == id)) {
                let _ = tx.send(Err("WK pages must close before site data removal".into()));
                return;
            }
            let tx = RefCell::new(Some(tx));
            let done = RcBlock::new(move || {
                if let Some(tx) = tx.borrow_mut().take() {
                    let _ = tx.send(Ok(()));
                }
            });
            unsafe {
                store.removeDataOfTypes_modifiedSince_completionHandler(
                    &WKWebsiteDataStore::allWebsiteDataTypes(MainThreadMarker::new().unwrap()),
                    &NSDate::distantPast(),
                    &done,
                );
            }
        });
        rx.await
            .map_err(|_| "WK data removal acknowledgement lost".to_owned())?
    }
    pub async fn close(&self) -> Result<(), String> {
        let id = self.id;
        on_main(move || {
            if REGISTRY.with(|r| r.borrow().pages.values().any(|p| p.context.id == id)) {
                return Err("WK context still owns pages".into());
            }
            REGISTRY.with(|r| r.borrow_mut().contexts.remove(&id));
            Ok(())
        })
        .await?;
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        let id = self.id;
        dispatch2::DispatchQueue::main().exec_async(move || {
            REGISTRY.with(|r| r.borrow_mut().contexts.remove(&id));
        });
    }
}

pub struct PopupCandidate {
    pub page: Arc<Page>,
    pub target_url: String,
    pub ready: oneshot::Receiver<Result<(), String>>,
}
pub struct Page {
    pub(crate) id: Uuid,
    pub(crate) engine: Arc<Engine>,
    pub(crate) context: Arc<Context>,
    metadata: watch::Sender<PageSnapshot>,
    closed: watch::Sender<bool>,
    pub(crate) close_requested: AtomicBool,
    visible: AtomicBool,
    input_locked: AtomicBool,
    blocked_inputs: AtomicUsize,
    dialog_draining: AtomicBool,
    change_listener: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    pub(crate) popup_sender: Mutex<Option<mpsc::Sender<PopupCandidate>>>,
    pending: Mutex<BTreeMap<Uuid, Box<dyn FnOnce(&'static str) + Send>>>,
    downloads: Mutex<Vec<BrowserDownloadSnapshot>>,
}
impl Page {
    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn snapshot(&self) -> PageSnapshot {
        self.metadata.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<PageSnapshot> {
        self.metadata.subscribe()
    }
    pub fn closed(&self) -> watch::Receiver<bool> {
        self.closed.subscribe()
    }
    pub fn input_locked(&self) -> bool {
        self.input_locked.load(Ordering::Acquire)
    }
    pub(crate) fn dialog_draining(&self) -> bool {
        self.dialog_draining.load(Ordering::Acquire)
    }
    pub(crate) fn inherit_input_lock(&self, other: &Page) {
        self.input_locked
            .store(other.input_locked(), Ordering::Release);
        self.dialog_draining
            .store(other.dialog_draining(), Ordering::Release);
    }
    pub(crate) fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Acquire)
    }
    pub fn blocked_input_count(&self) -> usize {
        self.blocked_inputs.load(Ordering::Acquire)
    }
    pub fn set_change_listener(&self, listener: Arc<dyn Fn() + Send + Sync>) {
        *self.change_listener.lock().unwrap() = Some(listener);
    }
    pub(crate) fn changed(&self, update: impl FnOnce(&mut PageSnapshot)) {
        self.metadata.send_modify(update);
        let listener = self.change_listener.lock().unwrap().clone();
        if let Some(listener) = listener {
            listener();
        }
    }
    pub(crate) fn update_downloads(&self, value: Vec<BrowserDownloadSnapshot>) {
        *self.downloads.lock().unwrap() = value;
        self.changed(|_| {});
    }
    pub async fn wait_bootstrap_ready(
        &self,
        cancel: &CancellationToken,
        closing: &CancellationToken,
    ) -> Result<(), String> {
        let mut state = self.subscribe();
        let mut closed = self.closed();
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let snapshot = state.borrow_and_update().clone();
                if *closed.borrow_and_update() || cancel.is_cancelled() || closing.is_cancelled() { return Err(INTERRUPTED.into()); }
                match snapshot.lifecycle { BrowserTabLifecycle::Ready => return Ok(()), BrowserTabLifecycle::Failed | BrowserTabLifecycle::Crashed => return Err("WK bootstrap failed".into()), _ => {} }
                tokio::select! { _ = cancel.cancelled() => return Err(INTERRUPTED.into()), _ = closing.cancelled() => return Err(INTERRUPTED.into()), r = state.changed() => { r.map_err(|_| INTERRUPTED)?; }, r = closed.changed() => {r.map_err(|_| INTERRUPTED)?;} }
            }
        }).await.map_err(|_| "WK bootstrap timed out".to_owned())?
    }
    pub fn listen_popups(&self) -> Result<mpsc::Receiver<PopupCandidate>, String> {
        let mut slot = self.popup_sender.lock().unwrap();
        if slot.is_some() {
            return Err("WK popup listener already registered".into());
        }
        let (tx, rx) = mpsc::channel(8);
        *slot = Some(tx);
        Ok(rx)
    }
    async fn mutate(
        self: &Arc<Self>,
        work: impl FnOnce(&NativePage) -> Result<(), String> + Send + 'static,
    ) -> Result<(), String> {
        let page = self.clone();
        on_main(move || {
            if page.close_requested.load(Ordering::Acquire) {
                return Err(INTERRUPTED.into());
            }
            let native = native_page(page.id).ok_or(INTERRUPTED)?;
            work(&native)
        })
        .await
    }
    pub async fn navigate(self: &Arc<Self>, url: String) -> Result<(), String> {
        self.navigate_guarded(NavigationCommand::Navigate(url), Arc::new(|| true))
            .await
    }
    pub async fn go_back(self: &Arc<Self>) -> Result<(), String> {
        self.navigate_guarded(NavigationCommand::Back, Arc::new(|| true))
            .await
    }
    pub async fn go_forward(self: &Arc<Self>) -> Result<(), String> {
        self.navigate_guarded(NavigationCommand::Forward, Arc::new(|| true))
            .await
    }
    pub async fn reload(self: &Arc<Self>) -> Result<(), String> {
        self.navigate_guarded(NavigationCommand::Reload, Arc::new(|| true))
            .await
    }
    pub async fn navigate_guarded(
        self: &Arc<Self>,
        command: NavigationCommand,
        guard: Arc<DispatchGuard>,
    ) -> Result<(), String> {
        if matches!(&command,NavigationCommand::Navigate(url) if !navigation_allowed(url)) {
            return Err("WK navigation requires an HTTP(S) URL without credentials".into());
        }
        self.mutate(move |native| {
            if !guard() {
                return Err("BROWSER_CANCELLED".into());
            }
            unsafe {
                if matches!(command, NavigationCommand::Back) && !native.view.canGoBack() {
                    return Ok(());
                }
                if matches!(command, NavigationCommand::Forward) && !native.view.canGoForward() {
                    return Ok(());
                }
                native.delegate.prepare_navigation();
                let navigation = match command {
                    NavigationCommand::Navigate(url) => {
                        let url = NSURL::URLWithString(&NSString::from_str(&url))
                            .ok_or("WK invalid URL")?;
                        native.view.loadRequest(&NSURLRequest::requestWithURL(&url))
                    }
                    NavigationCommand::Back => native.view.goBack(),
                    NavigationCommand::Forward => native.view.goForward(),
                    NavigationCommand::Reload => native.view.reload(),
                };
                native.delegate.track_navigation(navigation, &native.view);
            }
            Ok(())
        })
        .await
    }
    pub async fn stop_loading(self: &Arc<Self>) -> Result<(), String> {
        self.mutate(|n| {
            unsafe {
                n.view.stopLoading();
            }
            n.interactions.drain_dialogs();
            let lifecycle = n
                .owner
                .upgrade()
                .filter(|page| page.snapshot().lifecycle == BrowserTabLifecycle::Loading)
                .map(|_| BrowserTabLifecycle::Ready);
            n.delegate.refresh(&n.view, lifecycle);
            Ok(())
        })
        .await
    }
    pub async fn set_zoom_factor(self: &Arc<Self>, factor: f64) -> Result<(), String> {
        if !factor.is_finite() || !(0.25..=5.0).contains(&factor) {
            return Err("WK zoom outside range".into());
        }
        self.mutate(move |n| {
            unsafe {
                n.view.setPageZoom(factor);
            }
            Ok(())
        })
        .await
    }
    pub async fn hide(self: &Arc<Self>) -> Result<(), String> {
        let page = self.clone();
        self.mutate(move |n| {
            page.visible.store(false, Ordering::Release);
            release_browser_responder(n)?;
            // WebsiteDialog's renderer overlay hides the native child while
            // its retained JavaScript completion awaits an explicit answer.
            n.interactions.cancel_user_panels();
            n.view.setHidden(true);
            Ok(())
        })
        .await
    }
    pub async fn set_surface(
        self: &Arc<Self>,
        bounds: BrowserSurfaceBounds,
        visible: bool,
        cancel: CancellationToken,
    ) -> Result<(), String> {
        if !bounds.is_valid() {
            return Err("WK invalid surface bounds".into());
        }
        let page = self.clone();
        self.mutate(move |n| {
            if cancel.is_cancelled() {
                return Err("BROWSER_CANCELLED".into());
            }
            let y = if n.parent.isFlipped() {
                bounds.y
            } else {
                n.parent.bounds().size.height - bounds.y - bounds.height
            };
            n.view.setFrame(NSRect::new(
                NSPoint::new(bounds.x, y),
                NSSize::new(bounds.width, bounds.height),
            ));
            n.view.setHidden(!visible);
            page.visible.store(visible, Ordering::Release);
            if !visible {
                release_browser_responder(n)?;
                n.interactions.cancel_user_panels();
            }
            Ok(())
        })
        .await
    }
    pub async fn set_input_locked(self: &Arc<Self>, locked: bool) -> Result<(), String> {
        let page = self.clone();
        self.mutate(move |n| {
            page.input_locked.store(locked, Ordering::Release);
            if locked {
                // Edit-menu selectors bypass NSEvent filtering. Move the
                // responder out of WebKit before acknowledging Agent ownership.
                release_browser_responder(n)?;
                n.interactions.drain_dialogs();
                n.interactions.cancel_user_panels();
            }
            Ok(())
        })
        .await
    }
    pub async fn set_dialog_draining(self: &Arc<Self>, draining: bool) -> Result<(), String> {
        let page = self.clone();
        self.mutate(move |n| {
            page.dialog_draining.store(draining, Ordering::Release);
            if draining {
                n.interactions.drain_dialogs();
            }
            Ok(())
        })
        .await
    }
    pub async fn reply_dialog(
        self: &Arc<Self>,
        id: String,
        generation: u64,
        accept: bool,
        text: String,
        cancel: CancellationToken,
    ) -> Result<(), String> {
        self.mutate(move |n| {
            if cancel.is_cancelled() {
                return Err("BROWSER_CANCELLED".into());
            }
            n.interactions.reply_dialog(&id, generation, accept, &text)
        })
        .await
    }
    pub async fn reply_permission(
        self: &Arc<Self>,
        _id: String,
        _generation: u64,
        _allow: bool,
        _cancel: CancellationToken,
    ) -> Result<(), String> {
        Err("BROWSER_UNSUPPORTED_ACTION".into())
    }
    pub fn user_download_snapshot(&self) -> Vec<BrowserDownloadSnapshot> {
        self.downloads.lock().unwrap().clone()
    }
    pub async fn configure_user_downloads(self: &Arc<Self>, path: PathBuf) -> Result<(), String> {
        self.mutate(move |n| n.interactions.configure_user_downloads(path))
            .await
    }
    pub async fn cancel_user_download(self: &Arc<Self>, id: String) -> Result<(), String> {
        self.mutate(move |n| n.interactions.cancel_user_download(&id))
            .await
    }
    pub async fn cancel_user_downloads(self: &Arc<Self>) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.mutate(move |n| {
            n.interactions
                .cancel_user_downloads_with_completion(Box::new(move || {
                    let _ = tx.send(());
                }));
            Ok(())
        })
        .await?;
        rx.await
            .map_err(|_| "WK download cancellation acknowledgement lost".into())
    }
    pub async fn evaluate(
        self: &Arc<Self>,
        script: String,
        generation: u64,
        guard: Arc<DispatchGuard>,
        cancel: CancellationToken,
    ) -> Result<serde_json::Value, String> {
        let (tx, mut rx) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(tx)));
        let page = self.clone();
        let operation = Uuid::now_v7();
        let callback_cancel = cancel.clone();
        let pending_sender = sender.clone();
        self.pending.lock().unwrap().insert(
            operation,
            Box::new(move |error| {
                if let Some(tx) = pending_sender.lock().unwrap().take() {
                    let _ = tx.send(Err(error.into()));
                }
            }),
        );
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(native) = native_page(page.id) {
                native.delegate.refresh(&native.view, None);
            }
            let rejection = if callback_cancel.is_cancelled() || !guard() {
                Some("BROWSER_CANCELLED")
            } else if page.snapshot().document_generation != generation {
                Some(STALE)
            } else if page.close_requested.load(Ordering::Acquire) {
                Some(INTERRUPTED)
            } else if !page.is_visible() {
                Some("BROWSER_ELEMENT_NOT_ACTIONABLE")
            } else {
                None
            };
            if let Some(error) = rejection {
                page.pending.lock().unwrap().remove(&operation);
                if let Some(tx) = sender.lock().unwrap().take() {
                    let _ = tx.send(Err(error.into()));
                }
                return;
            }
            let Some(native) = native_page(page.id) else {
                page.interrupt_pending();
                return;
            };
            let callback_page = page.clone();
            let done = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                callback_page.pending.lock().unwrap().remove(&operation);
                let Some(tx) = sender.lock().unwrap().take() else {
                    return;
                };
                let result = if callback_cancel.is_cancelled()
                    || !guard()
                    || callback_page.close_requested.load(Ordering::Acquire)
                    || callback_page.snapshot().document_generation != generation
                {
                    Err(INTERRUPTED.into())
                } else if !error.is_null() {
                    // A transport/process error cannot prove a script had no
                    // effects; the host must never replay it automatically.
                    Err(INTERRUPTED.into())
                } else {
                    unsafe { value.as_ref() }
                        .and_then(|v| v.downcast_ref::<NSString>())
                        .ok_or_else(|| "WK evaluation expected a JSON string".to_owned())
                        .and_then(|v| {
                            serde_json::from_str(&v.to_string())
                                .map_err(|_| "WK evaluation returned invalid JSON".into())
                        })
                };
                if callback_page.input_locked() {
                    if let Some(native) = native_page(callback_page.id) {
                        let _ = release_browser_responder(&native);
                    }
                }
                let _ = tx.send(result);
            });
            unsafe {
                native
                    .view
                    .evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                        &NSString::from_str(&script),
                        None,
                        &native.content_world,
                        Some(&done),
                    );
            }
        });
        // A sent script is never retried and cancellation does not declare it
        // settled. Drain modal callbacks, then await WebKit's completion. A
        // nonsettling script closes its page, but retirement does not prove
        // execution settled. The host keeps its uncertainty latch/input lock.
        tokio::select! { result = &mut rx => result.map_err(|_| INTERRUPTED.to_owned())?, _ = cancel.cancelled() => {
            let _ = self.stop_loading().await;
            match tokio::time::timeout(Duration::from_secs(5), &mut rx).await { Ok(result) => result.map_err(|_| UNCONFIRMED.to_owned())?, Err(_) => {let _=self.force_close().await; Err(UNCONFIRMED.into())} }
        }}
    }
    pub async fn screenshot(
        self: &Arc<Self>,
        generation: u64,
        cancel: CancellationToken,
    ) -> Result<Vec<u8>, String> {
        let (tx, rx) = oneshot::channel();
        let page = self.clone();
        let sender = Arc::new(Mutex::new(Some(tx)));
        let pending_sender = sender.clone();
        let operation = Uuid::now_v7();
        let completion_cancel = cancel.clone();
        self.pending.lock().unwrap().insert(
            operation,
            Box::new(move |_| {
                if let Some(tx) = pending_sender.lock().unwrap().take() {
                    let _ = tx.send(Err(INTERRUPTED.into()));
                }
            }),
        );
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(native) = native_page(page.id) {
                native.delegate.refresh(&native.view, None);
            }
            if cancel.is_cancelled()
                || page.snapshot().document_generation != generation
                || !page.is_visible()
            {
                page.pending.lock().unwrap().remove(&operation);
                if let Some(tx) = sender.lock().unwrap().take() {
                    let _ = tx.send(Err(STALE.into()));
                }
                return;
            }
            let Some(native) = native_page(page.id) else {
                page.interrupt_pending();
                return;
            };
            let done = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                page.pending.lock().unwrap().remove(&operation);
                let Some(tx) = sender.lock().unwrap().take() else {
                    return;
                };
                let result = if cancel.is_cancelled()
                    || page.snapshot().document_generation != generation
                    || page.close_requested.load(Ordering::Acquire)
                    || !page.is_visible()
                {
                    Err(INTERRUPTED.into())
                } else if !error.is_null() {
                    Err("WK snapshot failed".into())
                } else {
                    image_png(image)
                };
                let _ = tx.send(result);
            });
            // Snapshot width is in points; account for the backing scale so
            // the actual PNG's longest pixel edge stays within tool limits.
            let bounds = native.view.bounds();
            let scale = native
                .view
                .window()
                .map_or(1.0, |w| w.backingScaleFactor())
                .max(1.0);
            let reduction = (1600.0 / (bounds.size.width.max(bounds.size.height) * scale)).min(1.0);
            let configuration =
                unsafe { WKSnapshotConfiguration::new(MainThreadMarker::new().unwrap()) };
            unsafe {
                configuration
                    .setSnapshotWidth(Some(&NSNumber::new_f64(bounds.size.width * reduction)));
                native
                    .view
                    .takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &done);
            }
        });
        // Snapshot is read-only: abandoning its result is safe. Removing the
        // sender makes a delayed WebKit callback inert, without claiming that
        // stopLoading cancels WebKit's snapshot work.
        tokio::select! {result=rx=>result.map_err(|_|INTERRUPTED.to_owned())?,_=completion_cancel.cancelled()=>{if let Some(reject)=self.pending.lock().unwrap().remove(&operation){reject(INTERRUPTED);}Err(INTERRUPTED.into())},_=tokio::time::sleep(Duration::from_secs(30))=>{if let Some(reject)=self.pending.lock().unwrap().remove(&operation){reject(INTERRUPTED);}Err("WK snapshot timed out".into())}}
    }
    pub(crate) fn interrupt_pending(&self) {
        self.settle_pending(INTERRUPTED);
    }
    fn settle_pending(&self, error: &'static str) {
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        for reject in pending.into_values() {
            reject(error);
        }
    }
    pub(crate) fn close_native(self: &Arc<Self>) {
        self.close_requested.store(true, Ordering::Release);
        self.visible.store(false, Ordering::Release);
        self.dialog_draining.store(true, Ordering::Release);
        if *self.closed.borrow() {
            return;
        }
        if let Some(native) = native_page(self.id) {
            let _ = release_browser_responder(&native);
            native.interactions.drain_dialogs();
            unsafe {
                native.view.stopLoading();
            }
            let page = self.clone();
            native
                .interactions
                .cancel_user_downloads_with_completion(Box::new(move || {
                    page.finish_close_native()
                }));
        } else {
            self.finish_close_native();
        }
    }
    fn finish_close_native(&self) {
        if *self.closed.borrow() {
            return;
        }
        let native = REGISTRY.with(|r| r.borrow_mut().pages.remove(&self.id));
        if let Some(native) = native {
            let _ = release_browser_responder(&native);
            native.interactions.drain_dialogs();
            unsafe {
                native.view.stopLoading();
                native.view.setNavigationDelegate(None);
                native.view.setUIDelegate(None);
            }
            native.view.removeFromSuperview();
        }
        self.settle_pending(UNCONFIRMED);
        self.changed(|s| {
            s.document_generation = s.document_generation.saturating_add(1);
            s.dialog = None;
        });
        self.closed.send_replace(true);
        self.engine.pages.lock().unwrap().remove(&self.id);
    }
    pub async fn force_close(self: &Arc<Self>) -> Result<(), String> {
        self.close_requested.store(true, Ordering::Release);
        let mut closed = self.closed();
        let page = self.clone();
        on_main(move || {
            page.close_native();
            Ok(())
        })
        .await?;
        while !*closed.borrow_and_update() {
            closed
                .changed()
                .await
                .map_err(|_| "WK close acknowledgement lost".to_owned())?;
        }
        Ok(())
    }
}
impl Drop for Page {
    fn drop(&mut self) {
        let id = self.id;
        dispatch2::DispatchQueue::main().exec_async(move || {
            let native = native_page(id);
            if let Some(native) = native {
                let _ = release_browser_responder(&native);
                native.interactions.drain_dialogs();
                native
                    .interactions
                    .cancel_user_downloads_with_completion(Box::new(move || {
                        if let Some(native) = REGISTRY.with(|r| r.borrow_mut().pages.remove(&id)) {
                            unsafe {
                                native.view.stopLoading();
                                native.view.setNavigationDelegate(None);
                                native.view.setUIDelegate(None);
                            }
                            native.view.removeFromSuperview();
                        }
                    }));
            }
        });
    }
}

pub(crate) fn create_native_page(
    page: &Arc<Page>,
    parent: Retained<NSView>,
    configuration: Option<&WKWebViewConfiguration>,
) -> Result<Rc<NativePage>, String> {
    if page.context.closed.load(Ordering::Acquire) || page.engine.closing.load(Ordering::Acquire) {
        return Err("WK owner is closing".into());
    }
    if parent.window().is_none() {
        return Err("WK parent view is not attached to a window".into());
    }
    let mtm = MainThreadMarker::new().ok_or("WK view creation requires main thread")?;
    let configuration = match configuration {
        Some(c) => c.retain(),
        None => unsafe { WKWebViewConfiguration::new(mtm) },
    };
    let store = REGISTRY
        .with(|r| {
            r.borrow()
                .contexts
                .get(&page.context.id)
                .map(|c| c.store.clone())
        })
        .ok_or("WK context is closed")?;
    unsafe {
        configuration.setWebsiteDataStore(&store);
        configuration
            .preferences()
            .setJavaScriptCanOpenWindowsAutomatically(false);
    }
    let view = crate::view::BrowserView::new(
        mtm,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(880.0, 600.0)),
        &configuration,
        Arc::downgrade(page),
    );
    let interactions = NativeInteractions::new(Arc::downgrade(page));
    let delegate = Delegate::new(mtm, Arc::downgrade(page), interactions.clone());
    unsafe {
        view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        view.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        parent.addSubview(&view);
    }
    view.setHidden(true);
    let native = Rc::new(NativePage {
        view,
        content_world: unsafe {
            WKContentWorld::worldWithName(&NSString::from_str("NomiFunAgent"), mtm)
        },
        parent,
        delegate,
        interactions,
        owner: Arc::downgrade(page),
        context: page.context.clone(),
    });
    REGISTRY.with(|r| r.borrow_mut().pages.insert(page.id, native.clone()));
    schedule_metadata(Arc::downgrade(page));
    Ok(native)
}
fn schedule_metadata(page: Weak<Page>) {
    let when = dispatch2::DispatchTime::try_from(Duration::from_millis(250))
        .expect("bounded metadata interval");
    let _ = dispatch2::DispatchQueue::main().after(when, move || {
        let Some(owner) = page.upgrade() else {
            return;
        };
        if owner.close_requested.load(Ordering::Acquire) {
            return;
        }
        if let Some(native) = native_page(owner.id) {
            native.delegate.refresh(&native.view, None);
            schedule_metadata(page);
        }
    });
}
fn release_browser_responder(native: &NativePage) -> Result<(), String> {
    if let Some(window) = native.view.window() {
        if window.firstResponder().is_some_and(|responder| {
            let is_view: bool =
                unsafe { msg_send![&responder,isKindOfClass:objc2::class!(NSView)] };
            is_view && unsafe { msg_send![&responder,isDescendantOf:&*native.view] }
        }) && !window.makeFirstResponder(None)
        {
            return Err("WK input responder could not be released".into());
        }
    }
    Ok(())
}
fn has_key_characters(kind: NSEventType) -> bool {
    matches!(kind, NSEventType::KeyDown | NSEventType::KeyUp)
}

#[cfg(test)]
mod event_contract_tests {
    use super::*;
    #[test]
    fn modifier_transitions_never_read_key_characters() {
        assert!(has_key_characters(NSEventType::KeyDown));
        assert!(has_key_characters(NSEventType::KeyUp));
        for kind in [
            NSEventType::FlagsChanged,
            NSEventType::LeftMouseDown,
            NSEventType::ScrollWheel,
            NSEventType::ApplicationDefined,
        ] {
            assert!(!has_key_characters(kind));
        }
    }
}
pub(crate) fn navigation_allowed(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https") && u.username().is_empty() && u.password().is_none()
    })
}
fn native_uuid(id: Uuid) -> Retained<NSUUID> {
    NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(&id.to_string()))
        .expect("validated UUID")
}
type StoreRemovalCompletion = Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>;
fn finish_store_removal(identifier: Uuid, tx: &StoreRemovalCompletion, result: Result<(), String>) {
    REGISTRY.with(|r| r.borrow_mut().removing.remove(&identifier));
    if let Some(tx) = tx.lock().unwrap().take() {
        let _ = tx.send(result);
    }
}
fn remove_store_attempt(identifier: Uuid, tx: StoreRemovalCompletion, attempt: u8) {
    autoreleasepool(|_| {
        let admitted = REGISTRY.with(|r| {
            let r = r.borrow();
            r.removing.contains(&identifier)
                && !r
                    .contexts
                    .values()
                    .any(|c| c.identifier == Some(identifier))
                && !r
                    .pages
                    .values()
                    .any(|p| p.context.identifier == Some(identifier))
        });
        if !admitted {
            finish_store_removal(
                identifier,
                &tx,
                Err("WK profile removal ownership changed".into()),
            );
            return;
        }
        let done = RcBlock::new(move |error: *mut NSError| {
            autoreleasepool(|_| {
                if let Some(error) = unsafe { error.as_ref() } {
                    let domain = error.domain().to_string();
                    let safe_domain = match domain.as_str() {
                        "WKWebSiteDataStore" => "WKWebSiteDataStore",
                        "WKErrorDomain" => "WKErrorDomain",
                        "NSCocoaErrorDomain" => "NSCocoaErrorDomain",
                        "NSPOSIXErrorDomain" => "NSPOSIXErrorDomain",
                        _ => "other",
                    };
                    // Classify only WebKit's fixed framework reasons. Never
                    // print arbitrary localized descriptions or userInfo.
                    let description = error.localizedDescription().to_string();
                    let busy = domain == "WKWebSiteDataStore"
                        && matches!(
                            description.as_str(),
                            "Data store is in use" | "Data store is in use (by network process)"
                        );
                    let category = if busy {
                        "native_in_use"
                    } else {
                        "native_remove_failed"
                    };
                    let code = error.code();
                    eprintln!(
                        "WK_PROFILE_REMOVE domain={safe_domain} code={code} category={category} attempt={attempt}"
                    );
                    if busy && attempt < 20 {
                        // Store deletion is idempotent. Hold the same removal
                        // lease while WebKit asynchronously releases its native
                        // network owner; neither a timeout nor in-use is success.
                        let when = dispatch2::DispatchTime::try_from(Duration::from_millis(100))
                            .expect("bounded store release interval");
                        let next = tx.clone();
                        let _ = dispatch2::DispatchQueue::main().after(when, move || {
                            remove_store_attempt(identifier, next, attempt + 1)
                        });
                        return;
                    }
                    finish_store_removal(
                        identifier,
                        &tx,
                        Err(format!(
                            "WK persistent store removal failed: domain={safe_domain} code={code} category={category}"
                        )),
                    );
                    return;
                }
                let tx = tx.clone();
                let verified =
                    RcBlock::new(move |identifiers: std::ptr::NonNull<NSArray<NSUUID>>| {
                        autoreleasepool(|_| {
                            let target = native_uuid(identifier);
                            let remains = unsafe { identifiers.as_ref() }
                                .iter()
                                .any(|id| *id == *target);
                            finish_store_removal(
                                identifier,
                                &tx,
                                if remains {
                                    Err("WK removed store remains registered".into())
                                } else {
                                    Ok(())
                                },
                            );
                        });
                    });
                unsafe {
                    WKWebsiteDataStore::fetchAllDataStoreIdentifiers(
                        &verified,
                        MainThreadMarker::new().unwrap(),
                    );
                }
            });
        });
        unsafe {
            WKWebsiteDataStore::removeDataStoreForIdentifier_completionHandler(
                &native_uuid(identifier),
                &done,
                MainThreadMarker::new().unwrap(),
            );
        }
    });
}
async fn on_main<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = oneshot::channel();
    dispatch2::DispatchQueue::main().exec_async(move || {
        // AppKit's event-loop pool may span many GCD blocks. Native ownership
        // receipts must follow release of this operation's autoreleased views,
        // configurations and stores, not wait for an unrelated user event.
        let result = autoreleasepool(|_| work());
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|_| "WK main-thread acknowledgement lost".to_owned())?
}
fn image_png(image: *mut NSImage) -> Result<Vec<u8>, String> {
    let image = unsafe { image.as_ref() }.ok_or("WK snapshot image absent")?;
    let tiff = image
        .TIFFRepresentation()
        .ok_or("WK snapshot conversion failed")?;
    let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)
        .ok_or("WK bitmap conversion failed")?;
    let data = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or("WK PNG conversion failed")?;
    Ok(data.to_vec())
}
