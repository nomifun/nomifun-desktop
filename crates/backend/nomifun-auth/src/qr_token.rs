use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use nomifun_common::AppError;
use nomifun_api_types::{DevicePairingStatus, DevicePairingTicket};

/// QR token time-to-live: 5 minutes.
const QR_TOKEN_TTL: Duration = Duration::from_secs(5 * 60);

/// Random token length in bytes (produces 64-char hex string).
const QR_TOKEN_BYTES: usize = 32;

/// Internal data for a QR login token.
struct QrTokenData {
    created_at: Instant,
    used: bool,
}

impl QrTokenData {
    fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.created_at) >= QR_TOKEN_TTL
    }
}

/// In-memory QR login token store with automatic expiration.
///
/// Tokens are one-time-use and expire after 5 minutes.
/// Thread-safe via `DashMap`.
pub struct QrTokenStore {
    tokens: DashMap<String, QrTokenData>,
    device_tickets: DashMap<String, DeviceTicketData>,
    /// Serializes supersede+insert so concurrent generates cannot leave two
    /// Ready tickets for one installation.
    device_ticket_generation: std::sync::Mutex<()>,
}

/// A device ticket only reports `used` after the credential was issued;
/// a claimed-but-unfinished redemption is `redeeming` and an explicitly
/// failed issuance is `failed` with a stable reason code.
enum DeviceTicketState {
    Ready,
    Redeeming,
    Used { device_id: String },
    Failed { code: &'static str },
}

struct DeviceTicketData {
    ticket_id: String,
    created_at: Instant,
    expires_at_ms: i64,
    state: DeviceTicketState,
}

/// Stable error codes for the Mobile pairing flow.
#[derive(Debug)]
pub struct DevicePairingError(pub &'static str);

impl axum::response::IntoResponse for DevicePairingError {
    fn into_response(self) -> axum::response::Response {
        let message = match self.0 {
            "DEVICE_PAIRING_USED" => "This pairing code has already been used. Create a new code on Desktop.",
            "DEVICE_PAIRING_EXPIRED" => "This pairing code has expired. Create a new code on Desktop.",
            _ => "This is not a valid Mobile pairing code. Create a code in Connect phone on Desktop.",
        };
        (axum::http::StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"success":false,"code":self.0,"error":message}))).into_response()
    }
}

impl Default for QrTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl QrTokenStore {
    pub fn new() -> Self {
        Self { tokens: DashMap::new(), device_tickets: DashMap::new(), device_ticket_generation: std::sync::Mutex::new(()) }
    }

    /// One installation keeps a single active code: every earlier still-Ready
    /// ticket is removed before the new one is inserted. LAN and relay codes
    /// share this store; claimed (Redeeming/Used/Failed) receipts stay until
    /// the regular cleanup so Desktop can explain what happened to them.
    pub fn generate_device_ticket(&self) -> DevicePairingTicket {
        let _generation = self.device_ticket_generation.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        self.device_tickets.retain(|_, data| !matches!(data.state, DeviceTicketState::Ready));
        let ticket = DevicePairingTicket {
            ticket_id: crate::generate_random_hex_secret(),
            qr_token: crate::generate_random_hex_secret(),
            expires_at_ms: nomifun_common::now_ms().saturating_add(QR_TOKEN_TTL.as_millis() as i64),
        };
        self.device_tickets.insert(ticket.qr_token.clone(), DeviceTicketData {
            ticket_id: ticket.ticket_id.clone(), created_at: Instant::now(),
            expires_at_ms: ticket.expires_at_ms, state: DeviceTicketState::Ready,
        });
        ticket
    }

