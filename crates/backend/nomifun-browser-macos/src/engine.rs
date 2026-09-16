//! CEF lifetime and main-thread scheduling. This module never owns the Tauri UI.
use cef::*;
use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, Mutex, OnceLock, Weak, atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering}}, time::{Duration, Instant}};
use tokio::sync::{oneshot, watch};
use crate::protocol::Protocol;
#[path = "callbacks.rs"]
mod callbacks;
#[path = "site_data.rs"]
mod site_data;
pub use callbacks::{NativeDialog, PageSnapshot};

pub type UiWork = Box<dyn FnOnce() + Send>;
pub type ParentView = dyn Fn() -> Result<objc2::rc::Retained<objc2_app_kit::NSView>, String> + Send + Sync;

pub struct Paths {
    pub framework: PathBuf,
    pub helper: PathBuf,
    pub main_bundle: PathBuf,
    pub data_root: PathBuf,
}

static INITIALIZED: OnceLock<()> = OnceLock::new();
const BOOTSTRAP_URL: &str = "data:text/html,";

/// Only the desktop host can construct this process-wide owner. All CEF object
/// mutations happen on the application's existing main thread.
pub struct Engine {
    ready: watch::Sender<bool>,
    stopped: AtomicBool,
    closing: AtomicBool,
    pump_generation: AtomicU64,
    pump_due: Mutex<Option<Instant>>,
    pump_active: AtomicBool,
    pump_reentered: AtomicBool,
    pages: Mutex<BTreeMap<uuid::Uuid, Weak<Page>>>,
    contexts: Mutex<BTreeMap<uuid::Uuid, Weak<Context>>>,
    pointer_owners: Mutex<BTreeMap<isize, Weak<Page>>>,
    root: PathBuf,
}

