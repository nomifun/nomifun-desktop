use std::sync::Arc;
use std::time::Duration;

use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use nomifun_api_types::WebSocketMessage;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::manager::{TokenAuthenticator, WebSocketManager};
use crate::types::{ConnectionId, PER_CONNECTION_BUFFER, WebSocketCloseCode, WsOutbound};

/// Extracts a JWT token from WebSocket upgrade request headers.
///
/// Injected by `nomifun-app` — wraps `nomifun_auth::extract_token_from_ws_headers`
/// so that `nomifun-realtime` does not depend on `nomifun-auth` directly.
pub type TokenExtractor = Arc<dyn Fn(&HeaderMap) -> Option<String> + Send + Sync>;

/// Shared state required by the WebSocket upgrade handler.
#[derive(Clone)]
pub struct WsHandlerState {
    pub manager: Arc<WebSocketManager>,
    pub token_authenticator: TokenAuthenticator,
    pub token_extractor: TokenExtractor,
    /// Operator-configured browser origins additionally accepted for the
    /// handshake (normalized `scheme://authority`, lowercase). Built with
    /// [`parse_allowed_origins`]; the deterministic escape hatch for reverse
    /// proxies that forward neither the original `Host` nor `X-Forwarded-Host`.
    pub allowed_origins: Arc<[String]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OriginDisposition {
    /// Browsers send `Origin` on WebSocket handshakes. Its absence is retained
    /// for CLI, agent, and other non-browser clients that authenticate with an
    /// explicit credential.
    NonBrowser,
    SameOrigin,
    /// The desktop renderer and local Vite renderer run on a different local
    /// origin from the embedded loopback backend.
    LocalWebview,
}

struct UpgradeCredentials {
    token: Option<String>,
    selected_protocol: Option<String>,
}

/// Axum handler for HTTP → WebSocket upgrade.
///
/// Validates an origin-bearing browser handshake before extracting its JWT,
/// then upgrades the connection to WebSocket on success.
/// On authentication failure, sends `auth-expired` and closes with 1008.
///
/// When the token is carried via `Sec-WebSocket-Protocol`, the server
/// echoes the protocol header back so the client handshake succeeds.
pub async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    axum::extract::State(state): axum::extract::State<WsHandlerState>,
) -> Response {
    // Validate Origin before touching cookie/Bearer credentials. In
    // particular, a hostile page must not be able to use an ambient session
    // cookie for a cross-site WebSocket handshake.
    let credentials = match upgrade_credentials(&headers, &state) {
        Ok(credentials) => credentials,
        Err(()) => {
            log_rejected_handshake(&headers);
            return StatusCode::FORBIDDEN.into_response();
        }
    };

    // Echo the selected Sec-WebSocket-Protocol so clients using it for auth
    // receive a valid subprotocol negotiation response. Select one protocol,
    // rather than echoing a possibly comma-delimited request header.
    let ws = if let Some(protocol) = credentials.selected_protocol.clone() {
        ws.protocols([protocol])
    } else {
        ws
    };

    ws.on_upgrade(move |socket| async move {
        handle_socket(socket, credentials.token, state).await;
    })
    .into_response()
}

/// Resolve the authentication material permitted for this request's origin.
///
/// Same-origin browsers retain the existing extractor priority (Bearer,
/// cookie, then subprotocol). A known local desktop/dev webview is necessarily
/// cross-origin with the loopback backend, so it may connect only when the
/// first requested subprotocol is itself a valid explicit credential. This
/// prevents an ambient cookie from authorizing a local cross-origin page.
fn upgrade_credentials(headers: &HeaderMap, state: &WsHandlerState) -> Result<UpgradeCredentials, ()> {
    match validate_origin(headers, &state.allowed_origins)? {
        OriginDisposition::LocalWebview => {
            let token = if let Some(protocol) = first_requested_protocol(headers) {
                let protocol = protocol.to_owned();
                (state.token_authenticator)(&protocol).ok_or(())?;
                protocol
            } else {
                // `--insecure-no-auth` / `dev:web` deliberately has no
                // browser-visible credential and its application-provided
                // extractor returns the synthetic local token. Preserve that
                // legal local development handshake, but remove every ambient
                // credential first so a Cookie or Bearer value can never make
                // an authenticated deployment take this fallback.
                let mut credentialless_headers = headers.clone();
                credentialless_headers.remove(header::AUTHORIZATION);
                credentialless_headers.remove(header::COOKIE);
                credentialless_headers.remove(header::SEC_WEBSOCKET_PROTOCOL);
                let token = (state.token_extractor)(&credentialless_headers).ok_or(())?;
                (state.token_authenticator)(&token).ok_or(())?;
                token
            };
            Ok(UpgradeCredentials {
                token: Some(token),
                selected_protocol: first_requested_protocol(headers).map(str::to_owned),
            })
        }
        OriginDisposition::NonBrowser | OriginDisposition::SameOrigin => Ok(UpgradeCredentials {
            token: (state.token_extractor)(headers),
            selected_protocol: first_requested_protocol(headers).map(str::to_owned),
        }),
    }
}

