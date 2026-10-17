//! WebKitGTK owns Linux renderer capture. Its unhandled UserMedia requests
//! are denied, so the main app must supply the native permission decision.
//! This is a per-request WebKit decision, not an OS/device grant inventory.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use tauri::Manager;
use url::Url;
use webkit2gtk::{LoadEvent, PermissionRequestExt, SettingsExt, UserMediaPermissionRequest,
    UserMediaPermissionRequestExt, WebViewExt};

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppOrigin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl AppOrigin {
    fn from_url(url: &Url) -> Option<Self> {
        if !url.username().is_empty() || url.password().is_some() { return None; }
        Some(Self { scheme: url.scheme().to_owned(), host: url.host_str()?.to_owned(),
            port: url.port_or_known_default() })
    }

    fn matches(&self, uri: &str) -> bool {
        Url::parse(uri).ok().as_ref().and_then(Self::from_url).as_ref() == Some(self)
    }

    fn context(&self) -> String {
        match self.port {
            Some(port) => format!("{}://{}:{}", self.scheme, self.host, port),
            None => format!("{}://{}", self.scheme, self.host),
        }
    }
}

fn app_origin(window: &tauri::WebviewWindow) -> Option<AppOrigin> {
    if tauri::is_dev() {
        // Match the configured local dev document, never a page reached by
        // navigation. An explicitly remote dev server gets no media prompt.
        let configured = window.config().build.dev_url.as_ref()?;
        let local = configured.host_str().is_some_and(|host| host == "localhost"
            || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()));
        if !local || !matches!(configured.scheme(), "http" | "https") { return None; }
        AppOrigin::from_url(configured)
    } else {
        AppOrigin::from_url(&Url::parse("tauri://localhost").expect("static app URL"))
    }
}

/// Install only on the main app WebView. Browser surfaces and WebUI clients
/// retain their own website/browser permission mechanisms.
pub(crate) fn install(window: &tauri::WebviewWindow) -> anyhow::Result<()> {
    let origin = app_origin(window);
    window.with_webview(move |platform| {
        let view = platform.inner();
        if let Some(settings) = WebViewExt::settings(&view) {
            // WebKit defaults vary by release. getUserMedia/MediaRecorder do
            // not need RTCPeerConnection, so leave enable-webrtc unchanged.
            settings.set_enable_media_stream(true);
        }
        let parent = view.toplevel().and_then(|widget| widget.downcast::<gtk::Window>().ok());
        let parent = parent.map(|window| window.downgrade());
        let generation = Rc::new(Cell::new(0_u64));
        let dialogs = Rc::new(RefCell::new(Vec::<gtk::glib::WeakRef<gtk::MessageDialog>>::new()));
        let load_generation = generation.clone();
        let load_dialogs = dialogs.clone();
        view.connect_load_changed(move |_, event| {
            if event == LoadEvent::Started {
                load_generation.set(load_generation.get().wrapping_add(1));
                // A response to an old document must not authorize a new one.
                for dialog in load_dialogs.borrow_mut().drain(..) {
                    if let Some(dialog) = dialog.upgrade() { dialog.response(gtk::ResponseType::Cancel); }
                }
            }
        });
        view.connect_permission_request(move |view, request| {
            let Some(media) = request.downcast_ref::<UserMediaPermissionRequest>() else { return false; };
            let devices = match (media.is_for_audio_device(), media.is_for_video_device()) {
                (true, true) => "microphone and camera",
                (true, false) => "microphone",
                (false, true) => "camera",
                (false, false) => { request.deny(); return true; }
            };
            let Some(origin) = origin.as_ref() else { request.deny(); return true; };
            if !view.uri().is_some_and(|uri| origin.matches(&uri)) {
                request.deny(); return true;
            }
            let Some(parent) = parent.as_ref().and_then(|parent| parent.upgrade()) else {
                request.deny(); return true;
            };
            // WebKitGTK's request exposes device kinds but no requesting-frame
            // origin. Display the actual top-page context, never claim it is
            // the request's origin, and never silently allow a media request.
            let dialog = gtk::MessageDialog::builder()
                .transient_for(&parent).modal(true).destroy_with_parent(true)
                .message_type(gtk::MessageType::Question)
                .text(format!("Allow access to the {devices}?"))
                .secondary_text(format!("Current NomiFun page: {}\n\nAllow only if you initiated this request in NomiFun. This decision applies to this request.", origin.context()))
                .build();
            dialog.add_button("Cancel", gtk::ResponseType::Cancel);
            dialog.add_button("Allow", gtk::ResponseType::Accept);
            dialog.set_default_response(gtk::ResponseType::Cancel);
            let request = request.clone();
            let expected = origin.clone();
            let expected_generation = generation.get();
            let response_generation = generation.clone();
            let weak_view = view.downgrade();
            dialog.connect_response(move |dialog, response| {
                let current = weak_view.upgrade().is_some_and(|view|
                    view.uri().is_some_and(|uri| expected.matches(&uri)));
                if response == gtk::ResponseType::Accept && current
                    && response_generation.get() == expected_generation {
                    request.allow();
                } else { request.deny(); }
                // Destroy directly: Window::close would emit another dialog response.
                unsafe { dialog.destroy(); }
            });
            dialogs.borrow_mut().retain(|dialog| dialog.upgrade().is_some());
            dialogs.borrow_mut().push(dialog.downgrade());
            dialog.show();
            true
        });
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_media_context_requires_the_exact_configured_origin() {
        let origin = AppOrigin::from_url(&Url::parse("http://localhost:5173/index.html").unwrap()).unwrap();
        assert!(origin.matches("http://localhost:5173/settings?tab=voice-input"));
        for uri in ["https://localhost:5173", "http://localhost:5174", "http://127.0.0.1:5173",
            "http://localhost.example:5173", "http://user@localhost:5173", "about:blank"] {
            assert!(!origin.matches(uri), "{uri}");
        }
        let bundled = AppOrigin::from_url(&Url::parse("tauri://localhost/index.html").unwrap()).unwrap();
        assert!(bundled.matches("tauri://localhost/settings"));
        assert!(!bundled.matches("https://tauri.localhost/settings"));
    }
}
