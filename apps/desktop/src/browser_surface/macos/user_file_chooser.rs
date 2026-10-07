//! UserReady file chooser for CEF. The website supplies only the exact native
//! chooser event; AppKit supplies paths, and document/input ownership gates the
//! result before it reaches `DOM.setFileInputFiles`.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSWindow};
use objc2_foundation::{NSString, NSURL};
use serde_json::json;
use tauri::Manager;
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio_util::sync::CancellationToken;

use super::{
    native::{self, View, file_chooser::Choice},
    super::automation::TabAutomation,
};

type Selection = Result<Option<Vec<PathBuf>>, String>;

struct PanelControl {
    app: tauri::AppHandle,
    raw: AtomicUsize,
    outcome: watch::Sender<Option<Selection>>,
}

#[derive(Clone)]
struct NativeOpenPanel(Arc<PanelControl>);

impl NativeOpenPanel {
    fn start(
        app: tauri::AppHandle,
        initial_directory: PathBuf,
        multiple: bool,
    ) -> Result<Self, String> {
        if !initial_directory.is_absolute() {
            return Err("Browser file picker directory is invalid".into());
        }
        let control = Arc::new(PanelControl {
            app: app.clone(),
            raw: AtomicUsize::new(0),
            outcome: watch::channel(None).0,
        });
        let setup = control.clone();
        app.run_on_main_thread(move || {
            let result = (|| {
                let mtm = objc2::MainThreadMarker::new()
                    .ok_or("Browser file picker requires the main thread")?;
                let window = setup
                    .app
                    .get_window("main")
                    .ok_or("Browser file picker parent window is unavailable")?;
                let raw_window = window
                    .ns_window()
                    .map_err(|_| "Browser file picker parent window is unavailable")?;
                let window = unsafe { raw_window.cast::<NSWindow>().as_ref() }
                    .ok_or("Browser file picker parent window is null")?;
                let panel = NSOpenPanel::openPanel(mtm);
                panel.setCanChooseFiles(true);
                panel.setCanChooseDirectories(false);
                panel.setAllowsMultipleSelection(multiple);
                panel.setResolvesAliases(true);
                panel.setCanCreateDirectories(false);
                panel.setTitle(Some(&NSString::from_str("NomiFun — Select files")));
                let directory = NSString::from_str(
                    initial_directory
                        .to_str()
                        .ok_or("Browser file picker directory must be UTF-8")?,
                );
                panel.setDirectoryURL(Some(&NSURL::fileURLWithPath_isDirectory(
                    &directory, true,
                )));

                let retained = panel.clone();
                let raw = Retained::into_raw(retained);
                setup.raw.store(raw as usize, Ordering::Release);
                let completed = setup.clone();
                let handler = RcBlock::new(move |response| {
                    let raw = completed.raw.swap(0, Ordering::AcqRel);
                    if raw == 0 {
                        return;
                    }
                    let panel = unsafe { Retained::from_raw(raw as *mut NSOpenPanel) }
                        .expect("non-null retained browser file panel");
                    let selection: Selection = (|| {
                        if response == NSModalResponseOK {
                            let urls = panel.URLs();
                            if urls.len() > 256 {
                                return Err("Browser file picker selected too many files".into());
                            }
                            let mut paths = Vec::with_capacity(urls.len());
                            for url in urls.iter() {
                                let path = url
                                    .path()
                                    .map(|path| PathBuf::from(path.to_string()))
                                    .filter(|path| path.is_absolute())
                                    .ok_or("Browser file picker returned an invalid path")?;
                                paths.push(path);
                            }
                            if paths.is_empty() {
                                Ok(None)
                            } else {
                                Ok(Some(paths))
                            }
                        } else {
                            Ok(None)
                        }
                    })();
                    completed.outcome.send_replace(Some(selection));
                    drop(panel);
                });
                panel.beginSheetModalForWindow_completionHandler(window, &handler);
                Ok::<_, &'static str>(())
            })();
            if let Err(error) = result {
                setup.outcome.send_replace(Some(Err(error.into())));
            }
        })
        .map_err(|_| "Browser file picker dispatch failed")?;
        Ok(Self(control))
    }

    fn cancel(&self) {
        let control = self.0.clone();
        let _ = self.0.app.run_on_main_thread(move || {
            let raw = control.raw.load(Ordering::Acquire);
            if let Some(panel) = unsafe { (raw as *const NSOpenPanel).as_ref() } {
                unsafe { panel.cancel(None) };
            }
        });
    }

    async fn finished(&self) -> Selection {
        let mut outcome = self.0.outcome.subscribe();
        loop {
            if let Some(result) = outcome.borrow().clone() {
                return result;
            }
            outcome
                .changed()
                .await
                .map_err(|_| "Browser file picker completion was lost")?;
        }
    }
}