/// Validate a browser WebSocket Origin against the authorities this
/// deployment is reachable as.
///
/// This follows the application's existing browser-viewer origin model:
/// ordinary WebUI browsers must present an Origin whose authority matches the
/// request `Host`, the first (client-facing) `X-Forwarded-Host` entry set by a
/// fronting reverse proxy, or an operator-configured allowed origin. The known
/// Tauri and loopback development origins are classified separately and bound
/// to explicit subprotocol authentication by [`upgrade_credentials`]. Missing
/// Origin is allowed only as the non-browser compatibility path. Any present
/// but malformed, opaque, duplicated, or untrusted Origin fails closed.
///
/// Accepting `X-Forwarded-Host` does not weaken the CSRF boundary this check
/// exists for: a hostile page's `new WebSocket(...)` handshake carries only
/// browser-controlled headers — scripts cannot attach `X-Forwarded-Host`, so
/// for cookie-bearing browser traffic the header is always proxy-controlled.
/// Non-browser clients can forge it, but they hold no ambient credential:
/// with no Origin they already take the [`OriginDisposition::NonBrowser`]
/// path, and either way the handshake still requires a valid token.
fn validate_origin(headers: &HeaderMap, allowed_origins: &[String]) -> Result<OriginDisposition, ()> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let Some(origin_value) = origins.next() else {
        return Ok(OriginDisposition::NonBrowser);
    };
    if origins.next().is_some() {
        return Err(());
    }
    let origin = origin_value
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|origin| !origin.is_empty() && !origin.contains(','))
        .ok_or(())?;

    let uri = origin.parse::<Uri>().map_err(|_| ())?;
    let scheme = uri.scheme_str().ok_or(())?;
    let authority = uri.authority().ok_or(())?;
    if uri
        .path_and_query()
        .is_some_and(|path_and_query| path_and_query.as_str() != "/")
        || authority.as_str().contains('@')
    {
        return Err(());
    }

    // Operator-configured allowlist: exact scheme + authority. Checked before
    // the Host comparison so a proxy that mangles Host cannot defeat an
    // explicit configuration — which also means an allowlisted origin is
    // admitted even when Host is duplicated or unparseable; the fail-closed
    // Host hygiene below applies to every non-allowlisted origin.
    let normalized_origin = format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.as_str().to_ascii_lowercase()
    );
    if allowed_origins.contains(&normalized_origin) {
        return Ok(OriginDisposition::SameOrigin);
    }

    // Host stays strict where present: duplicates or garbage fail closed.
    // Absence (HTTP/2 upstream hops put the authority in `:authority`) leaves
    // only the other authority sources: a matching X-Forwarded-Host below, or
    // the credential-gated LocalWebview classification for the known local
    // origins. Everything else still fails closed.
    let mut hosts = headers.get_all(header::HOST).iter();
    let host = match hosts.next() {
        Some(value) => {
            if hosts.next().is_some() {
                return Err(());
            }
            let host = value
                .to_str()
                .ok()
                .map(str::trim)
                .filter(|host| !host.is_empty() && !host.contains(',') && !host.contains('@'))
                .ok_or(())?;
            Some(host.parse::<axum::http::uri::Authority>().map_err(|_| ())?)
        }
        None => None,
    };

    let forwarded_host = first_forwarded_host(headers);

    if matches!(scheme, "http" | "https") {
        let origin_matches = |candidate: &Option<axum::http::uri::Authority>| {
            candidate
                .as_ref()
                .is_some_and(|candidate| authority.as_str().eq_ignore_ascii_case(candidate.as_str()))
        };
        if origin_matches(&host) || origin_matches(&forwarded_host) {
            return Ok(OriginDisposition::SameOrigin);
        }
    }

    let origin_host = authority
        .host()
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or_else(|| authority.host());
    let is_tauri_origin = (scheme == "tauri" && origin_host.eq_ignore_ascii_case("localhost"))
        || (matches!(scheme, "http" | "https") && origin_host.eq_ignore_ascii_case("tauri.localhost"));
    let is_loopback_dev_origin = matches!(scheme, "http" | "https")
        && (origin_host.eq_ignore_ascii_case("localhost") || origin_host == "127.0.0.1" || origin_host == "::1");

    if is_tauri_origin || is_loopback_dev_origin {
        Ok(OriginDisposition::LocalWebview)
    } else {
        Err(())
    }
}

