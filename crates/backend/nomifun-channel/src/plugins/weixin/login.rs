use std::time::Duration;

use reqwest::Client;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use super::api::WeixinApi;
use super::types::{SseDoneEvent, SseErrorEvent, SseQrEvent};

/// Default base URL for the iLink Bot login API.
const LOGIN_BASE_URL: &str = "https://ilinkai.weixin.qq.com";

/// Polling interval for checking QR code scan status.
const QR_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Maximum time to wait for QR code scan before timeout.
const QR_LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// SSE event emitted during the WeChat QR code login flow.
#[derive(Debug, Clone)]
pub enum WeixinLoginEvent {
    /// QR code ticket data — frontend renders this as a QR image.
    Qr(String),
    /// User scanned the QR code.
    Scanned,
    /// Login successful — returns credentials for `channel.enable-plugin`.
    Done {
        account_id: String,
        bot_token: String,
        base_url: String,
    },
    /// Login failed with an error message.
    Error(String),
}

impl WeixinLoginEvent {
    /// SSE event name string.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Qr(_) => "qr",
            Self::Scanned => "scanned",
            Self::Done { .. } => "done",
            Self::Error(_) => "error",
        }
    }

    /// Serialize as a single `channel.weixin-login` WS payload, using a `phase`
    /// field as the discriminator (the SSE event-name dimension is folded in).
    ///
    /// This is the shape the desktop/WebUI frontend subscribes to over the
    /// WebSocket — the QR login flow can't use SSE because `EventSource` cannot
    /// carry the desktop's `x-nomi-local-trust` header.
    pub fn to_ws_payload(&self) -> serde_json::Value {
        match self {
            Self::Qr(ticket) => serde_json::json!({ "phase": "qr", "qrcodeData": ticket }),
            Self::Scanned => serde_json::json!({ "phase": "scanned" }),
            Self::Done {
                account_id,
                bot_token,
                base_url,
            } => serde_json::json!({
                "phase": "done",
                "accountId": account_id,
                "botToken": bot_token,
                "baseUrl": base_url,
            }),
            Self::Error(message) => serde_json::json!({ "phase": "error", "message": message }),
        }
    }

    /// Serialize the event payload as JSON.
    pub fn to_json_data(&self) -> String {
        match self {
            Self::Qr(ticket) => serde_json::to_string(&SseQrEvent {
                qrcode_data: ticket.clone(),
            })
            .unwrap_or_default(),
            Self::Scanned => "{}".into(),
            Self::Done {
                account_id,
                bot_token,
                base_url,
            } => serde_json::to_string(&SseDoneEvent {
                account_id: account_id.clone(),
                bot_token: bot_token.clone(),
                base_url: base_url.clone(),
            })
            .unwrap_or_default(),
            Self::Error(message) => serde_json::to_string(&SseErrorEvent {
                message: message.clone(),
            })
            .unwrap_or_default(),
        }
    }
}

/// Start the WeChat QR code login flow, returning a channel of SSE events.
pub fn weixin_login_stream() -> mpsc::Receiver<WeixinLoginEvent> {
    let (tx, rx) = mpsc::channel(16);
    tokio::spawn(login_flow(tx));
    rx
}