impl Engine {
    /// Must be called on the main thread after the application implements
    /// CefAppProtocol, and before the backend advertises a native provider.
    pub fn initialize(paths: Paths) -> Result<Arc<Self>, String> {
        if objc2::MainThreadMarker::new().is_none() { return Err("CEF initialization requires the main thread".into()); }
        if INITIALIZED.set(()).is_err() { return Err("CEF cannot be initialized twice in one process".into()); }
        let framework = paths.framework.canonicalize().map_err(|_| "CEF framework is missing")?;
        let helper = paths.helper.canonicalize().map_err(|_| "CEF helper is missing")?;
        let main_bundle = paths.main_bundle.canonicalize().map_err(|_| "CEF main bundle is missing")?;
        std::fs::create_dir_all(&paths.data_root).map_err(|_| "CEF data root cannot be created")?;
        let root = paths.data_root.canonicalize().map_err(|_| "CEF data root cannot be resolved")?;
        let library = std::ffi::CString::new(framework.join("Chromium Embedded Framework").as_os_str().as_encoded_bytes())
            .map_err(|_| "CEF framework path is invalid")?;
        if unsafe { load_library(Some(&*library.as_ptr().cast())) } != 1 { return Err("CEF framework could not be loaded".into()); }
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
        let (ready, _) = watch::channel(false);
        let engine = Arc::new(Self { ready, stopped: AtomicBool::new(false), closing: AtomicBool::new(false), pump_generation: AtomicU64::new(0), pump_due: Mutex::new(None), pump_active: AtomicBool::new(false), pump_reentered: AtomicBool::new(false), pages: Mutex::new(BTreeMap::new()), contexts: Mutex::new(BTreeMap::new()), pointer_owners: Mutex::new(BTreeMap::new()), root });
        crate::application::install(Arc::downgrade(&engine))?;
        let args = args::Args::new();
        let helper_text = crate::text::Text::new(helper.to_str().ok_or("CEF helper path must be UTF-8")?);
        let framework_text = crate::text::Text::new(framework.to_str().ok_or("CEF framework path must be UTF-8")?);
        let bundle_text = crate::text::Text::new(main_bundle.to_str().ok_or("CEF bundle path must be UTF-8")?);
        let root_text = crate::text::Text::new(engine.root.to_str().ok_or("CEF data root must be UTF-8")?);
        let settings = Settings {
            // CEF copies these fields during initialize; buffers stay alive
            // until that call returns. See text.rs for the binding boundary.
            browser_subprocess_path: unsafe { helper_text.field() },
            framework_dir_path: unsafe { framework_text.field() },
            main_bundle_path: unsafe { bundle_text.field() },
            root_cache_path: unsafe { root_text.field() },
            no_sandbox: 0,
            external_message_pump: 1,
            command_line_args_disabled: 1,
            windowless_rendering_enabled: 0,
            remote_debugging_port: 0,
            ..Default::default()
        };
        let mut app = Application::new(engine.clone());
        if initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
            engine.stopped.store(true, Ordering::Release);
            return Err("CEF initialization failed".into());
        }
        Ok(engine)
    }

    pub fn post(self: &Arc<Self>, work: UiWork) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) { return Err("CEF is stopped".into()); }
        // CEF work belongs to its UI task runner, even though that runner and
        // Tauri share the same macOS main thread. Serialize the CEF API call
        // with shutdown on that thread: an async caller must never race a
        // check-then-post against cef_shutdown from another thread.
        let engine = self.clone();
        dispatch2::DispatchQueue::main().exec_async(move || {
            if engine.stopped.load(Ordering::Acquire) { return; }
            let mut task = NativeTask::new(Arc::new(Mutex::new(Some(work))));
            let _ = post_task(ThreadId::UI, Some(&mut task));
            // A rejected task drops its owned oneshot sender. The waiter gets
            // failure rather than a synthetic completion acknowledgement.
        });
        Ok(())
    }

    pub(crate) fn blocks_user_event(&self, event: &objc2_app_kit::NSEvent) -> bool {
        use objc2_app_kit::{NSEventType, NSEventModifierFlags, NSView};
        use objc2::msg_send;
        let kind = event.r#type();
        let keyboard = matches!(kind, NSEventType::KeyDown | NSEventType::KeyUp | NSEventType::FlagsChanged);
        if matches!(kind, NSEventType::KeyDown | NSEventType::KeyUp) && event.modifierFlags().contains(NSEventModifierFlags::Command)
            && event.charactersIgnoringModifiers().is_some_and(|value| value.to_string().eq_ignore_ascii_case("q")) {
            // Application Quit belongs to Tauri's existing exit coordinator.
            return false;
        }
        let pointer = matches!(kind, NSEventType::LeftMouseDown | NSEventType::LeftMouseUp | NSEventType::RightMouseDown | NSEventType::RightMouseUp | NSEventType::OtherMouseDown | NSEventType::OtherMouseUp | NSEventType::MouseMoved | NSEventType::LeftMouseDragged | NSEventType::RightMouseDragged | NSEventType::OtherMouseDragged | NSEventType::ScrollWheel | NSEventType::Magnify | NSEventType::Rotate | NSEventType::Swipe | NSEventType::BeginGesture | NSEventType::EndGesture | NSEventType::Pressure);
        if !keyboard && !pointer { return false; }
        let Some(mtm) = objc2::MainThreadMarker::new() else { return true; };
        let window = event.window(mtm).or_else(|| if keyboard { objc2_app_kit::NSApplication::sharedApplication(mtm).keyWindow() } else { None });
        let down = matches!(kind, NSEventType::LeftMouseDown | NSEventType::RightMouseDown | NSEventType::OtherMouseDown);
        let up = matches!(kind, NSEventType::LeftMouseUp | NSEventType::RightMouseUp | NSEventType::OtherMouseUp);
        let dragged = matches!(kind, NSEventType::LeftMouseDragged | NSEventType::RightMouseDragged | NSEventType::OtherMouseDragged);
        if up || dragged {
            let owner = if up { self.pointer_owners.lock().unwrap().remove(&event.buttonNumber()) }
                else { self.pointer_owners.lock().unwrap().get(&event.buttonNumber()).cloned() };
            if let Some(page) = owner.and_then(|owner| owner.upgrade()).filter(|page| page.input_locked()) {
                page.blocked_inputs.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        let pages: Vec<_> = self.pages.lock().unwrap().values().filter_map(Weak::upgrade).collect();
        let hit = pages.iter().find(|page| {
            let view = page.view.load(Ordering::Acquire);
            let Some(view) = (unsafe { (view as *const NSView).as_ref() }) else { return false; };
            let Some(page_window) = view.window() else { return false; };
            if view.isHidden() || window.as_ref().is_some_and(|window| **window != *page_window) { return false; }
            if keyboard {
                let Some(responder) = page_window.firstResponder() else { return false; };
                let is_view: objc2::runtime::Bool = unsafe { msg_send![&responder, isKindOfClass: objc2::class!(NSView)] };
                is_view.as_bool() && unsafe { msg_send![&responder, isDescendantOf: view] }
            } else {
                let location = if window.is_some() { event.locationInWindow() } else { page_window.convertPointFromScreen(event.locationInWindow()) };
                let point = view.convertPoint_fromView(location, None);
                let bounds = view.bounds();
                point.x >= 0.0 && point.y >= 0.0 && point.x < bounds.size.width && point.y < bounds.size.height
            }
        });
        if down {
            let mut owners = self.pointer_owners.lock().unwrap();
            if let Some(page) = hit.filter(|_| (0..32).contains(&event.buttonNumber())) { owners.insert(event.buttonNumber(), Arc::downgrade(page)); }
            else { owners.remove(&event.buttonNumber()); }
        }
        if let Some(page) = hit.filter(|page| page.input_locked()) {
            page.blocked_inputs.fetch_add(1, Ordering::Relaxed);
            true
        } else { false }
    }

    fn schedule(self: &Arc<Self>, delay: i64) {
        // CEF's reference external pump requires a maximum 30 Hz fallback,
        // even when no further OnScheduleMessagePumpWork callback arrives.
        // See tests/shared/browser/main_message_loop_external_pump.cc upstream.
        let delay = Duration::from_millis(delay.clamp(0, 33) as u64);
        let due = Instant::now() + delay;
        let mut pending = self.pump_due.lock().unwrap();
        if pending.is_some_and(|pending| pending <= due) { return; }
        *pending = Some(due);
        let generation = self.pump_generation.fetch_add(1, Ordering::AcqRel) + 1;
        drop(pending);
        let weak = Arc::downgrade(self);
        let work = Box::new(move || {
            if let Some(engine) = weak.upgrade() {
                if engine.stopped.load(Ordering::Acquire) { return; }
                // Claim under the same lock used by schedule(). A stale task
                // must never erase a newer immediate wakeup's deadline.
                let mut pending = engine.pump_due.lock().unwrap();
                if engine.pump_generation.load(Ordering::Acquire) != generation { return; }
                pending.take();
                drop(pending);
                if engine.pump_active.swap(true, Ordering::AcqRel) {
                    engine.pump_reentered.store(true, Ordering::Release);
                    return;
                }
                engine.pump_reentered.store(false, Ordering::Release);
                do_message_loop_work();
                engine.pump_active.store(false, Ordering::Release);
                if !engine.stopped.load(Ordering::Acquire) {
                    engine.schedule(if engine.pump_reentered.swap(false, Ordering::AcqRel) { 0 } else { 33 });
                }
            }
        });
        if delay.is_zero() { dispatch2::DispatchQueue::main().exec_async(work); }
        else {
            let when = dispatch2::DispatchTime::try_from(delay).expect("bounded CEF delay");
            let _ = dispatch2::DispatchQueue::main().after(when, work);
        }
    }

    pub async fn wait_ready(&self) -> Result<(), String> {
        if self.closing.load(Ordering::Acquire) { return Err("CEF is closing".into()); }
        let mut ready = self.ready.subscribe();
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while !*ready.borrow_and_update() {
                if self.stopped.load(Ordering::Acquire) || self.closing.load(Ordering::Acquire) { return Err("CEF initialization stopped".into()); }
                ready.changed().await.map_err(|_| "CEF readiness channel closed")?;
            }
            Ok(())
        }).await.map_err(|_| "CEF initialization timed out")?
    }

    /// Construct this once per Conversation, including ephemeral conversations.
    /// Every tab in that Conversation receives the same Context owner.
    pub async fn create_context(self: &Arc<Self>, profile: Option<PathBuf>) -> Result<Arc<Context>, String> {
        self.wait_ready().await?;
        let profile = profile.map(|path| crate::profile::resolve(&self.root, &path)).transpose()?;
        let (tx, rx) = oneshot::channel();
        let owner = self.clone();
        self.post(Box::new(move || {
            if owner.closing.load(Ordering::Acquire) { let _ = tx.send(Err("CEF is closing".into())); return; }
            let path = match profile.as_ref().map(|path| path.to_str().ok_or("CEF profile path must be UTF-8")).transpose() {
                Ok(path) => path.unwrap_or(""),
                Err(error) => { let _ = tx.send(Err(error.into())); return; }
            };
            let path_text = crate::text::Text::new(path);
            let settings = RequestContextSettings { cache_path: unsafe { path_text.field() }, persist_session_cookies: i32::from(profile.is_some()), ..Default::default() };
            let result = request_context_create_context(Some(&settings), None).filter(|raw| CefString::from(&raw.cache_path()).to_string() == path).map(|raw| {
                let context = Arc::new(Context { raw: Mutex::new(Some(raw)), profile });
                owner.contexts.lock().unwrap().insert(uuid::Uuid::now_v7(), Arc::downgrade(&context));
                context
            })
                .ok_or_else(|| "CEF request context creation failed".to_owned());
            let _ = tx.send(result);
        }))?;
        rx.await.map_err(|_| "CEF request context creation was interrupted")?
    }

    /// Resolve and retain the parent on the UI thread. No raw native handle is
    /// accepted from a serialized request, and queued work cannot use a freed view.
    pub async fn create_page(self: &Arc<Self>, parent: Arc<ParentView>, context: Arc<Context>) -> Result<Arc<Page>, String> {
        self.wait_ready().await?;
        let (created, wait) = oneshot::channel();
        let (closed, _) = watch::channel(false);
        let page = Arc::new_cyclic(|weak: &Weak<Page>| {
            let weak = weak.clone();
            let engine = self.clone();
            let protocol = Protocol::new(Box::new(move |message| {
                let weak = weak.clone();
                engine.post(Box::new(move || {
                    if let Some(page) = weak.upgrade() {
                        if message.guard.as_ref().is_some_and(|guard| !guard()) { page.protocol.reject(message.id); return; }
                        let browser = page.browser.lock().unwrap().clone();
                        if let Some(browser) = browser {
                            if let Some(host) = browser.host() {
                                if host.send_dev_tools_message(Some(&message.bytes)) == 1 { return; }
                            }
                        }
                        page.protocol.close();
                    }
                }))
            }));
            Page { metadata: watch::channel(PageSnapshot::default()).0, change_listener: Mutex::new(None), dialog: Mutex::new(None), dialog_draining: AtomicBool::new(true), id: uuid::Uuid::now_v7(), engine: self.clone(), protocol, browser: Mutex::new(None), registration: Mutex::new(None), created: Mutex::new(Some(created)), closed, view: AtomicUsize::new(0), parent: AtomicUsize::new(0), input_locked: AtomicBool::new(true), blocked_inputs: AtomicUsize::new(0), visible: AtomicBool::new(false), close_requested: AtomicBool::new(false), _context: context.clone() }
        });
        self.pages.lock().unwrap().insert(page.id, Arc::downgrade(&page));
        let pending = page.clone();
        self.post(Box::new(move || {
            if pending.engine.closing.load(Ordering::Acquire) { pending.creation_failed(); return; }
            let Ok(parent) = parent() else { pending.creation_failed(); return; };
            if parent.window().is_none() { pending.creation_failed(); return; }
            let parent = objc2::rc::Retained::into_raw(parent);
            pending.parent.store(parent as usize, Ordering::Release);
            let Some(mut context) = context.raw.lock().unwrap().clone() else { pending.creation_failed(); return; };
            let window = WindowInfo { parent_view: parent.cast(), bounds: Rect { x: 0, y: 0, width: 880, height: 600 }, hidden: 1, runtime_style: RuntimeStyle::ALLOY, ..Default::default() };
            let mut client = PageClient::new(pending.clone());
            // A fresh off-the-record context may defer renderer creation for
            // about:blank. Bootstrap the same view with inert, host-owned HTML
            // so the protocol can attach before navigating to an untrusted site.
            let bootstrap = CefString::from(BOOTSTRAP_URL);
            if browser_host_create_browser(Some(&window), Some(&mut client), Some(&bootstrap), Some(&BrowserSettings::default()), None, Some(&mut context)) != 1 { pending.creation_failed(); }
        }))?;
        // No timeout that would orphan a late native creation. The owner remains
        // retained by CEF until its create/abort callback; app shutdown owns it.
        wait.await.map_err(|_| "CEF page creation was interrupted")??;
        Ok(page)
    }

    pub async fn shutdown(self: &Arc<Self>) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) { return Ok(()); }
        self.closing.store(true, Ordering::Release);
        self.ready.send_replace(false);
        let pages: Vec<_> = self.pages.lock().unwrap().values().filter_map(Weak::upgrade).collect();
        for page in pages { page.force_close().await?; }
        let (tx, rx) = oneshot::channel();
        let engine = self.clone();
        dispatch2::DispatchQueue::main().exec_async(move || {
            if engine.pages.lock().unwrap().values().any(|page| page.strong_count() != 0) {
                let _ = tx.send(Err("CEF pages remain during shutdown".to_owned()));
                return;
            }
            let contexts: Vec<_> = std::mem::take(&mut *engine.contexts.lock().unwrap()).into_values().filter_map(|context| context.upgrade()).collect();
            for context in contexts { let raw = context.raw.lock().unwrap().take(); drop(raw); }
            if !engine.stopped.swap(true, Ordering::AcqRel) { shutdown(); }
            let _ = tx.send(Ok(()));
        });
        rx.await.map_err(|_| "CEF shutdown acknowledgement was lost")?
    }
}