/// The first (client-facing) `X-Forwarded-Host` entry, parsed with the same
/// hygiene as `Host`. Chained proxies append their hop to the right, so the
/// first entry of the first header is the authority the browser actually
/// requested — the value its Origin will name. A malformed value degrades to
/// `None` (it can only deny, never grant).
fn first_forwarded_host(headers: &HeaderMap) -> Option<axum::http::uri::Authority> {
    headers
        .get("x-forwarded-host")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.contains('@'))
        .and_then(|value| value.parse::<axum::http::uri::Authority>().ok())
}

/// Parse an operator-supplied comma-separated origin allowlist
/// (`NOMIFUN_ALLOWED_ORIGINS`) into normalized `scheme://authority` strings.
///
/// Entries must be full `http`/`https` origins with no path/query/userinfo; a
/// single trailing `/` is tolerated and a redundant default port (`:80`/`:443`)
/// is stripped, since browser `Origin` values never carry it. Invalid entries
/// are skipped with a warning — including the literal `null` origin (sandboxed
/// iframes, `file://` pages), non-HTTP schemes, and `tauri.localhost` (the
/// desktop webview must stay bound to its per-boot secret, never to an
/// ambient cookie). Loopback entries are accepted but warned about: they
/// convert the deliberately token-only local-webview classification into a
/// cookie-accepting one for that exact origin.
pub fn parse_allowed_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let candidate = entry.strip_suffix('/').unwrap_or(entry);
            let parsed = candidate.parse::<Uri>().ok().and_then(|uri| {
                let scheme = uri.scheme_str()?.to_ascii_lowercase();
                let authority = uri.authority()?;
                if !matches!(scheme.as_str(), "http" | "https")
                    || uri
                        .path_and_query()
                        .is_some_and(|path_and_query| path_and_query.as_str() != "/")
                    || authority.as_str().contains('@')
                    || authority.host().eq_ignore_ascii_case("null")
                    || authority.host().eq_ignore_ascii_case("tauri.localhost")
                {
                    return None;
                }
                let mut authority = authority.as_str().to_ascii_lowercase();
                let default_port = if scheme == "https" { ":443" } else { ":80" };
                if let Some(stripped) = authority.strip_suffix(default_port) {
                    authority = stripped.to_owned();
                }
                let is_loopback_entry = ["localhost", "127.0.0.1", "[::1]"].iter().any(|loopback| {
                    authority == *loopback
                        || authority
                            .strip_prefix(loopback)
                            .is_some_and(|rest| rest.starts_with(':'))
                });
                if is_loopback_entry {
                    tracing::warn!(
                        entry,
                        "allowed-origin entry names a loopback origin: pages served at exactly this \
                         origin may authenticate with the ambient session cookie instead of the \
                         token-only local-webview handshake"
                    );
                }
                Some(format!("{scheme}://{authority}"))
            });
            if parsed.is_none() {
                tracing::warn!(
                    entry,
                    "ignoring invalid allowed-origin entry; expected a full http(s) origin like https://nomi.example.com"
                );
            }
            parsed
        })
        .collect()
}