struct RequestControl {
    cancel: CancellationToken,
    panel: Mutex<Option<NativeOpenPanel>>,
}

pub(crate) struct UserFileChooser {
    app: tauri::AppHandle,
    view: View,
    automation: Arc<AsyncMutex<TabAutomation>>,
    initial_directory: PathBuf,
    input_locked: Arc<AtomicBool>,
    visible: Arc<AtomicBool>,
    closed_flag: Arc<AtomicBool>,
    closed: CancellationToken,
    request: Arc<Mutex<Option<Arc<RequestControl>>>>,
    done: watch::Sender<Option<Result<(), String>>>,
    _navigation: nomifun_browser_macos::protocol::CallbackSubscription,
}

impl UserFileChooser {
    pub(crate) async fn install(
        app: tauri::AppHandle,
        view: View,
        automation: Arc<AsyncMutex<TabAutomation>>,
        input_locked: Arc<AtomicBool>,
        initial_directory: PathBuf,
    ) -> Result<Arc<Self>, String> {
        let (done, _) = watch::channel(None);
        let navigation_request = Arc::new(Mutex::new(None::<Arc<RequestControl>>));
        let navigation_owner = navigation_request.clone();
        let navigation = view.page.protocol.subscribe_callback(
            &["Page.frameNavigated", "Page.frameDetached"],
            Arc::new(move |_| {
                if let Some(request) = navigation_owner.lock().unwrap().as_ref() {
                    request.cancel.cancel();
                    if let Some(panel) = request.panel.lock().unwrap().clone() {
                        panel.cancel();
                    }
                }
                true
            }),
        )?;
        let chooser = Arc::new(Self {
            app,
            view,
            automation,
            initial_directory,
            input_locked,
            visible: Arc::new(AtomicBool::new(false)),
            closed_flag: Arc::new(AtomicBool::new(false)),
            closed: CancellationToken::new(),
            request: navigation_request,
            done,
            _navigation: navigation,
        });
        native::protocol_call(&chooser.view, "Page.enable", json!({})).await?;
        native::protocol_call(
            &chooser.view,
            "Page.setInterceptFileChooserDialog",
            json!({"enabled":true}),
        )
        .await?;
        let worker = chooser.clone();
        tauri::async_runtime::spawn(async move {
            let result = worker.run().await;
            worker.done.send_replace(Some(result));
        });
        Ok(chooser)
    }

    fn permitted(&self) -> bool {
        self.visible.load(Ordering::Acquire)
            && !self.closed_flag.load(Ordering::Acquire)
            && !self.input_locked.load(Ordering::Acquire)
    }

