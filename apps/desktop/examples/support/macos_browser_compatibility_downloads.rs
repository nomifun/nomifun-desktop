//! Real native destination picker with no configured automatic Downloads path.
//! DesktopBrowserHost normally configures the user's Downloads directory. This
//! isolated Page tests its existing picker branch without selecting any file.
use super::fixture::Fixture;
use nomifun_browser_macos::engine::{Engine, Page, ParentView};
use nomifun_browser_platform::runtime::*;
use objc2::msg_send;
use serde_json::{Value, json};
use std::{sync::Arc, time::{Duration, Instant}};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

pub(crate) async fn verify(app: &tauri::AppHandle, fixture: &Fixture) -> Result<Value, String> {
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE first_attachment_destination_picker");
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || { let _ = tx.send(Engine::initialize()); }).map_err(|error| error.to_string())?;
    let engine = rx.await.map_err(|_| "Native attachment engine dispatch was lost")??;
    let context = match engine.create_context(None).await {
        Ok(context) => context,
        Err(error) => { let _ = engine.shutdown().await; return Err(error); }
    };
    let handle = app.clone();
    let parent: Arc<ParentView> = Arc::new(move || {
        let window = handle.get_window("main").ok_or("Missing disposable attachment window")?;
        let raw = window.ns_window().map_err(|_| "Missing attachment NSWindow")?;
        let window = unsafe { raw.cast::<objc2_app_kit::NSWindow>().as_ref() }.ok_or("Null attachment NSWindow")?;
        window.contentView().ok_or_else(|| "Missing attachment content view".into())
    });
    let mut pages = Vec::new();
    let result = async {
        for cancellation in ["native_cancel_download", "actual_panel_hide", "input_ownership_switch"] {
            let page = engine.create_page(parent.clone(), context.clone()).await?;
            pages.push(page.clone());
            page.set_input_locked(false).await?;
            // Deliberately do not configure_user_downloads: that branch would
            // automatically choose a real directory rather than a save panel.
            page.set_surface(bounds(), true, CancellationToken::new()).await?;
            page.navigate(fixture.url("/attachment")).await?;
            let download = choosing(&page).await?;
            wait_save_panel(app, true).await?;
            if page.snapshot().load.content_state != BrowserContentState::None {
                return Err("Attachment unexpectedly committed a website document".into());
            }
            if !download.can_cancel || download.filename != "compatibility-fixture.txt" {
                return Err("Attachment destination metadata was not available to the user".into());
            }
            // A metadata refresh/content mask must not dismiss this user-owned
            // save panel even though the WK document remains hidden.
            tokio::time::sleep(Duration::from_millis(300)).await;
            wait_save_panel(app, true).await?;
            match cancellation {
                "native_cancel_download" => page.cancel_user_download(download.id.clone()).await?,
                "actual_panel_hide" => page.set_surface(bounds(), false, CancellationToken::new()).await?,
                _ => page.set_input_locked(true).await?,
            }
            wait_cancelled(&page, &download.id).await?;
            wait_save_panel(app, false).await?;
            page.force_close().await?;
        }
        Ok(json!({"scope":"native_page_destination_picker","ephemeral_profile":true,
            "first_attachment_without_html_commit":true,"content_mask_preserves_choosing_panel":true,
            "native_cancel_download":true,"actual_panel_hide_cancels_picker":true,
            "input_ownership_switch_cancels_picker":true,"file_destination_selected":false,
            "user_downloads_directory_used":false,"completed_file_download":"not_covered"}))
    }.await;
    let mut cleanup_error = None;
    for page in pages { if let Err(error) = page.force_close().await { cleanup_error = Some(error); } }
    if let Err(error) = context.close().await { cleanup_error = Some(error); }
    if let Err(error) = engine.shutdown().await { cleanup_error = Some(error); }
    match (result, cleanup_error) {
        (Ok(report), None) => Ok(report), (Err(error), None) => Err(error),
        (Ok(_), Some(error)) => Err(format!("Attachment cleanup failed: {error}")),
        (Err(error), Some(cleanup)) => Err(format!("{error}; attachment cleanup: {cleanup}")),
    }
}

fn bounds() -> BrowserSurfaceBounds { BrowserSurfaceBounds {x:20.0,y:40.0,width:840.0,height:540.0} }

async fn choosing(page: &Arc<Page>) -> Result<BrowserDownloadSnapshot, String> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let downloads = page.user_download_snapshot();
        if let Some(download) = downloads.iter().find(|download| download.state == BrowserDownloadState::Choosing
            && download.filename == "compatibility-fixture.txt") { return Ok(download.clone()); }
        if downloads.iter().any(|download| matches!(download.state, BrowserDownloadState::Cancelled | BrowserDownloadState::Failed | BrowserDownloadState::Completed)) {
            return Err("First attachment was rejected or selected a destination before the save panel could be observed".into());
        }
        if Instant::now() >= deadline {
            let snapshot = page.snapshot();
            return Err(format!("First attachment picker did not appear; phase={:?}, content={:?}, downloads={downloads:?}", snapshot.load.phase, snapshot.load.content_state));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn wait_cancelled(page: &Arc<Page>, id: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if page.user_download_snapshot().iter().any(|download| download.id == id && download.state == BrowserDownloadState::Cancelled) { return Ok(()); }
        if Instant::now() >= deadline { return Err("Attachment picker cancellation did not settle to Cancelled".into()); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn save_panel_visible(app: &tauri::AppHandle) -> Result<bool, String> {
    let handle = app.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<bool, String> {
            let window = handle.get_window("main").ok_or("Missing attachment window")?;
            let raw = window.ns_window().map_err(|error| error.to_string())?;
            let window = unsafe { raw.cast::<objc2_app_kit::NSWindow>().as_ref() }.ok_or("Null attachment window")?;
            let Some(sheet) = window.attachedSheet() else { return Ok(false); };
            let class = objc2::runtime::AnyClass::get(c"NSSavePanel").ok_or("Missing NSSavePanel class")?;
            let save: bool = unsafe { msg_send![&*sheet, isKindOfClass: class] };
            Ok(save && sheet.isVisible())
        })();
        let _ = tx.send(result);
    }).map_err(|error| error.to_string())?;
    rx.await.map_err(|_| "Attachment panel inspection dispatch was lost")?
}
async fn wait_save_panel(app: &tauri::AppHandle, visible: bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if save_panel_visible(app).await? == visible { return Ok(()); }
        if Instant::now() >= deadline { return Err(format!("Native save panel did not become visible={visible}")); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