/// WARN once per distinct (origin, host, forwarded-host) combination that a
/// handshake was rejected, with the actual values. A deployment behind a
/// misconfigured proxy rejects the SAME combination thousands of times a day;
/// without the values in the log the failure is undiagnosable, and with an
/// unconditional WARN it would drown the log. The set is bounded so hostile
/// scanners cannot grow it without limit — and so a scanner cannot silence
/// the diagnostics either: once the set is full, unseen combinations still
/// WARN at most once per hour instead of dropping to debug forever.
fn log_rejected_handshake(headers: &HeaderMap) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    const MAX_DISTINCT_REJECTIONS_LOGGED: usize = 64;
    const MAX_LOGGED_VALUE_LEN: usize = 256;
    const OVERFLOW_WARN_INTERVAL: Duration = Duration::from_secs(3600);
    static SEEN: OnceLock<Mutex<(HashSet<String>, Option<Instant>)>> = OnceLock::new();

    let header_str = |name: &str| {
        let value = headers
            .get(name)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
            .unwrap_or_else(|| "<absent>".to_owned());
        match value.char_indices().nth(MAX_LOGGED_VALUE_LEN) {
            Some((cut, _)) => format!("{}…", &value[..cut]),
            None => value,
        }
    };
    let origin = header_str("origin");
    let host = header_str("host");
    let forwarded_host = header_str("x-forwarded-host");

    let key = format!("{origin}|{host}|{forwarded_host}");
    let warn = SEEN
        .get_or_init(Default::default)
        .lock()
        .map(|mut state| {
            let (seen, last_overflow_warn) = &mut *state;
            if seen.contains(&key) {
                return false;
            }
            if seen.len() < MAX_DISTINCT_REJECTIONS_LOGGED {
                seen.insert(key);
                return true;
            }
            let due = last_overflow_warn.is_none_or(|at| at.elapsed() >= OVERFLOW_WARN_INTERVAL);
            if due {
                *last_overflow_warn = Some(Instant::now());
            }
            due
        })
        .unwrap_or(false);

    if warn {
        tracing::warn!(
            origin,
            host,
            forwarded_host,
            "rejected websocket upgrade: the browser Origin matches neither the request Host nor a \
             trusted forwarded/configured authority, so realtime updates cannot connect. If this \
             deployment sits behind a reverse proxy, forward the original authority \
             (e.g. nginx: `proxy_set_header Host $host;`) or set NOMIFUN_ALLOWED_ORIGINS to the \
             public origin. Further rejections of this combination are logged at debug level."
        );
    } else {
        debug!(origin, host, forwarded_host, "rejected websocket upgrade with an untrusted origin");
    }
}

fn first_requested_protocol(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|protocols| protocols.split(',').next())
        .map(str::trim)
        .filter(|protocol| !protocol.is_empty())
}

/// Post-upgrade connection handler.
///
/// Validates the token, registers the client, spawns send/recv loops.
async fn handle_socket(socket: WebSocket, token: Option<String>, state: WsHandlerState) {
    let Some(token) = token else {
        send_close_no_token(socket).await;
        return;
    };

    let Some(user_id) = (state.token_authenticator)(&token) else {
        send_auth_expired_and_close(socket).await;
        return;
    };

    let (tx, rx) = mpsc::channel::<WsOutbound>(PER_CONNECTION_BUFFER);
    let conn_id = state.manager.add_client(user_id, token, tx);
    let cancellation = state
        .manager
        .connection_cancellation(conn_id)
        .expect("newly registered websocket has a cancellation signal");

    info!(%conn_id, "websocket connection established");

    let (ws_sender, ws_receiver) = socket.split();

    let mut send_handle = tokio::spawn(send_loop(
        conn_id,
        rx,
        ws_sender,
        cancellation.clone(),
    ));
    let mut send_loop_finished = false;
    let server_close = tokio::select! {
        _ = recv_loop(conn_id, ws_receiver, &state) => false,
        _ = cancellation.cancelled() => true,
        _ = &mut send_handle => {
            // A heartbeat policy close is queued before its manager entry is
            // removed. Once that queue drains, the send loop ending must also
            // terminate an otherwise-idle receive loop.
            send_loop_finished = true;
            false
        },
    };

    if server_close {
        // Let the send loop put a real close frame on the wire. A bounded wait
        // avoids a stalled socket sink retaining the task indefinitely.
        let _ = tokio::time::timeout(Duration::from_secs(1), &mut send_handle).await;
    }
    if !send_loop_finished {
        send_handle.abort();
    }
    state.manager.remove_client(conn_id);
    info!(%conn_id, "websocket connection closed");
}

/// Send a close frame with 1008 when no token is provided.
async fn send_close_no_token(mut socket: WebSocket) {
    let close = Message::Close(Some(CloseFrame {
        code: WebSocketCloseCode::PolicyViolation.as_u16(),
        reason: "no token provided".into(),
    }));
    let _ = socket.send(close).await;
}

/// Send `auth-expired` event then close with 1008.
async fn send_auth_expired_and_close(mut socket: WebSocket) {
    let auth_expired = WebSocketMessage::new("auth-expired", json!({"message": "Token expired or invalid"}));
    if let Ok(text) = serde_json::to_string(&auth_expired) {
        let _ = socket.send(Message::Text(text.into())).await;
    }
    let close = Message::Close(Some(CloseFrame {
        code: WebSocketCloseCode::PolicyViolation.as_u16(),
        reason: "authentication failed".into(),
    }));
    let _ = socket.send(close).await;
}

