pub mod api_response;
pub mod egress;
pub mod proxy;
pub mod secret_redaction;

use std::time::Duration;

/// Ceiling on establishing a TCP+TLS connection.
///
/// Without one, a blackholed route or a stalled resolver hangs until the
/// caller's own (often much longer) request timeout fires — or indefinitely
/// where the caller sets none. It bounds only connection setup, so slow-but-
/// working transfers are unaffected.
///
/// Deliberately no `read_timeout` here: this client is shared with long-lived
/// consumers (MCP SSE sessions, browser-engine archive downloads) where an
/// idle read gap is normal. Per-request timeouts remain each caller's choice.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

pub fn http_client() -> reqwest::Client {
    proxy::apply_detected_proxy(reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Build the shared outbound client with redirects disabled.
///
/// Provider attempts must have exactly one network request boundary. Following
/// an HTTP redirect would silently create a second POST outside the Broker's
/// retry/failover accounting and could forward credentials to a new origin.
pub fn http_client_no_redirect() -> Result<reqwest::Client, reqwest::Error> {
    proxy::apply_detected_proxy(
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none()),
    )
    .build()
}