/// Internal login flow that drives the SSE event sequence.
async fn login_flow(tx: mpsc::Sender<WeixinLoginEvent>) {
    let client = match Client::builder().timeout(Duration::from_secs(40)).build() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx
                .send(WeixinLoginEvent::Error(format!("HTTP client init failed: {e}")))
                .await;
            return;
        }
    };

    let api = WeixinApi::new(client, LOGIN_BASE_URL, "");

    // Step 1: Fetch QR code
    let qr_data = match api.get_bot_qrcode().await {
        Ok(data) => data,
        Err(e) => {
            error!(error = %e, "Failed to fetch WeChat QR code");
            let _ = tx
                .send(WeixinLoginEvent::Error(format!("Failed to fetch QR code: {e}")))
                .await;
            return;
        }
    };

    let ticket = match qr_data.qrcode {
        Some(t) if !t.is_empty() => t,
        _ => {
            let _ = tx
                .send(WeixinLoginEvent::Error("QR code response missing ticket".into()))
                .await;
            return;
        }
    };

    let qr_content = match qr_data.qrcode_img_content {
        Some(ref url) if !url.is_empty() => url.clone(),
        _ => {
            let _ = tx
                .send(WeixinLoginEvent::Error(
                    "QR code response missing qrcode_img_content".into(),
                ))
                .await;
            return;
        }
    };

    info!("WeChat QR code generated, waiting for scan");
    if tx.send(WeixinLoginEvent::Qr(qr_content)).await.is_err() {
        return;
    }

    // Step 2: Poll for scan status
    let deadline = tokio::time::Instant::now() + QR_LOGIN_TIMEOUT;
    let mut scanned_sent = false;

    loop {
        if tokio::time::Instant::now() >= deadline {
            let _ = tx.send(WeixinLoginEvent::Error("QR code login timeout".into())).await;
            return;
        }

        tokio::time::sleep(QR_POLL_INTERVAL).await;

        match api.get_qrcode_status(&ticket).await {
            Ok(status) => {
                let state = status.status.as_deref().unwrap_or("wait");
                debug!(status = state, "WeChat QR code status");

                match state {
                    // NOTE: The API returns "scaned" (missing an 'n') — this is intentional.
                    "scaned" if !scanned_sent => {
                        scanned_sent = true;
                        if tx.send(WeixinLoginEvent::Scanned).await.is_err() {
                            return;
                        }
                    }
                    "confirmed" => {
                        let account_id = status.ilink_bot_id.unwrap_or_default();
                        let bot_token = status.bot_token.unwrap_or_default();
                        let base_url = status.baseurl.unwrap_or_else(|| LOGIN_BASE_URL.into());

                        info!(
                            account_id = %account_id,
                            "WeChat QR code login confirmed"
                        );
                        let _ = tx
                            .send(WeixinLoginEvent::Done {
                                account_id,
                                bot_token,
                                base_url,
                            })
                            .await;
                        return;
                    }
                    "expired" => {
                        let _ = tx.send(WeixinLoginEvent::Error("QR code expired".into())).await;
                        return;
                    }
                    _ => {}
                }
            }
            Err(e) => {
                // Timeout on long-poll is expected — treat as "wait" and retry
                let err_str = e.to_string();
                if err_str.contains("timed out") || err_str.contains("Timeout") {
                    debug!("QR status poll timeout, retrying");
                    continue;
                }
                error!(error = %e, "Failed to poll QR code status");
                let _ = tx
                    .send(WeixinLoginEvent::Error(format!("Status poll failed: {e}")))
                    .await;
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_event_names() {
        assert_eq!(WeixinLoginEvent::Qr("t".into()).event_name(), "qr");
        assert_eq!(WeixinLoginEvent::Scanned.event_name(), "scanned");
        assert_eq!(
            WeixinLoginEvent::Done {
                account_id: "a".into(),
                bot_token: "b".into(),
                base_url: "c".into(),
            }
            .event_name(),
            "done"
        );
        assert_eq!(WeixinLoginEvent::Error("err".into()).event_name(), "error");
    }

    #[test]
    fn login_event_qr_json() {
        let evt = WeixinLoginEvent::Qr("ticket_123".into());
        let json = evt.to_json_data();
        assert!(json.contains("qrcodeData"));
        assert!(json.contains("ticket_123"));
    }

    #[test]
    fn login_event_scanned_json() {
        let evt = WeixinLoginEvent::Scanned;
        assert_eq!(evt.to_json_data(), "{}");
    }

    #[test]
    fn login_event_done_json() {
        let evt = WeixinLoginEvent::Done {
            account_id: "acc_1".into(),
            bot_token: "tok_1".into(),
            base_url: "https://ilinkai.weixin.qq.com".into(),
        };
        let json = evt.to_json_data();
        assert!(json.contains("accountId"));
        assert!(json.contains("acc_1"));
        assert!(json.contains("botToken"));
        assert!(json.contains("tok_1"));
        assert!(json.contains("baseUrl"));
    }

    #[test]
    fn login_event_error_json() {
        let evt = WeixinLoginEvent::Error("timeout".into());
        let json = evt.to_json_data();
        assert!(json.contains(r#""message":"timeout"#));
    }

    #[test]
    fn to_ws_payload_phases() {
        // qr → { phase: "qr", qrcodeData }
        let qr = WeixinLoginEvent::Qr("https://example.com/qr.png".into()).to_ws_payload();
        assert_eq!(qr["phase"], "qr");
        assert_eq!(qr["qrcodeData"], "https://example.com/qr.png");

        // scanned → { phase: "scanned" }
        let scanned = WeixinLoginEvent::Scanned.to_ws_payload();
        assert_eq!(scanned["phase"], "scanned");

        // done → { phase: "done", accountId, botToken, baseUrl }
        let done = WeixinLoginEvent::Done {
            account_id: "acc_1".into(),
            bot_token: "tok_1".into(),
            base_url: "https://ilinkai.weixin.qq.com".into(),
        }
        .to_ws_payload();
        assert_eq!(done["phase"], "done");
        assert_eq!(done["accountId"], "acc_1");
        assert_eq!(done["botToken"], "tok_1");
        assert_eq!(done["baseUrl"], "https://ilinkai.weixin.qq.com");

        // error → { phase: "error", message }
        let err = WeixinLoginEvent::Error("QR code expired".into()).to_ws_payload();
        assert_eq!(err["phase"], "error");
        assert_eq!(err["message"], "QR code expired");
    }

    #[test]
    fn default_constants() {
        assert_eq!(QR_POLL_INTERVAL, Duration::from_secs(2));
        assert_eq!(QR_LOGIN_TIMEOUT, Duration::from_secs(300));
    }
}