// -------------------------------------------------------------------
// Send loop
// -------------------------------------------------------------------

/// Reads `WsOutbound` from the per-connection channel and forwards
/// them to the WebSocket sink.
async fn send_loop(
    conn_id: ConnectionId,
    mut rx: mpsc::Receiver<WsOutbound>,
    mut sender: futures_util::stream::SplitSink<WebSocket, Message>,
    cancellation: CancellationToken,
) {
    loop {
        let outbound = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                let close = Message::Close(Some(CloseFrame {
                    code: WebSocketCloseCode::NormalClosure.as_u16(),
                    reason: "server resync requested".into(),
                }));
                let _ = sender.send(close).await;
                break;
            }
            outbound = rx.recv() => match outbound {
                Some(outbound) => outbound,
                None => break,
            },
        };
        let msg = match outbound {
            WsOutbound::Text(text) => Message::Text(text.into()),
            WsOutbound::Close(code, reason) => Message::Close(Some(CloseFrame {
                code: code.as_u16(),
                reason: reason.into(),
            })),
        };
        if sender.send(msg).await.is_err() {
            debug!(%conn_id, "send loop: socket write failed, exiting");
            break;
        }
    }
}

// -------------------------------------------------------------------
// Receive loop
// -------------------------------------------------------------------

/// Reads messages from the WebSocket stream, parses JSON, routes.
async fn recv_loop(
    conn_id: ConnectionId,
    mut receiver: futures_util::stream::SplitStream<WebSocket>,
    state: &WsHandlerState,
) {
    while let Some(result) = receiver.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                debug!(%conn_id, error = %e, "recv error, closing");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                handle_text_message(conn_id, &text, state);
            }
            Message::Close(_) => {
                debug!(%conn_id, "received close frame");
                break;
            }
            // Ping/Pong at the WebSocket protocol level are handled
            // automatically by axum/tungstenite. Binary frames are ignored.
            _ => {}
        }
    }
}

/// Process a text message: parse JSON and dispatch the built-in kinds.
///
/// Business requests flow over HTTP, not the WebSocket: any upstream message
/// other than `pong` / `subscribe-show-open` is discarded with a debug log.
fn handle_text_message(conn_id: ConnectionId, text: &str, state: &WsHandlerState) {
    let parsed: Result<WebSocketMessage<Value>, _> = serde_json::from_str(text);

    let msg = match parsed {
        Ok(m) => m,
        Err(_) => {
            send_error_response(state, conn_id);
            return;
        }
    };

    match msg.name.as_str() {
        "pong" => {
            state.manager.update_last_ping(conn_id);
        }
        "subscribe-show-open" => {
            handle_subscribe_show_open(state, conn_id, msg.data);
        }
        name => {
            debug!(%conn_id, message_name = name, "unhandled upstream WS message discarded");
        }
    }
}

/// Send an error response for invalid message format.
fn send_error_response(state: &WsHandlerState, conn_id: ConnectionId) {
    let error = json!({
        "error": "Invalid message format",
        "expected": r#"{ "name": "event-name", "data": {...} }"#
    });

    if let Ok(text) = serde_json::to_string(&error) {
        state.manager.send_raw_to(conn_id, WsOutbound::Text(text));
    }
}