    pub fn device_ticket_status(&self, ticket_id: &str) -> Option<DevicePairingStatus> {
        let ticket = self.device_tickets.iter().find(|entry| entry.value().ticket_id == ticket_id)?;
        let data = ticket.value();
        let (state, device_id, failure_code) = match &data.state {
            DeviceTicketState::Ready if data.created_at.elapsed() >= QR_TOKEN_TTL => ("expired", None, None),
            DeviceTicketState::Ready => ("ready", None, None),
            DeviceTicketState::Redeeming => ("redeeming", None, None),
            DeviceTicketState::Used { device_id } => ("used", Some(device_id.clone()), None),
            DeviceTicketState::Failed { code } => ("failed", None, Some(code.to_string())),
        };
        Some(DevicePairingStatus {
            ticket_id: data.ticket_id.clone(),
            state: state.into(),
            device_id, failure_code, expires_at_ms: data.expires_at_ms,
        })
    }

    /// A Ready, unexpired ticket moves to Redeeming so a second consumer (or a
    /// retry of a failed/completed redemption) always sees USED.
    pub fn consume_device_ticket(&self, qr_token: &str) -> Result<(), DevicePairingError> {
        let Some(mut data) = self.device_tickets.get_mut(qr_token) else { return Err(DevicePairingError("DEVICE_PAIRING_INVALID")); };
        match data.state {
            DeviceTicketState::Ready => {
                if data.created_at.elapsed() >= QR_TOKEN_TTL { return Err(DevicePairingError("DEVICE_PAIRING_EXPIRED")); }
                data.state = DeviceTicketState::Redeeming;
                Ok(())
            }
            _ => Err(DevicePairingError("DEVICE_PAIRING_USED")),
        }
    }

    pub fn complete_device_ticket(&self, qr_token: &str, device_id: &str) {
        if let Some(mut data) = self.device_tickets.get_mut(qr_token) {
            data.state = DeviceTicketState::Used { device_id: device_id.to_owned() };
        }
    }

    pub fn fail_device_ticket(&self, qr_token: &str, code: &'static str) {
        if let Some(mut data) = self.device_tickets.get_mut(qr_token) {
            data.state = DeviceTicketState::Failed { code };
        }
    }

    /// Generate a new QR login token and store it.
    ///
    /// Returns the 64-character hex token string.
    pub fn generate(&self) -> String {
        self.generate_with_expiry().0
    }

    /// Generate a new QR login token and return it along with its expiry timestamp (ms).
    ///
    /// Returns `(token, expires_at_ms)` where `expires_at_ms` is the absolute
    /// Unix time in milliseconds when the token is expected to become invalid.
    /// Enforcement uses monotonic elapsed time so wall-clock changes cannot
    /// extend the token's lifetime.
    pub fn generate_with_expiry(&self) -> (String, i64) {
        let mut buf = [0u8; QR_TOKEN_BYTES];
        getrandom::getrandom(&mut buf).expect("OS entropy source unavailable");

        let mut token = String::with_capacity(QR_TOKEN_BYTES * 2);
        for byte in buf {
            let _ = write!(token, "{byte:02x}");
        }

        let created_at_ms = nomifun_common::now_ms();
        self.tokens.insert(
            token.clone(),
            QrTokenData {
                created_at: Instant::now(),
                used: false,
            },
        );

        (token, created_at_ms.saturating_add(QR_TOKEN_TTL.as_millis() as i64))
    }

    /// Validate and consume a QR token (one-time use).
    ///
    /// Checks existence, expiry (5 min), and used status atomically.
    pub fn validate_and_consume(&self, token: &str) -> Result<(), AppError> {
        let mut entry = self
            .tokens
            .get_mut(token)
            .ok_or_else(|| AppError::Unauthorized("Invalid QR token".into()))?;

        if entry.used {
            return Err(AppError::Unauthorized("QR token already used".into()));
        }

        if entry.is_expired(Instant::now()) {
            return Err(AppError::Unauthorized("QR token expired".into()));
        }

        entry.used = true;
        Ok(())
    }

    /// Remove expired tokens to prevent unbounded memory growth.
    pub fn cleanup(&self) {
        let now = Instant::now();
        self.tokens
            .retain(|_, data| !data.is_expired(now));
        // Keep the used/expired receipt briefly so the Desktop can explain it.
        self.device_tickets.retain(|_, data| now.duration_since(data.created_at) < QR_TOKEN_TTL * 2);
    }