wrap_task! { struct NativeTask { work: Arc<Mutex<Option<UiWork>>>, } impl Task {
    fn execute(&self) {
        let work = self.work.lock().unwrap().take();
        if let Some(work) = work { work(); }
    }
} }

pub struct Context {
    raw: Mutex<Option<RequestContext>>,
    pub profile: Option<PathBuf>,
}

pub struct Page {
    metadata: watch::Sender<PageSnapshot>,
    change_listener: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    dialog: Mutex<Option<callbacks::DeferredDialog>>,
    dialog_draining: AtomicBool,
    id: uuid::Uuid,
    engine: Arc<Engine>,
    pub protocol: Arc<Protocol>,
    browser: Mutex<Option<Browser>>,
    registration: Mutex<Option<Registration>>,
    created: Mutex<Option<oneshot::Sender<Result<(), String>>>>,
    closed: watch::Sender<bool>,
    view: AtomicUsize,
    parent: AtomicUsize,
    input_locked: AtomicBool,
    blocked_inputs: AtomicUsize,
    visible: AtomicBool,
    close_requested: AtomicBool,
    _context: Arc<Context>,
}

impl Page {
    pub fn id(&self) -> uuid::Uuid { self.id }
    pub fn closed(&self) -> watch::Receiver<bool> { self.closed.subscribe() }
    /// Native browser zoom, independent of backing scale and CSS transforms.
    pub async fn set_zoom_factor(self: &Arc<Self>, factor: f64) -> Result<(), String> {
        if !factor.is_finite() || !(0.25..=5.0).contains(&factor) { return Err("CEF zoom factor is outside its range".into()); }
        let page = self.clone();
        let (tx, rx) = oneshot::channel();
        self.engine.post(Box::new(move || {
            let browser = page.browser.lock().unwrap().clone();
            let result = if let Some(host) = browser.and_then(|browser| browser.host()) {
                let level = factor.ln() / 1.2_f64.ln();
                host.set_zoom_level(level);
                if (host.zoom_level() - level).abs() < 1e-9 { Ok(()) } else { Err("CEF zoom readback differs".into()) }
            } else { Err("CEF page is closed".into()) };
            let _ = tx.send(result);
        }))?;
        rx.await.map_err(|_| "CEF zoom acknowledgement was lost")?
    }
    pub async fn hide(self: &Arc<Self>) -> Result<(), String> {
        let page = self.clone();
        let (tx, rx) = oneshot::channel();
        self.engine.post(Box::new(move || {
            if let Some(view) = unsafe { (page.view.load(Ordering::Acquire) as *const objc2_app_kit::NSView).as_ref() } {
                view.setHidden(true);
            }
            page.visible.store(false, Ordering::Release);
            let _ = tx.send(());
        }))?;
        rx.await.map_err(|_| "CEF hide acknowledgement was lost".into())
    }
    fn creation_failed(&self) {
        self.release_parent();
        self.protocol.close();
        if let Some(tx) = self.created.lock().unwrap().take() { let _ = tx.send(Err("CEF page creation failed".into())); }
        self.closed.send_replace(true);
        self.engine.pages.lock().unwrap().remove(&self.id);
    }
    fn release_parent(&self) {
        // Called only by CEF/UI creation and destruction callbacks.
        debug_assert!(objc2::MainThreadMarker::new().is_some());
        let parent = self.parent.swap(0, Ordering::AcqRel);
        if parent != 0 { drop(unsafe { objc2::rc::Retained::from_raw(parent as *mut objc2_app_kit::NSView) }); }
    }
    /// Used after explicit discard/clear confirmation or application shutdown.
    pub async fn force_close(self: &Arc<Self>) -> Result<(), String> {
        self.close_requested.store(true, Ordering::Release);
        let mut closed = self.closed.subscribe();
        if *closed.borrow() { return Ok(()); }
        let page = self.clone();
        self.engine.post(Box::new(move || {
            page.dialog_draining.store(true, Ordering::Release);
            page.clear_dialog(true);
            let browser = page.browser.lock().unwrap().clone();
            if let Some(host) = browser.and_then(|browser| browser.host()) { host.close_browser(1); }
        }))?;
        while !*closed.borrow_and_update() { closed.changed().await.map_err(|_| "CEF close confirmation channel ended")?; }
        Ok(())
    }