/// Handle `subscribe-show-open`: reply with `show-open-request`.
///
/// The inbound `data` is the @office-ai/platform bridge envelope
/// `{ id, data: <user-params> }`. The renderer awaits a callback whose event
/// name embeds `id` (`subscribe.callback-show-open<id>`), so we must echo it
/// back; without it, the frontend's `useDirectorySelection` hook builds the
/// wrong callback name and the original `invoke()` Promise never resolves.
///
/// `isFileMode` is `true` when `properties` contains `openFile`
/// but NOT `openDirectory`.
fn handle_subscribe_show_open(state: &WsHandlerState, conn_id: ConnectionId, data: Value) {
    let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let inner = data.get("data").unwrap_or(&Value::Null);

    let properties = inner
        .get("properties")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let has_open_file = properties.iter().any(|v| v.as_str() == Some("openFile"));
    let has_open_directory = properties.iter().any(|v| v.as_str() == Some("openDirectory"));

    let is_file_mode = has_open_file && !has_open_directory;

    let response = WebSocketMessage::new(
        "show-open-request",
        json!({
            "id": id,
            "properties": properties,
            "isFileMode": is_file_mode,
        }),
    );

    state.manager.send_to(conn_id, response);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(manager: Arc<WebSocketManager>) -> WsHandlerState {
        WsHandlerState {
            manager,
            token_authenticator: Arc::new(|_| Some("user".to_owned())),
            token_extractor: Arc::new(|_| None),
            allowed_origins: Arc::from(Vec::new()),
        }
    }

    fn origin_headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, origin.parse().unwrap());
        }
        headers
    }

    #[test]
    fn origin_validation_accepts_same_authority_and_non_browser_absence() {
        assert_eq!(
            validate_origin(&origin_headers("nomifun.example:8443", None), &[]),
            Ok(OriginDisposition::NonBrowser)
        );
        assert_eq!(
            validate_origin(
                &origin_headers("nomifun.example:8443", Some("https://NOMIFUN.example:8443")),
                &[]
            ),
            Ok(OriginDisposition::SameOrigin)
        );
        assert!(
            validate_origin(
                &origin_headers("nomifun.example:8443", Some("https://nomifun.example")),
                &[]
            )
            .is_err(),
            "the origin port is part of the WebSocket same-origin boundary"
        );
    }

    /// A reverse proxy that rewrites `Host` to its upstream address (nginx's
    /// `proxy_set_header Host $proxy_host` default, several tunnel tools) must
    /// not break the browser handshake as long as it forwards the original
    /// authority in `X-Forwarded-Host` — that header is proxy-controlled and
    /// can never be set by a hostile page's WebSocket handshake.
    #[test]
    fn origin_validation_accepts_first_forwarded_host_authority() {
        let mut headers = origin_headers("nomifun-upstream:8787", Some("https://nomi.example.com"));
        headers.insert("x-forwarded-host", "nomi.example.com".parse().unwrap());
        assert_eq!(validate_origin(&headers, &[]), Ok(OriginDisposition::SameOrigin));

        // Explicit port must match exactly, like the Host comparison.
        let mut headers = origin_headers("nomifun-upstream:8787", Some("https://nomi.example.com:8443"));
        headers.insert("x-forwarded-host", "nomi.example.com:8443".parse().unwrap());
        assert_eq!(validate_origin(&headers, &[]), Ok(OriginDisposition::SameOrigin));

        // Chained proxies append; only the FIRST (client-facing) entry counts.
        let mut headers = origin_headers("nomifun-upstream:8787", Some("https://nomi.example.com"));
        headers.insert(
            "x-forwarded-host",
            "nomi.example.com, internal-tier2:8080".parse().unwrap(),
        );
        assert_eq!(validate_origin(&headers, &[]), Ok(OriginDisposition::SameOrigin));
        let mut headers = origin_headers("nomifun-upstream:8787", Some("http://internal-tier2:8080"));
        headers.insert(
            "x-forwarded-host",
            "nomi.example.com, internal-tier2:8080".parse().unwrap(),
        );
        assert!(
            validate_origin(&headers, &[]).is_err(),
            "non-first forwarded entries are intermediate hops, not the browser origin"
        );

        // An untrusted origin still fails even when X-Forwarded-Host is present.
        let mut headers = origin_headers("nomifun-upstream:8787", Some("https://attacker.example"));
        headers.insert("x-forwarded-host", "nomi.example.com".parse().unwrap());
        assert!(validate_origin(&headers, &[]).is_err());

        // Host match keeps working when a (different) X-Forwarded-Host rides along.
        let mut headers = origin_headers("nomi.example.com", Some("https://nomi.example.com"));
        headers.insert("x-forwarded-host", "something-else.example".parse().unwrap());
        assert_eq!(validate_origin(&headers, &[]), Ok(OriginDisposition::SameOrigin));
    }

    /// HTTP/2 upstream hops carry the authority in `:authority`, not `Host`.
    /// A missing Host header with a matching `X-Forwarded-Host` must succeed;
    /// a missing Host with no forwarded authority stays failing closed.
    #[test]
    fn origin_validation_handles_missing_host_with_forwarded_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://nomi.example.com".parse().unwrap());
        headers.insert("x-forwarded-host", "nomi.example.com".parse().unwrap());
        assert_eq!(validate_origin(&headers, &[]), Ok(OriginDisposition::SameOrigin));

        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://nomi.example.com".parse().unwrap());
        assert!(validate_origin(&headers, &[]).is_err());
    }

    /// `NOMIFUN_ALLOWED_ORIGINS` is the deterministic operator escape hatch for
    /// deployments whose proxy forwards neither the original Host nor
    /// X-Forwarded-Host. Matching is exact on scheme + authority.
    #[test]
    fn origin_validation_accepts_configured_allowed_origins() {
        let allowed = parse_allowed_origins("https://Nomi.Example.com/, http://10.0.0.5:8787");
        assert_eq!(
            allowed,
            vec!["https://nomi.example.com".to_owned(), "http://10.0.0.5:8787".to_owned()]
        );

        assert_eq!(
            validate_origin(
                &origin_headers("nomifun-upstream:8787", Some("https://nomi.example.com")),
                &allowed
            ),
            Ok(OriginDisposition::SameOrigin)
        );
        assert_eq!(
            validate_origin(
                &origin_headers("nomifun-upstream:8787", Some("http://10.0.0.5:8787")),
                &allowed
            ),
            Ok(OriginDisposition::SameOrigin)
        );
        // Scheme is part of the identity: https entry does not admit http.
        assert!(
            validate_origin(
                &origin_headers("nomifun-upstream:8787", Some("http://nomi.example.com")),
                &allowed
            )
            .is_err()
        );
        assert!(
            validate_origin(
                &origin_headers("nomifun-upstream:8787", Some("https://attacker.example")),
                &allowed
            )
            .is_err()
        );
    }

    /// Hostile or malformed allowlist entries are dropped at parse time. In
    /// particular the literal `null` origin (sandboxed iframes, file:// pages)
    /// must never become acceptable through configuration, and only http(s)
    /// origins are meaningful for a browser handshake.
    #[test]
    fn allowed_origins_parsing_rejects_unsafe_entries() {
        assert!(parse_allowed_origins("null").is_empty());
        assert!(parse_allowed_origins("nomi.example.com").is_empty(), "scheme is required");
        assert!(parse_allowed_origins("https://nomi.example.com/app").is_empty());
        assert!(parse_allowed_origins("https://user@nomi.example.com").is_empty());
        assert!(parse_allowed_origins("").is_empty());
        assert!(parse_allowed_origins(" , ,").is_empty());
        assert!(parse_allowed_origins("tauri://localhost").is_empty(), "non-http(s) schemes are refused");
        assert!(parse_allowed_origins("ws://nomi.example.com").is_empty());
        assert!(
            parse_allowed_origins("http://tauri.localhost").is_empty(),
            "the desktop webview origin must stay bound to its per-boot secret"
        );
        // One bad entry must not poison the valid ones around it.
        assert_eq!(
            parse_allowed_origins("null, https://nomi.example.com"),
            vec!["https://nomi.example.com".to_owned()]
        );
        // Loopback entries are accepted (with a warning): they are the escape
        // hatch for local tunnel clients that rewrite Host.
        assert_eq!(
            parse_allowed_origins("http://localhost:9000"),
            vec!["http://localhost:9000".to_owned()]
        );
    }

    /// Browsers omit the default port from `Origin`, so a configured
    /// `https://x:443` must match an incoming `https://x`.
    #[test]
    fn allowed_origins_parsing_strips_redundant_default_ports() {
        assert_eq!(
            parse_allowed_origins("https://nomi.example.com:443, http://nomi.example.com:80"),
            vec!["https://nomi.example.com".to_owned(), "http://nomi.example.com".to_owned()]
        );
        // Non-default ports are identity, not noise.
        assert_eq!(
            parse_allowed_origins("https://nomi.example.com:8443"),
            vec!["https://nomi.example.com:8443".to_owned()]
        );
        assert_eq!(
            validate_origin(
                &origin_headers("nomifun-upstream:8787", Some("https://nomi.example.com")),
                &parse_allowed_origins("https://nomi.example.com:443")
            ),
            Ok(OriginDisposition::SameOrigin)
        );
    }

    /// Two physically separate `X-Forwarded-Host` headers: only the first
    /// header's first entry (closest to the client) is trusted.
    #[test]
    fn forwarded_host_uses_only_the_first_header_instance() {
        let mut headers = origin_headers("nomifun-upstream:8787", Some("https://nomi.example.com"));
        headers.append("x-forwarded-host", "nomi.example.com".parse().unwrap());
        headers.append("x-forwarded-host", "second-hop.example".parse().unwrap());
        assert_eq!(validate_origin(&headers, &[]), Ok(OriginDisposition::SameOrigin));

        let mut headers = origin_headers("nomifun-upstream:8787", Some("https://second-hop.example"));
        headers.append("x-forwarded-host", "nomi.example.com".parse().unwrap());
        headers.append("x-forwarded-host", "second-hop.example".parse().unwrap());
        assert!(validate_origin(&headers, &[]).is_err());
    }

    /// A missing Host with a known local-webview Origin stays classified
    /// LocalWebview — reachable only over HTTP/2-style hops — and therefore
    /// still demands an explicit subprotocol credential, never a cookie.
    #[test]
    fn missing_host_with_local_origin_stays_credential_gated() {
        for origin in ["tauri://localhost", "http://localhost:5173"] {
            let mut headers = HeaderMap::new();
            headers.insert(header::ORIGIN, origin.parse().unwrap());
            assert_eq!(
                validate_origin(&headers, &[]),
                Ok(OriginDisposition::LocalWebview),
                "expected {origin} without Host to stay on the credential-gated path"
            );
        }
    }

    #[test]
    fn origin_validation_classifies_known_local_webview_origins() {
        for origin in [
            "tauri://localhost",
            "http://tauri.localhost",
            "http://localhost:5173",
            "http://127.0.0.1:5173",
            "http://[::1]:5173",
        ] {
            assert_eq!(
                validate_origin(&origin_headers("127.0.0.1:25808", Some(origin)), &[]),
                Ok(OriginDisposition::LocalWebview),
                "expected {origin} to be a known local webview origin"
            );
        }
    }

    #[test]
    fn origin_validation_fails_closed_for_untrusted_or_ambiguous_values() {
        for origin in [
            "https://attacker.example",
            "null",
            "https://nomifun.example:8443/path",
            "https://user@nomifun.example:8443",
            "https://nomifun.example:8443, https://attacker.example",
        ] {
            assert!(
                validate_origin(&origin_headers("nomifun.example:8443", Some(origin)), &[]).is_err(),
                "expected {origin} to be rejected"
            );
        }

        let mut duplicate = origin_headers("nomifun.example:8443", None);
        duplicate.append(header::ORIGIN, "https://nomifun.example:8443".parse().unwrap());
        duplicate.append(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(validate_origin(&duplicate, &[]).is_err());

        let mut missing_host = HeaderMap::new();
        missing_host.insert(header::ORIGIN, "https://nomifun.example:8443".parse().unwrap());
        assert!(validate_origin(&missing_host, &[]).is_err());
    }

    #[test]
    fn subscribe_show_open_file_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "abc123", "data": {"properties": ["openFile"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["name"], "show-open-request");
                assert_eq!(parsed["data"]["id"], "abc123");
                assert_eq!(parsed["data"]["isFileMode"], true);
                assert_eq!(parsed["data"]["properties"], json!(["openFile"]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_directory_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "dir1", "data": {"properties": ["openDirectory"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "dir1");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_mixed_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "mixed", "data": {"properties": ["openFile", "openDirectory"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "mixed");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_empty_properties() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "empty", "data": {"properties": []}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "empty");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_missing_properties() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        handle_subscribe_show_open(&state, conn_id, json!({"id": "noprops", "data": {}}));

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "noprops");
                assert_eq!(parsed["data"]["isFileMode"], false);
                assert_eq!(parsed["data"]["properties"], json!([]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_missing_id_falls_back_to_empty_string() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        handle_subscribe_show_open(&state, conn_id, json!({}));

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "");
                assert_eq!(parsed["data"]["isFileMode"], false);
                assert_eq!(parsed["data"]["properties"], json!([]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn text_message_pong_updates_last_ping() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, _rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        std::thread::sleep(std::time::Duration::from_millis(5));

        handle_text_message(conn_id, r#"{"name":"pong","data":{}}"#, &state);
        // No panic = success (update_last_ping was called)
    }

    #[test]
    fn text_message_invalid_json_sends_error() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        handle_text_message(conn_id, "not json", &state);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["error"], "Invalid message format");
                assert!(parsed["expected"].is_string());
            }
            _ => panic!("expected error text"),
        }
    }

    #[test]
    fn text_message_missing_fields_sends_error() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        handle_text_message(conn_id, r#"{"foo":"bar"}"#, &state);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["error"], "Invalid message format");
            }
            _ => panic!("expected error text"),
        }
    }

    #[test]
    fn text_message_unknown_is_discarded_without_response() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);

        let state = test_state(manager);

        handle_text_message(
            conn_id,
            r#"{"name":"conversation.send-message","data":{"text":"hi"}}"#,
            &state,
        );

        // Unknown upstream messages are silently discarded: no error frame.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn error_response_to_disconnected_client_is_noop() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        drop(rx); // close channel

        let state = test_state(manager.clone());

        // Should not panic — client will be removed
        send_error_response(&state, conn_id);
        assert_eq!(manager.client_count(), 0);
    }
}
