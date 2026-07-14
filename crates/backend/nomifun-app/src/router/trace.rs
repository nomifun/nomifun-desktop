//! HTTP request access-log layer.

use std::borrow::Cow;

use axum::Router;
use nomifun_api_types::is_preview_capability;
use tower_http::trace::TraceLayer;

const REDACTED_CAPABILITY: &str = "[REDACTED]";

/// Capability URLs are bearer credentials. Preserve the exact route shape and
/// asset suffix for diagnostics, but never put the credential in an access-log
/// span. Similar prefixes, legacy numeric ports, and malformed tokens are not
/// treated as capability routes.
fn access_log_path(path: &str) -> Cow<'_, str> {
    let Some(path_without_root) = path.strip_prefix('/') else {
        return Cow::Borrowed(path);
    };
    let mut segments = path_without_root.splitn(4, '/');
    let (Some("api"), Some(prefix), Some(capability)) =
        (segments.next(), segments.next(), segments.next())
    else {
        return Cow::Borrowed(path);
    };
    if !matches!(prefix, "ppt-proxy" | "office-watch-proxy")
        || !is_preview_capability(capability)
    {
        return Cow::Borrowed(path);
    }

    match segments.next() {
        Some(suffix) => Cow::Owned(format!(
            "/api/{prefix}/{REDACTED_CAPABILITY}/{suffix}"
        )),
        None => Cow::Owned(format!("/api/{prefix}/{REDACTED_CAPABILITY}")),
    }
}

/// Returns true if a request path is high-frequency terminal I/O whose access
/// log would flood the console (per-keystroke input, per-frame resize). These
/// are logged at TRACE so they are silent at the normal dev `debug` level, while
/// lifecycle routes (create/get/kill/relaunch/list) keep INFO logging.
fn is_noisy_terminal_path(path: &str) -> bool {
    path.starts_with("/api/terminals/") && (path.ends_with("/input") || path.ends_with("/resize"))
}

pub(super) fn with_access_log(router: Router) -> Router {
    router.layer(
        TraceLayer::new_for_http()
            .make_span_with(|req: &axum::http::Request<_>| {
                let path = access_log_path(req.uri().path());
                let method = req.method().clone();
                // Noisy terminal I/O gets a TRACE-level span (filtered out at the
                // dev `debug` level); everything else keeps the INFO access span.
                if is_noisy_terminal_path(path.as_ref()) {
                    tracing::trace_span!("http.io", method = %method, path = %path)
                } else {
                    tracing::info_span!("http", method = %method, path = %path)
                }
            })
            .on_response(
                |res: &axum::http::Response<_>, latency: std::time::Duration, span: &tracing::Span| {
                    let status = res.status().as_u16();
                    let latency_ms = latency.as_millis() as u64;
                    // The span carries the request's level: TRACE for noisy
                    // terminal I/O (a disabled TRACE span still reports its
                    // metadata level), INFO otherwise. Mirror it for the
                    // response event so noisy success responses stay silent.
                    let is_noisy = span
                        .metadata()
                        .map(|m| *m.level() == tracing::Level::TRACE)
                        .unwrap_or(false);
                    if status >= 500 {
                        tracing::error!(status, latency_ms, "response");
                    } else if status >= 400 {
                        tracing::warn!(status, latency_ms, "response");
                    } else if is_noisy {
                        tracing::trace!(status, latency_ms, "response");
                    } else {
                        tracing::info!(status, latency_ms, "response");
                    }
                },
            )
            .on_failure(
                |error: tower_http::classify::ServerErrorsFailureClass,
                 latency: std::time::Duration,
                 _span: &tracing::Span| {
                    tracing::error!(
                        %error,
                        latency_ms = latency.as_millis() as u64,
                        "request failed"
                    );
                },
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::{access_log_path, is_noisy_terminal_path};

    const CAPABILITY: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn redacts_only_structural_office_preview_capabilities() {
        assert_eq!(
            access_log_path(&format!("/api/ppt-proxy/{CAPABILITY}")),
            "/api/ppt-proxy/[REDACTED]"
        );
        assert_eq!(
            access_log_path(&format!(
                "/api/office-watch-proxy/{CAPABILITY}/assets/app.js"
            )),
            "/api/office-watch-proxy/[REDACTED]/assets/app.js"
        );
        assert_eq!(
            access_log_path(&format!("/api/office-watch-proxy/{CAPABILITY}/")),
            "/api/office-watch-proxy/[REDACTED]/"
        );
    }

    #[test]
    fn does_not_broaden_redaction_to_similar_or_legacy_paths() {
        for path in [
            "/api/ppt-proxy/43210/",
            "/api/office-watch-proxy/not-a-capability/",
            "/api/not-ppt-proxy/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/",
            "/other/ppt-proxy/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/",
        ] {
            assert_eq!(access_log_path(path), path);
        }
    }

    #[test]
    fn filters_terminal_input_and_resize() {
        assert!(is_noisy_terminal_path("/api/terminals/abc123/input"));
        assert!(is_noisy_terminal_path("/api/terminals/abc123/resize"));
    }

    #[test]
    fn keeps_lifecycle_and_unrelated_paths() {
        assert!(!is_noisy_terminal_path("/api/terminals")); // list/create
        assert!(!is_noisy_terminal_path("/api/terminals/abc123")); // get/delete
        assert!(!is_noisy_terminal_path("/api/terminals/abc123/kill"));
        assert!(!is_noisy_terminal_path("/api/terminals/abc123/relaunch"));
        assert!(!is_noisy_terminal_path("/api/conversations/x/input"));
    }
}