    /// Start a background task that cleans up expired tokens periodically.
    /// Stops on the next tick after the last owner drops the store.
    pub fn start_cleanup_task(self: &Arc<Self>, interval: Duration) {
        let store = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let Some(store) = store.upgrade() else {
                    break;
                };
                store.cleanup();
            }
        });
    }

    /// Number of stored tokens (for monitoring/testing).
    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_64_hex_chars() {
        let store = QrTokenStore::new();
        let token = store.generate();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_produces_unique_tokens() {
        let store = QrTokenStore::new();
        let t1 = store.generate();
        let t2 = store.generate();
        assert_ne!(t1, t2);
    }

    #[test]
    fn generate_increments_count() {
        let store = QrTokenStore::new();
        assert_eq!(store.token_count(), 0);
        store.generate();
        assert_eq!(store.token_count(), 1);
        store.generate();
        assert_eq!(store.token_count(), 2);
    }

    #[test]
    fn validate_and_consume_valid_token() {
        let store = QrTokenStore::new();
        let token = store.generate();
        assert!(store.validate_and_consume(&token).is_ok());
    }

    #[test]
    fn validate_nonexistent_token_fails() {
        let store = QrTokenStore::new();
        let err = store.validate_and_consume("nonexistent").unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn validate_already_used_token_fails() {
        let store = QrTokenStore::new();
        let token = store.generate();
        store.validate_and_consume(&token).unwrap();

        let err = store.validate_and_consume(&token).unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn validate_expired_token_fails() {
        let store = QrTokenStore::new();
        // Manually insert an expired token
        store.tokens.insert(
            "expired_token".to_owned(),
            QrTokenData {
                created_at: Instant::now() - QR_TOKEN_TTL,
                used: false,
            },
        );

        let err = store.validate_and_consume("expired_token").unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn cleanup_removes_expired_tokens() {
        let store = QrTokenStore::new();
        // Insert an expired token
        store.tokens.insert(
            "old".to_owned(),
            QrTokenData {
                created_at: Instant::now() - QR_TOKEN_TTL,
                used: false,
            },
        );
        // Insert a fresh token
        store.generate();
        assert_eq!(store.token_count(), 2);

        store.cleanup();
        assert_eq!(store.token_count(), 1);
    }

    #[test]
    fn cleanup_keeps_fresh_tokens() {
        let store = QrTokenStore::new();
        store.generate();
        store.cleanup();
        assert_eq!(store.token_count(), 1);
    }

    #[test]
    fn expiry_includes_the_exact_deadline() {
        let created_at = Instant::now();
        let data = QrTokenData { created_at, used: false };
        assert!(!data.is_expired(created_at + QR_TOKEN_TTL - Duration::from_nanos(1)));
        assert!(data.is_expired(created_at + QR_TOKEN_TTL));
    }

    #[test]
    fn concurrent_consumers_have_exactly_one_winner() {
        let store = QrTokenStore::new();
        let token = store.generate();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8).map(|_| scope.spawn(|| {
                barrier.wait();
                store.validate_and_consume(&token).is_ok()
            })).collect();
            let winners = workers.into_iter().filter_map(|worker| {
                worker.join().unwrap().then_some(())
            }).count();
            assert_eq!(winners, 1);
        });
    }

    #[test]
    fn device_invitation_has_expiry_and_exactly_one_consumer() {
        let store = QrTokenStore::new();
        let ticket = store.generate_device_ticket();
        assert!(store.validate_and_consume(&ticket.qr_token).is_err(), "Mobile invitation must not log into a browser");
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8).map(|_| scope.spawn(|| {
                barrier.wait();
                store.consume_device_ticket(&ticket.qr_token).is_ok()
            })).collect();
            assert_eq!(workers.into_iter().filter_map(|worker| worker.join().unwrap().then_some(())).count(), 1);
        });
        let ticket = store.generate_device_ticket();
        store.device_tickets.get_mut(&ticket.qr_token).unwrap().created_at = Instant::now() - QR_TOKEN_TTL;
        assert_eq!(store.consume_device_ticket(&ticket.qr_token).unwrap_err().0, "DEVICE_PAIRING_EXPIRED");
        assert_eq!(store.device_ticket_status(&ticket.ticket_id).unwrap().state, "expired");
    }

    #[test]
    fn device_ticket_claim_complete_fail_report_exact_statuses() {
        let store = QrTokenStore::new();
        let ticket = store.generate_device_ticket();
        assert_eq!(store.device_ticket_status(&ticket.ticket_id).unwrap().state, "ready");
        store.consume_device_ticket(&ticket.qr_token).unwrap();
        assert_eq!(store.device_ticket_status(&ticket.ticket_id).unwrap().state, "redeeming");
        assert_eq!(store.consume_device_ticket(&ticket.qr_token).unwrap_err().0, "DEVICE_PAIRING_USED");
        store.complete_device_ticket(&ticket.qr_token, "device-1");
        let status = store.device_ticket_status(&ticket.ticket_id).unwrap();
        assert_eq!(status.state, "used");
        assert_eq!(status.device_id.as_deref(), Some("device-1"));
        assert_eq!(status.failure_code, None);
    }

    #[test]
    fn device_ticket_failed_and_redeeming_receipts_cannot_be_reclaimed() {
        let store = QrTokenStore::new();
        let ticket = store.generate_device_ticket();
        store.consume_device_ticket(&ticket.qr_token).unwrap();
        store.fail_device_ticket(&ticket.qr_token, "DEVICE_PAIRING_FAILED");
        let status = store.device_ticket_status(&ticket.ticket_id).unwrap();
        assert_eq!(status.state, "failed");
        assert_eq!(status.failure_code.as_deref(), Some("DEVICE_PAIRING_FAILED"));
        assert_eq!(status.device_id, None);
        assert_eq!(store.consume_device_ticket(&ticket.qr_token).unwrap_err().0, "DEVICE_PAIRING_USED");
    }

    #[test]
    fn a_new_ticket_supersedes_only_ready_ones() {
        let store = QrTokenStore::new();
        let a = store.generate_device_ticket();
        let b = store.generate_device_ticket();
        assert_eq!(store.consume_device_ticket(&a.qr_token).unwrap_err().0, "DEVICE_PAIRING_INVALID");
        assert!(store.device_ticket_status(&a.ticket_id).is_none());
        store.consume_device_ticket(&b.qr_token).unwrap();
        store.complete_device_ticket(&b.qr_token, "kept");
        let c = store.generate_device_ticket();
        let used = store.device_ticket_status(&b.ticket_id).unwrap();
        assert_eq!(used.state, "used");
        assert_eq!(used.device_id.as_deref(), Some("kept"));
        assert!(store.consume_device_ticket(&c.qr_token).is_ok());
    }

    #[tokio::test]
    async fn background_cleanup_expires_tokens_without_retaining_store() {
        let store = Arc::new(QrTokenStore::new());
        store.tokens.insert("expired".into(), QrTokenData {
            created_at: Instant::now() - QR_TOKEN_TTL,
            used: false,
        });
        let fresh = store.generate();
        let weak = Arc::downgrade(&store);
        store.start_cleanup_task(Duration::from_secs(60));
        // Tokio intervals tick immediately; no wall-clock sleep is needed.
        tokio::time::timeout(Duration::from_secs(1), async {
            while store.token_count() != 1 {
                tokio::task::yield_now().await;
            }
        }).await.expect("cleanup did not remove the expired token");
        assert!(store.validate_and_consume(&fresh).is_ok());
        drop(store);
        assert!(weak.upgrade().is_none(), "cleanup retained the store between ticks");
    }
}
