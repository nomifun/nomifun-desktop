//! HTTP request access-log layer.

use axum::Router;
use tower_http::trace::TraceLayer;

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
                let path = req.uri().path();
                let method = req.method().clone();
                // Noisy terminal I/O gets a TRACE-level span (filtered out at the
                // dev `debug` level); everything else keeps the INFO access span.
                if is_noisy_terminal_path(path) {
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
    use super::is_noisy_terminal_path;

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