    pub async fn set_surface(self: &Arc<Self>, bounds: nomifun_browser_platform::runtime::BrowserSurfaceBounds, visible: bool, cancel: tokio_util::sync::CancellationToken) -> Result<(), String> {
        if !bounds.is_valid() { return Err("CEF surface bounds are invalid".into()); }
        let page = self.clone();
        let (tx, rx) = oneshot::channel();
        self.engine.post(Box::new(move || {
            if cancel.is_cancelled() { let _ = tx.send(Ok(())); return; }
            let result = (|| {
                let view = page.view.load(Ordering::Acquire);
                let view = unsafe { (view as *const objc2_app_kit::NSView).as_ref() }.ok_or("CEF page is closed")?;
                let parent = unsafe { view.superview() }.ok_or("CEF page is detached")?;
                let y = if parent.isFlipped() { bounds.y } else { parent.bounds().size.height - bounds.y - bounds.height };
                view.setFrame(objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(bounds.x, y), objc2_foundation::NSSize::new(bounds.width, bounds.height)));
                view.setHidden(!visible);
                page.visible.store(visible, Ordering::Release);
                Ok(())
            })().map_err(str::to_owned);
            let _ = tx.send(result);
        }))?;
        rx.await.map_err(|_| "CEF surface update was interrupted")?
    }

    pub fn input_locked(&self) -> bool { self.input_locked.load(Ordering::Acquire) }
    pub fn blocked_input_count(&self) -> usize { self.blocked_inputs.load(Ordering::Acquire) }

    pub async fn set_input_locked(self: &Arc<Self>, locked: bool) -> Result<(), String> {
        let page = self.clone();
        let (tx, rx) = oneshot::channel();
        self.engine.post(Box::new(move || {
            let result = if page.view.load(Ordering::Acquire) == 0 || page.protocol.is_closed() { Err("CEF input gate has no live page".into()) }
                else {
                    let browser = page.browser.lock().unwrap().clone();
                    if let Some(host) = browser.and_then(|browser| browser.host()) {
                        page.input_locked.store(locked, Ordering::Release);
                        host.set_accessibility_state(if locked { State::DISABLED } else { State::ENABLED });
                        Ok(())
                    } else { Err("CEF input gate has no browser host".into()) }
                };
            let _ = tx.send(result);
        }))?;
        rx.await.map_err(|_| "CEF input gate acknowledgement was lost")?
    }
}

