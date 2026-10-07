//! Explicit user handoff, not another Agent-controlled browser or cookie copy.
use nomifun_browser_platform::runtime::{BrowserTabSnapshot, BrowserTabTarget, WorkspaceError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;
use tauri::Manager;
use windows::{
    Win32::UI::{
        Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_UNICODE, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
    core::{HSTRING, PCWSTR, PWSTR},
};

fn web_url(value: &str) -> Result<url::Url, WorkspaceError> {
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return Err(WorkspaceError::InvalidUrl);
    }
    let url = url::Url::parse(value).map_err(|_| WorkspaceError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(WorkspaceError::InvalidUrl);
    }
    Ok(url)
}

/// Resolve the OS-configured folder, not a page URL or a renderer-supplied path.
/// An acknowledged handoff opens a folder only; it never executes a download.
pub(crate) async fn open_downloads(
    app: &tauri::AppHandle,
    locked: Arc<AtomicBool>,
    cancel: CancellationToken,
    closing: CancellationToken,
) -> Result<(), WorkspaceError> {
    let path = app.path().download_dir().map_err(|_| WorkspaceError::NativeCommandFailed)?;
    if !path.is_absolute() || !path.is_dir() { return Err(WorkspaceError::NativeCommandFailed); }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = if locked.load(Ordering::Acquire) || cancel.is_cancelled() || closing.is_cancelled() {
            Err(WorkspaceError::NotActionable)
        } else {
            let verb = HSTRING::from("explore");
            let file = HSTRING::from(path.as_os_str());
            let mut request = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_UNICODE,
                lpVerb: PCWSTR(verb.as_ptr()),
                lpFile: PCWSTR(file.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            unsafe { ShellExecuteExW(&mut request) }.map_err(|_| WorkspaceError::NativeCommandFailed)
        };
        let _ = tx.send(result);
    }).map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}

pub(crate) async fn open(
    view: &tauri::Webview,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    target: BrowserTabTarget,
    locked: Arc<AtomicBool>,
    cancel: CancellationToken,
    closing: CancellationToken,
) -> Result<(), WorkspaceError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| {
            let current = || {
                !locked.load(Ordering::Acquire)
                    && !cancel.is_cancelled()
                    && !closing.is_cancelled()
                    && metadata.lock().unwrap_or_else(|e| e.into_inner()).target == target
            };
            if !current() {
                return Err(WorkspaceError::StaleTarget);
            }
            let controller = platform.controller();
            let core = unsafe { controller.CoreWebView2() }
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            let mut source = PWSTR::null();
            unsafe { core.Source(&mut source) }.map_err(|_| WorkspaceError::NativeCommandFailed)?;
            let source = super::event_string(source, 8192).ok_or(WorkspaceError::InvalidUrl)?;
            let url = web_url(&source)?;
            let mut parent = windows::Win32::Foundation::HWND::default();
            unsafe { controller.ParentWindow(&mut parent) }
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            if !current() {
                return Err(WorkspaceError::StaleTarget);
            }
            let verb = HSTRING::from("open");
            let file = HSTRING::from(url.as_str());
            let mut request = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_UNICODE,
                hwnd: parent,
                lpVerb: PCWSTR(verb.as_ptr()),
                lpFile: PCWSTR(file.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            // Native STA dispatch, with no command shell, environment expansion,
            // zone-check bypass, or detached application-owned worker. Success is
            // OS handoff acknowledgement, not proof the external page has loaded.
            unsafe { ShellExecuteExW(&mut request) }
                .map_err(|_| WorkspaceError::NativeCommandFailed)
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_web_urls_can_reach_the_system_handler() {
        for value in [
            "about:blank",
            "file:///C:/Windows/notepad.exe",
            "javascript:alert(1)",
            "data:text/html,test",
            "mailto:a@example.com",
            "nomifun://settings",
            "https://u:p@example.com/",
            "https://example.com/\n",
            "C:\\Windows\\notepad.exe",
            "\\\\server\\share",
        ] {
            assert!(web_url(value).is_err(), "{value:?}");
        }
        assert!(web_url(&format!("https://example.com/{}", "a".repeat(8192))).is_err());
    }
    #[test]
    fn current_page_query_fragment_and_unicode_are_preserved_without_shell_syntax() {
        let url = web_url("https://example.com/中文?state=abc&next=%2Fapp#return").unwrap();
        assert_eq!(url.query(), Some("state=abc&next=%2Fapp"));
        assert_eq!(url.fragment(), Some("return"));
        assert!(url.as_str().contains("%E4%B8%AD"));
        assert!(
            !web_url("http://localhost:3000/\" --flag")
                .unwrap()
                .as_str()
                .contains('"')
        );
    }
}
