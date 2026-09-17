//! Main-window Surface placement and change stream. No Agent/run authority here.

use nomifun_browser_platform::{
    runtime::BrowserSurfaceBounds,
    workspace::{BrowserResource, BrowserResourceSnapshot},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tauri::ipc::Channel;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(crate) struct BrowserSurfaceState {
    next: AtomicU64,
    current: Arc<Mutex<Option<Attachment>>>,
}

struct Attachment {
    id: u64,
    sequence: u64,
    resource: Arc<BrowserResource>,
    bounds: BrowserSurfaceBounds,
    stop: CancellationToken,
    layout: CancellationToken,
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum BrowserViewEvent {
    Snapshot { snapshot: BrowserResourceSnapshot },
    Unavailable { code: &'static str },
}

fn require_main(view: &tauri::Webview) -> Result<(), String> {
    if view.label() == "main" && view.window().label() == "main" {
        Ok(())
    } else {
        Err("Browser surfaces belong to the main application view.".into())
    }
}

fn validate_bounds(view: &tauri::Webview, bounds: BrowserSurfaceBounds) -> Result<(), String> {
    let window = view.window();
    let scale = window
        .scale_factor()
        .map_err(|_| "Window scale is unavailable.")?;
    let size = window
        .inner_size()
        .map_err(|_| "Window size is unavailable.")?
        .to_logical::<f64>(scale);
    if bounds.is_valid()
        && bounds.x + bounds.width <= size.width + 1.0
        && bounds.y + bounds.height <= size.height + 1.0
    {
        Ok(())
    } else {
        Err("Browser surface is outside the application window.".into())
    }
}

async fn detach(current: &Mutex<Option<Attachment>>, id: u64) -> Result<(), String> {
    let mut current = current.lock().await;
    if let Some(attachment) = current.as_mut().filter(|a| a.id == id) {
        attachment.stop.cancel();
        attachment.layout.cancel();
        attachment.layout = CancellationToken::new();
        // Hide never waits for a page operation. Retain ownership on failure
        // so a retry cannot orphan a visible native surface.
        attachment
            .resource
            .set_surface(attachment.bounds, false, attachment.layout.clone())
            .await
            .map_err(|error| error.to_string())?;
        current.take();
    }
    Ok(())
}

async fn update(
    current: &Mutex<Option<Attachment>>,
    id: u64,
    sequence: u64,
    bounds: BrowserSurfaceBounds,
    visible: bool,
    validation: Result<(), String>,
) -> Result<(), String> {
    let (resource, bounds, visible, layout) = {
        let mut current = current.lock().await;
        let Some(attachment) = current
            .as_mut()
            .filter(|a| a.id == id && !a.stop.is_cancelled())
        else {
            return Ok(());
        };
        if sequence <= attachment.sequence {
            return Ok(());
        }
        attachment.sequence = sequence;
        attachment.layout.cancel();
        attachment.layout = CancellationToken::new();
        let visible = visible && validation.is_ok();
        if bounds.is_valid() && validation.is_ok() {
            attachment.bounds = bounds;
        }
        (
            attachment.resource.clone(),
            attachment.bounds,
            visible,
            attachment.layout.clone(),
        )
    };
    // No attachment mutex across a possibly input-blocked visible resize.
    let result = resource
        .set_surface(bounds, visible, layout.clone())
        .await
        .map_err(|error| error.to_string());
    if layout.is_cancelled() {
        return Ok(());
    }
    result?;
    validation
}

#[tauri::command]
pub(crate) async fn browser_surface_attach(
    view: tauri::Webview,
    server: tauri::State<'_, Arc<nomifun_app::DesktopServer>>,
    state: tauri::State<'_, BrowserSurfaceState>,
    conversation_id: String,
    bounds: BrowserSurfaceBounds,
    events: Channel<BrowserViewEvent>,
) -> Result<u64, String> {
    require_main(&view)?;
    validate_bounds(&view, bounds)?;
    let id = state.next.fetch_add(1, Ordering::AcqRel) + 1;
    let resource = server
        .browser_resource_for_local_surface(&conversation_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut current = state.current.lock().await;
    if state.next.load(Ordering::Acquire) != id {
        return Err("Browser attachment was superseded.".into());
    }
    if let Some(previous) = current.as_mut() {
        previous.stop.cancel();
        previous.layout.cancel();
        previous.layout = CancellationToken::new();
        previous
            .resource
            .set_surface(previous.bounds, false, previous.layout.clone())
            .await
            .map_err(|error| error.to_string())?;
    }
    let mut run = resource.run_changes();
    let mut page = resource
        .runtime_changes()
        .await
        .map_err(|error| error.to_string())?;
    if state.next.load(Ordering::Acquire) != id {
        return Err("Browser attachment was superseded.".into());
    }
    let stop = CancellationToken::new();
    *current = Some(Attachment {
        id,
        sequence: 0,
        resource: resource.clone(),
        bounds,
        stop: stop.clone(),
        layout: CancellationToken::new(),
    });
    drop(current);
    // Attach is hidden. The renderer's first measured update decides visibility,
    // including overlays or disposal which happened while attach was pending.
    let current = state.current.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if stop.is_cancelled() {
                break;
            }
            run.borrow_and_update();
            page.borrow_and_update();
            let snapshot = resource.snapshot().await;
            if stop.is_cancelled() {
                break;
            }
            match snapshot {
                Ok(snapshot) => {
                    if events
                        .send(BrowserViewEvent::Snapshot { snapshot })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = events.send(BrowserViewEvent::Unavailable { code: error.code() });
                    break;
                }
            }
            tokio::select! {
                biased;
                _=stop.cancelled()=>break,
                result=run.changed()=>if result.is_err(){break},
                result=page.changed()=>if result.is_err(){break},
            }
        }
        let _ = detach(&current, id).await;
    });
    Ok(id)
}

#[tauri::command]
pub(crate) async fn browser_surface_update(
    view: tauri::Webview,
    state: tauri::State<'_, BrowserSurfaceState>,
    attachment_id: u64,
    sequence: u64,
    bounds: BrowserSurfaceBounds,
    visible: bool,
) -> Result<(), String> {
    require_main(&view)?;
    let validation = if visible {
        validate_bounds(&view, bounds)
    } else {
        Ok(())
    };
    update(
        &state.current,
        attachment_id,
        sequence,
        bounds,
        visible,
        validation,
    )
    .await
}

#[tauri::command]
pub(crate) async fn browser_surface_detach(
    view: tauri::Webview,
    state: tauri::State<'_, BrowserSurfaceState>,
    attachment_id: u64,
) -> Result<(), String> {
    require_main(&view)?;
    detach(&state.current, attachment_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nomifun_browser_platform::{
        run_guard::{NativeInputGate, RunAdmissionError},
        runtime::*,
        product::{
            BrowserCapabilityAction, BrowserProviderDescriptor, BrowserResourceBinding,
            BrowserSessionAuthority,
        },
        workspace::BrowserResourceService,
    };
    use std::sync::atomic::AtomicBool;

    #[derive(Default)]
    struct Surface {
        visible: AtomicBool,
        fail_close: AtomicBool,
        fail_hide: AtomicBool,
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    struct Factory(Arc<Surface>);
    #[async_trait]
    impl BrowserRuntimeFactory for Factory {
        async fn create(
            &self,
            _: CreateBrowserRuntime,
        ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
            Ok(self.0.clone())
        }
    }
    #[async_trait]
    impl NativeInputGate for Surface {
        async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
            Ok(())
        }
        async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
            Ok(())
        }
        async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
            Ok(())
        }
    }
    #[async_trait]
    impl BrowserNativeSurfacePort for Surface {
        async fn set_surface(
            &self,
            _: BrowserSurfaceBounds,
            visible: bool,
            cancel: CancellationToken,
        ) -> Result<(), WorkspaceError> {
            if !visible && self.fail_hide.load(Ordering::SeqCst) {return Err(WorkspaceError::NativeCommandFailed);}
            if visible {
                self.started.notify_one();
                tokio::select! {biased; _=cancel.cancelled()=>return Ok(()),_=self.release.notified()=>{}}
            }
            if !cancel.is_cancelled() {
                self.visible.store(visible, Ordering::SeqCst);
            }
            Ok(())
        }
    }
    #[async_trait]
    impl BrowserRuntime for Surface {
        fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> {
            Some(self)
        }
        async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
            Ok(BrowserRuntimeSnapshot {
                downloads: vec![],
                runtime_generation: 1,
                revision: 1,
                active_tab_id: None,
                tabs: vec![],
            })
        }
        async fn execute(
            &self,
            _: BrowserTabCommand,
            _: CancellationToken,
        ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
            Err(WorkspaceError::UnsupportedAction)
        }
        async fn close(&self) -> Result<(), WorkspaceError> {
            if self.fail_close.load(Ordering::SeqCst) {return Err(WorkspaceError::NativeCommandFailed);}
            self.visible.store(false,Ordering::SeqCst);
            Ok(())
        }
    }

    async fn fixture() -> (
        Arc<Mutex<Option<Attachment>>>,
        Arc<Surface>,
        BrowserSurfaceBounds,
    ) {
        let surface = Arc::new(Surface::default());
        let service = BrowserResourceService::new(Arc::new(Factory(surface.clone())));
        let authority = BrowserSessionAuthority::new(
            "surface-test",
            "surface-test",
            BrowserCapabilityAction::all(),
            BrowserResourceBinding::new(
                "surface-test",
                "managed-browser",
                "surface-test",
                BrowserProviderDescriptor::managed("managed", "native-test").unwrap(),
                BrowserCapabilityAction::all()
                    .map(BrowserCapabilityAction::resource_operation),
            )
            .unwrap(),
        )
        .unwrap();
        let resource = service
            .ensure(
                authority,
                BrowserProfile::Ephemeral,
            )
            .await
            .unwrap();
        let bounds = BrowserSurfaceBounds {
            x: 0.0,
            y: 0.0,
            width: 900.0,
            height: 600.0,
        };
        let current = Arc::new(Mutex::new(Some(Attachment {
            id: 1,
            sequence: 0,
            resource,
            bounds,
            stop: CancellationToken::new(),
            layout: CancellationToken::new(),
        })));
        (current, surface, bounds)
    }

    #[tokio::test]
    async fn hide_preempts_waiting_resize_and_stale_sequence_cannot_show() {
        let (current, surface, bounds) = fixture().await;
        let resize = {
            let current = current.clone();
            tokio::spawn(async move { update(&current, 1, 1, bounds, true, Ok(())).await })
        };
        surface.started.notified().await;
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            update(&current, 1, 2, bounds, false, Ok(())),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(200), resize)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        update(&current, 1, 1, bounds, true, Ok(())).await.unwrap();
        surface.release.notify_waiters();
        assert!(!surface.visible.load(Ordering::SeqCst));
        assert_eq!(current.lock().await.as_ref().unwrap().sequence, 2);
    }

    #[tokio::test]
    async fn detach_retains_failed_hide_but_releases_a_closed_runtime() {
        let (current,surface,bounds)=fixture().await;
        surface.release.notify_one();
        update(&current,1,1,bounds,true,Ok(())).await.unwrap();
        let resource=current.lock().await.as_ref().unwrap().resource.clone();
        surface.fail_close.store(true,Ordering::SeqCst);
        surface.fail_hide.store(true,Ordering::SeqCst);
        assert_eq!(resource.close().await,Err(WorkspaceError::NativeCommandFailed));
        assert!(detach(&current,1).await.is_err());
        assert!(current.lock().await.is_some());
        surface.fail_close.store(false,Ordering::SeqCst);
        resource.close().await.unwrap();
        detach(&current,1).await.unwrap();
        assert!(current.lock().await.is_none());
        assert!(!surface.visible.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn failed_close_can_detach_after_hiding_the_remaining_surface() {
        let (current,surface,bounds)=fixture().await;
        surface.release.notify_one();
        update(&current,1,1,bounds,true,Ok(())).await.unwrap();
        let resource=current.lock().await.as_ref().unwrap().resource.clone();
        surface.fail_close.store(true,Ordering::SeqCst);
        assert!(resource.close().await.is_err());
        detach(&current,1).await.unwrap();
        assert!(current.lock().await.is_none());
        assert!(!surface.visible.load(Ordering::SeqCst));
        surface.fail_close.store(false,Ordering::SeqCst);
        resource.close().await.unwrap();
    }

    #[tokio::test]
    async fn detach_cancels_resize_without_waiting_for_page_input() {
        let (current, surface, bounds) = fixture().await;
        let resize = {
            let current = current.clone();
            tokio::spawn(async move { update(&current, 1, 1, bounds, true, Ok(())).await })
        };
        surface.started.notified().await;
        tokio::time::timeout(std::time::Duration::from_millis(200), detach(&current, 1))
            .await
            .unwrap()
            .unwrap();
        resize.await.unwrap().unwrap();
        assert!(current.lock().await.is_none());
        update(&current, 1, 99, bounds, true, Ok(())).await.unwrap();
        assert!(!surface.visible.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn invalid_bounds_hide_with_last_valid_bounds_and_end_old_update() {
        let (current, surface, bounds) = fixture().await;
        let resize = {
            let current = current.clone();
            tokio::spawn(async move { update(&current, 1, 1, bounds, true, Ok(())).await })
        };
        surface.started.notified().await;
        let invalid = BrowserSurfaceBounds {
            width: f64::NAN,
            ..bounds
        };
        assert!(
            update(&current, 1, 2, invalid, true, Err("outside window".into()))
                .await
                .is_err()
        );
        resize.await.unwrap().unwrap();
        assert_eq!(current.lock().await.as_ref().unwrap().bounds.width, 900.0);
        assert!(!surface.visible.load(Ordering::SeqCst));
    }
}
