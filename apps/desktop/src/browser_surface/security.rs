//! A window label is not an IPC principal: a window may contain external pages.

pub(crate) fn is_app_webview(webview: &str, window: &str) -> bool {
    webview == window
        && (webview == "main"
            || webview
                .strip_prefix("companion-")
                .is_some_and(|id| !id.is_empty()))
}

/// Custom application commands are allowed by Tauri unless explicitly scoped.
/// Check the host-owned WebView identity before dispatching any application command.
pub(crate) fn app_commands_only<R: tauri::Runtime>(
    dispatch: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let view = invoke.message.webview_ref();
        if !is_app_webview(view.label(), view.window().label()) {
            invoke
                .resolver
                .reject("This page cannot invoke desktop commands.");
            return true;
        }
        dispatch(invoke)
    }
}

#[cfg(test)]
mod tests {
    use super::is_app_webview;

    #[test]
    fn external_children_do_not_inherit_the_parent_app_identity() {
        assert!(is_app_webview("main", "main"));
        assert!(is_app_webview("companion-123", "companion-123"));
        assert!(!is_app_webview("browser-123", "main"));
        assert!(!is_app_webview("browser-123", "companion-123"));
        assert!(!is_app_webview("main", "browser-popout"));
        assert!(!is_app_webview("companion-", "companion-"));
    }

    #[test]
    fn plugin_capabilities_scope_webviews_not_parent_windows() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json")).unwrap();
        // Tauri ORs window and WebView patterns; keeping a window pattern leaks
        // permissions to a browser child even when WebView patterns are present.
        assert!(capability.get("windows").is_none());
        assert_eq!(
            capability["webviews"],
            serde_json::json!(["main", "companion-*"])
        );
        assert!(capability.get("remote").is_none());
    }
}