    pub(crate) fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Release);
        if !visible {
            self.cancel();
        }
    }

    pub(crate) fn cancel(&self) {
        if let Some(request) = self.request.lock().unwrap().as_ref() {
            request.cancel.cancel();
            if let Some(panel) = request.panel.lock().unwrap().clone() {
                panel.cancel();
            }
        }
    }

    #[allow(dead_code)] // Native conformance fixture synchronization.
    pub(crate) async fn wait_for_panel(&self) -> Result<(), String> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let opened = self.request.lock().unwrap().as_ref().and_then(|request| {
                    request.panel.lock().unwrap().as_ref().map(|panel| {
                        panel.0.raw.load(Ordering::Acquire) != 0
                    })
                }).unwrap_or(false);
                if opened {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Browser file picker did not open".to_owned())
    }

    #[allow(dead_code)] // Native conformance fixture synchronization.
    pub(crate) async fn wait_for_idle(&self) -> Result<(), String> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if self.request.lock().unwrap().is_none() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Browser file picker did not settle".to_owned())
    }

    pub(crate) async fn close(&self) -> Result<(), String> {
        self.closed_flag.store(true, Ordering::Release);
        self.closed.cancel();
        self.cancel();
        let mut done = self.done.subscribe();
        loop {
            if let Some(result) = done.borrow().clone() {
                return match result {
                    // AppKit may destroy the CEF protocol before the backend
                    // receives Command-Q. A closed page proves no chooser can
                    // outlive this shutdown, so its terminal listener error is
                    // cleanup success. Live-page worker failures still escape.
                    Err(_) if self.view.page.protocol.is_closed() => Ok(()),
                    result => result,
                };
            }
            done.changed()
                .await
                .map_err(|_| "Browser file chooser worker completion was lost")?;
        }
    }

    async fn run(self: &Arc<Self>) -> Result<(), String> {
        loop {
            if self.closed.is_cancelled() {
                return Ok(());
            }
            let mut listener = match native::file_chooser::FileChooser::listen(&self.view).await {
                Ok(listener) => listener,
                // Command-Q can tear down the native protocol immediately
                // before Browser Resource shutdown reaches this worker. Once
                // close owns the cancellation token, a closed protocol is the
                // expected terminal state rather than a failed file chooser.
                Err(_) if self.closed.is_cancelled() || self.view.page.protocol.is_closed() => {
                    return Ok(())
                }
                Err(error) => return Err(error.to_string()),
            };
            listener.arm();
            let choice = match listener.next_or_idle(&self.closed).await {
                Ok(Some(choice)) => choice,
                Ok(None) => continue,
                Err(_) if self.closed.is_cancelled() || self.view.page.protocol.is_closed() => {
                    return Ok(())
                }
                Err(error) => return Err(error.to_string()),
            };
            if !self.permitted() {
                continue;
            }
            let request = Arc::new(RequestControl {
                cancel: self.closed.child_token(),
                panel: Mutex::new(None),
            });
            {
                let mut current = self.request.lock().unwrap();
                if current.is_some() {
                    request.cancel.cancel();
                    continue;
                }
                *current = Some(request.clone());
            }
            let result = self.run_request(&choice, &request).await;
            self.request.lock().unwrap().take();
            if let Err(error) = result {
                tracing::debug!(%error, "macOS user file selection ended without delivery");
            }
        }
    }

    async fn run_request(
        &self,
        choice: &Choice,
        request: &Arc<RequestControl>,
    ) -> Result<(), String> {
        let route = {
            let mut driver = tokio::select! {
                biased;
                _ = request.cancel.cancelled() => return Ok(()),
                driver = self.automation.lock() => driver,
            };
            if !self.permitted() {
                return Ok(());
            }
            driver.user_file_route(&self.view, choice).await?
        };
        let target = FileTarget::prepare(route, choice).await?;
        let outcome = async {
            if request.cancel.is_cancelled() || !self.permitted() {
                return Ok(());
            }
            let panel = NativeOpenPanel::start(
                self.app.clone(),
                self.initial_directory.clone(),
                choice.multiple,
            )?;
            *request.panel.lock().unwrap() = Some(panel.clone());
            let selected = tokio::select! {
                biased;
                _ = request.cancel.cancelled() => {
                    panel.cancel();
                    panel.finished().await?
                }
                result = panel.finished() => result?,
            };
            request.panel.lock().unwrap().take();
            let Some(paths) = selected else {
                return Ok(());
            };
            if request.cancel.is_cancelled() || !self.permitted() {
                return Ok(());
            }
            choice.require_current().map_err(|error| error.to_string())?;
            target.validate(paths.len()).await?;
            target
                .route
                .set_user_files(
                    &target.object,
                    &paths,
                    native::UserFileCommandGuard {
                        cancel: request.cancel.clone(),
                        locked: self.input_locked.clone(),
                        visible: self.visible.clone(),
                        closed: self.closed_flag.clone(),
                    },
                )
                .await
        }
        .await;
        target.release().await;
        outcome
    }
}

struct FileTarget {
    route: native::frames::OwnedFrameRoute,
    object: String,
    group: String,
}

impl FileTarget {
    async fn prepare(route: native::frames::OwnedFrameRoute, choice: &Choice) -> Result<Self, String> {
        let group = format!("nomifun-user-file-{}", uuid::Uuid::now_v7());
        let world = route
            .command(
                "Page.createIsolatedWorld",
                json!({"frameId":choice.frame,"worldName":"nomifun-user-file-picker","grantUniveralAccess":false}),
            )
            .await?;
        let context = world["executionContextId"]
            .as_i64()
            .ok_or("File chooser document is unavailable")?;
        let resolved = route
            .command(
                "DOM.resolveNode",
                json!({"backendNodeId":choice.backend_node,"executionContextId":context,"objectGroup":group}),
            )
            .await?;
        let object = resolved["object"]["objectId"]
            .as_str()
            .ok_or("File chooser input is unavailable")?
            .to_owned();
        let target = Self { route, object, group };
        if let Err(error) = target.validate(0).await {
            target.release().await;
            return Err(error);
        }
        Ok(target)
    }

    async fn validate(&self, count: usize) -> Result<(), String> {
        let result = self
            .route
            .command(
                "Runtime.callFunctionOn",
                json!({"objectId":self.object,"functionDeclaration":"function(count){return this instanceof HTMLInputElement && this.ownerDocument===document && this.type==='file' && !this.disabled && !this.webkitdirectory && (this.multiple||count<2);}","arguments":[{"value":count}],"returnByValue":true}),
            )
            .await?;
        if result["result"]["value"] == true {
            Ok(())
        } else {
            Err("File chooser input changed".into())
        }
    }

    async fn release(&self) {
        let _ = self
            .route
            .command(
                "Runtime.releaseObjectGroup",
                json!({"objectGroup":self.group}),
            )
            .await;
    }
}