wrap_app! { struct Application { engine: Arc<Engine>, } impl App {
    fn on_before_command_line_processing(&self, _process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
        if let Some(command_line) = command_line {
            // CEF 152.0.6: native tracing proves the experimental declarative
            // observer store misses the ClearAllData deadline (data_type 16).
            // Disable that nonessential reporting API before contexts exist;
            // retain the complete storageTypes=all cleanup and its callback.
            let key = CefString::from("disable-features");
            let existing = CefString::from(&command_line.switch_value(Some(&key))).to_string();
            let value = if existing.is_empty() { "DeclarativePerformanceObserver".to_owned() }
                else { format!("{existing},DeclarativePerformanceObserver") };
            command_line.append_switch_with_value(Some(&key), Some(&CefString::from(value.as_str())));
        }
    }
    fn browser_process_handler(&self) -> Option<BrowserProcessHandler> { Some(ProcessHandler::new(self.engine.clone())) }
} }
wrap_browser_process_handler! { struct ProcessHandler { engine: Arc<Engine>, } impl BrowserProcessHandler {
    fn on_context_initialized(&self) { self.engine.ready.send_replace(true); }
    fn on_schedule_message_pump_work(&self, delay_ms: i64) { self.engine.schedule(delay_ms); }
} }
wrap_client! { struct PageClient { page: Arc<Page>, } impl Client {
    fn dialog_handler(&self) -> Option<DialogHandler> { Some(callbacks::FileDialogs::new()) }
    fn download_handler(&self) -> Option<DownloadHandler> { Some(callbacks::Downloads::new()) }
    fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(callbacks::Menus::new(self.page.clone())) }
    fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(callbacks::Dialogs::new(self.page.clone())) }
    fn load_handler(&self) -> Option<LoadHandler> { Some(callbacks::Loading::new(self.page.clone())) }
    fn display_handler(&self) -> Option<DisplayHandler> { Some(callbacks::Display::new(self.page.clone())) }
    fn permission_handler(&self) -> Option<PermissionHandler> { Some(callbacks::Permissions::new(self.page.clone())) }
    fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(Lifetime::new(self.page.clone())) }
    fn drag_handler(&self) -> Option<DragHandler> { Some(ExternalDrag::new(self.page.clone())) }
    fn request_handler(&self) -> Option<RequestHandler> { Some(RequestPolicy::new(self.page.clone())) }
} }
wrap_life_span_handler! { struct Lifetime { page: Arc<Page>, } impl LifeSpanHandler {
    fn on_before_popup(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _popup_id: i32, _target_url: Option<&CefString>, _target_frame_name: Option<&CefString>, _target_disposition: WindowOpenDisposition, _user_gesture: i32, _popup_features: Option<&PopupFeatures>, _window_info: Option<&mut WindowInfo>, _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>, _extra_info: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut i32>) -> i32 {
        // Admission must eventually supply an owned child NSView and PageClient.
        // Never let CEF create an unmanaged window while that owner is absent.
        1
    }
    fn on_after_created(&self, browser: Option<&mut Browser>) {
        let Some(browser) = browser else { self.page.creation_failed(); return; };
        let Some(host) = browser.host() else { self.page.creation_failed(); return; };
        self.page.view.store(host.window_handle() as usize, Ordering::Release);
        host.set_accessibility_state(State::DISABLED);
        if let Some(view) = unsafe { host.window_handle().cast::<objc2_app_kit::NSView>().as_ref() } { view.setHidden(true); }
        let mut observer = Observer::new(self.page.protocol.clone());
        let registration = host.add_dev_tools_message_observer(Some(&mut observer));
        let registered = registration.is_some();
        *self.page.registration.lock().unwrap() = registration;
        *self.page.browser.lock().unwrap() = Some(browser.clone());
        let created = self.page.created.lock().unwrap().take();
        if let Some(tx) = created {
            let closing = self.page.close_requested.load(Ordering::Acquire) || self.page.engine.closing.load(Ordering::Acquire);
            let result = if !registered { Err("CEF observer registration failed".into()) } else if closing { Err("CEF page creation was cancelled".into()) } else { Ok(()) };
            if tx.send(result).is_err() || !registered || closing { host.close_browser(1); }
        }
    }
    fn do_close(&self, _browser: Option<&mut Browser>) -> i32 {
        let page = self.page.clone();
        // Detach only this child after CEF has acknowledged close. Never send
        // performClose to the application's main window.
        let _ = self.page.engine.post(Box::new(move || {
            let view = page.view.swap(0, Ordering::AcqRel);
            if let Some(view) = unsafe { (view as *const objc2_app_kit::NSView).as_ref() } { view.removeFromSuperview(); }
        }));
        1
    }
    fn on_before_close(&self, _browser: Option<&mut Browser>) {
        self.page.clear_dialog(false);
        self.page.protocol.close();
        let registration = self.page.registration.lock().unwrap().take();
        let browser = self.page.browser.lock().unwrap().take();
        drop(registration);
        drop(browser);
        self.page.view.store(0, Ordering::Release);
        if let Some(tx) = self.page.created.lock().unwrap().take() { let _ = tx.send(Err("CEF page closed during creation".into())); }
        self.page.release_parent();
        self.page.engine.pages.lock().unwrap().remove(&self.page.id);
        self.page.closed.send_replace(true);
    }
} }
wrap_drag_handler! { struct ExternalDrag { page: Arc<Page>, } impl DragHandler {
    fn on_drag_enter(&self, _browser: Option<&mut Browser>, _drag_data: Option<&mut DragData>, _mask: DragOperationsMask) -> i32 {
        i32::from(self.page.input_locked())
    }
} }
wrap_request_handler! { struct RequestPolicy { page: Arc<Page>, } impl RequestHandler {
    fn on_before_browse(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>, _user_gesture: i32, _is_redirect: i32) -> i32 {
        let Some(frame) = frame else { return 1; };
        if frame.is_main() == 0 { return 0; }
        let Some(request) = request else { return 1; };
        let url = CefString::from(&request.url()).to_string();
        i32::from(!navigation_allowed(&url))
    }
    fn on_render_process_terminated(&self, _browser: Option<&mut Browser>, _status: TerminationStatus, _error_code: i32, _error_string: Option<&CefString>) {
        self.page.input_locked.store(true, Ordering::Release);
        self.page.clear_dialog(false);
        self.page.changed(|state| state.lifecycle = nomifun_browser_platform::runtime::BrowserTabLifecycle::Crashed);
        self.page.protocol.close();
    }
} }

fn navigation_allowed(value: &str) -> bool {
    if value == BOOTSTRAP_URL || value == "about:blank" { return true; }
    url::Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.username().is_empty() && url.password().is_none() && url.host_str().is_some())
}
wrap_dev_tools_message_observer! { struct Observer { protocol: Arc<Protocol>, } impl DevToolsMessageObserver {
    fn on_dev_tools_message(&self, _browser: Option<&mut Browser>, message: Option<&[u8]>) -> i32 {
        if let Some(message) = message { self.protocol.receive(message); } else { self.protocol.close(); }
        1
    }
    fn on_dev_tools_agent_detached(&self, _browser: Option<&mut Browser>) { self.protocol.close(); }
} }
