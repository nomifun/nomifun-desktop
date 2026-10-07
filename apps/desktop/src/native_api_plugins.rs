//! Keep native application APIs without replacing standard APIs in websites.
//!
//! The upstream initializer injects an async confirm shim into every WebView,
//! even those denied plugin IPC. That changes synchronous website semantics and
//! bypasses the native browser's ScriptDialogOpening handling. No browser-page
//! restoration script or upstream fork is needed: omit the global initializer.
use tauri::{
    AppHandle, Runtime, Webview, Window,
    plugin::{Plugin, TauriPlugin},
};

pub(crate) struct NativeApiPlugin<R: Runtime> {
    inner: TauriPlugin<R>,
    script: Option<String>,
}
pub(crate) fn dialog<R: Runtime>() -> NativeApiPlugin<R> {
    NativeApiPlugin {
        inner: tauri_plugin_dialog::init(),
        script: None,
    }
}
pub(crate) fn notification<R: Runtime>() -> NativeApiPlugin<R> {
    let inner = tauri_plugin_notification::init();
    // The official notification JS API uses the plugin's Notification shim.
    // Retain it only in first-party top-level documents, never in browser pages
    // or their subframes. This is not an IPC authorization check; ACLs remain.
    let script=inner.initialization_script().map(|script|format!(r#"
if (window === window.top) {{
  const metadata = window.__TAURI_INTERNALS__?.metadata;
  const view = metadata?.currentWebview?.label;
  const owner = metadata?.currentWindow?.label;
  if (view === owner && (view === 'main' || (typeof view === 'string' && view.startsWith('companion-') && view.length > 10))) {{
    {script}
  }}
}}
"#));
    NativeApiPlugin { inner, script }
}
impl<R: Runtime> Plugin<R> for NativeApiPlugin<R> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn initialize(
        &mut self,
        app: &AppHandle<R>,
        config: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.inner.initialize(app, config)
    }
    fn initialization_script(&self) -> Option<String> {
        self.script.clone()
    }
    fn window_created(&mut self, window: Window<R>) {
        self.inner.window_created(window);
    }
    fn webview_created(&mut self, webview: Webview<R>) {
        self.inner.webview_created(webview);
    }
    fn on_navigation(&mut self, webview: &Webview<R>, url: &url::Url) -> bool {
        self.inner.on_navigation(webview, url)
    }
    fn on_page_load(
        &mut self,
        webview: &Webview<R>,
        payload: &tauri::webview::PageLoadPayload<'_>,
    ) {
        self.inner.on_page_load(webview, payload);
    }
    fn on_event(&mut self, app: &AppHandle<R>, event: &tauri::RunEvent) {
        self.inner.on_event(app, event);
    }
    fn extend_api(&mut self, invoke: tauri::ipc::Invoke<R>) -> bool {
        self.inner.extend_api(invoke)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_dialog_plugin_does_not_inject_website_overrides() {
        let plugin = dialog::<tauri::Wry>();
        assert_eq!(plugin.name(), "dialog");
        assert!(plugin.initialization_script().is_none());
        assert!(plugin.initialization_script_2().is_none());
    }
    #[test]
    fn notification_initialization_is_confined_to_first_party_documents() {
        let plugin = notification::<tauri::Wry>();
        assert_eq!(plugin.name(), "notification");
        let script = plugin.initialization_script().unwrap();
        assert!(script.contains("window === window.top"));
        assert!(script.contains("view === owner"));
        assert!(script.contains("view === 'main'"));
        assert!(script.contains("view.startsWith('companion-')"));
    }
}
