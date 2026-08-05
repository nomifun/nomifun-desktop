//! The lifecycle of one pooled SSH link, as data.
//!
//! Everything here is pure: no sockets, no clock, no logging. The pool owns a
//! `watch` of [`SshLinkState`] and is the only writer; the wire projection, the
//! header pill and the teardown report all read that one value, so "what the
//! socket is doing" and "what the operator sees" cannot drift apart.
use std::time::Duration;

use nomi_ssh::connection::SshError;
use nomi_ssh::shell::ShellCloseProof;
use serde::{Deserialize, Serialize};

/// First retry waits a second — long enough that a server restarting its sshd
/// is not hammered, short enough that a blip is invisible to the operator.
pub const SSH_RECONNECT_INITIAL_BACKOFF_MS: u64 = 1_000;
/// The ceiling of the doubling ladder. A minute is the longest an idle link may
/// stay silently broken before the next attempt refreshes its status.
pub const SSH_RECONNECT_MAX_BACKOFF_MS: u64 = 60_000;
/// After this many consecutive failures the link stays `Dropped` and waits for
/// a human. Retrying forever hides a real misconfiguration behind a spinner.
pub const SSH_RECONNECT_MAX_ATTEMPTS: u32 = 10;
/// How often the supervisor asks the transport whether it is still there. The
/// probe is a local check, so it never competes with a long command for the
/// shell lock.
pub const SSH_LIVENESS_POLL_INTERVAL: Duration = Duration::from_secs(15);
/// How long a close may spend collecting exit evidence before giving up and
/// reporting the teardown as unproven.
pub const SSH_CLOSE_BUDGET: Duration = Duration::from_secs(5);

/// How long to wait before retry number `attempt` (1-based): the initial backoff
/// doubled once per previous attempt, capped at [`SSH_RECONNECT_MAX_BACKOFF_MS`].
pub fn reconnect_delay(attempt: u32) -> Duration {
    let doublings = attempt.saturating_sub(1);
    let ms = match SSH_RECONNECT_INITIAL_BACKOFF_MS.checked_shl(doublings) {
        Some(ms) => ms.min(SSH_RECONNECT_MAX_BACKOFF_MS),
        // Beyond 63 doublings the shift overflows; the cap is the answer anyway.
        None => SSH_RECONNECT_MAX_BACKOFF_MS,
    };
    Duration::from_millis(ms)
}

/// The coarse, machine-readable half of a link state — what the UI colours by.
/// Payload-free on purpose: clients pick a colour from this and never string-
/// match on `detail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SshLinkPhase {
    Idle,
    Connecting,
    Connected,
    Degraded,
    Reconnecting,
    Dropped,
    Closed,
}

/// What we can prove about a link that is now gone.
///
/// `Reaped` is a claim about the remote side, so it may only be built from
/// evidence — see [`SshTeardown::from_proof`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshTeardown {
    /// The remote shell ended and said how (exit status or signal).
    Reaped { detail: String },
    /// We let go of the link without proof that the remote shell died. This is a
    /// teardown *failure*, not a quieter success.
    Lost { detail: String },
    /// There was nothing left to close: the link had already dropped.
    AlreadyDown { detail: String },
}

impl SshTeardown {
    /// The only path to [`SshTeardown::Reaped`]. An unproven close keeps the
    /// proof's own errors in its detail so the operator learns *why* it is
    /// unproven instead of just seeing "lost".
    pub fn from_proof(proof: &ShellCloseProof) -> Self {
        if proof.is_reaped() {
            let mut evidence = Vec::new();
            if let Some(code) = proof.exit_status {
                evidence.push(format!("exit status {code}"));
            }
            if let Some(signal) = &proof.exit_signal {
                evidence.push(format!("exit signal {signal}"));
            }
            return SshTeardown::Reaped {
                detail: format!("remote shell closed with {}", evidence.join(", ")),
            };
        }

        let why = if !proof.errors.is_empty() {
            proof.errors.join("; ")
        } else if proof.channel_closed {
            "the channel closed without an exit status or signal".to_string()
        } else {
            "the channel never closed".to_string()
        };
        SshTeardown::Lost {
            detail: format!("no exit evidence: {why}"),
        }
    }
}

/// The state of one pooled link. The pool's `watch` of this value is the single
/// source of truth for the reconnect ladder, the realtime event and the REST
/// snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshLinkState {
    /// Known to the pool but never dialled (or released back to nothing).
    Idle,
    Connecting { attempt: u32 },
    Connected { fingerprint: Option<String> },
    /// The transport is up but the shell is not answering; recoverable by
    /// reopening the channel rather than redialling.
    Degraded { detail: String },
    Reconnecting { attempt: u32, next_retry_in_ms: u64 },
    /// Down and not currently retrying. `retryable` is false for the failures a
    /// retry cannot fix (rejected credentials, a changed host key).
    Dropped { detail: String, retryable: bool },
    Closed { teardown: SshTeardown },
}

impl SshLinkState {
    /// Total by construction — no catch-all arm, so a new variant is a compile
    /// error here (and therefore in the wire contract) rather than a silent
    /// default phase on the operator's screen.
    pub fn phase(&self) -> SshLinkPhase {
        match self {
            SshLinkState::Idle => SshLinkPhase::Idle,
            SshLinkState::Connecting { .. } => SshLinkPhase::Connecting,
            SshLinkState::Connected { .. } => SshLinkPhase::Connected,
            SshLinkState::Degraded { .. } => SshLinkPhase::Degraded,
            SshLinkState::Reconnecting { .. } => SshLinkPhase::Reconnecting,
            SshLinkState::Dropped { .. } => SshLinkPhase::Dropped,
            SshLinkState::Closed { .. } => SshLinkPhase::Closed,
        }
    }
}

/// Whether redialling after `err` could plausibly succeed.
///
/// Credential and host-key rejections are terminal: replaying a rejected
/// credential only walks the account into a server-side lockout, and a host key
/// that changed under us must never be re-accepted without a human looking at
/// it. Matched exhaustively so a new transport error has to be classified.
pub fn is_retryable(err: &SshError) -> bool {
    match err {
        SshError::Unreachable(_) | SshError::Disconnected(_) | SshError::Protocol(_) => true,
        SshError::AuthFailed(_)
        | SshError::HostKeyUnknown { .. }
        | SshError::HostKeyChanged { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_delay_doubles_and_caps_at_60s() {
        let expected_ms = [
            1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 60_000, 60_000, 60_000, 60_000, 60_000,
            60_000,
        ];
        for (i, want) in expected_ms.iter().enumerate() {
            let attempt = (i + 1) as u32;
            assert_eq!(
                reconnect_delay(attempt),
                std::time::Duration::from_millis(*want),
                "attempt {attempt}"
            );
        }
    }

    #[test]
    fn max_attempts_and_backoff_constants_are_pinned() {
        assert_eq!(SSH_RECONNECT_INITIAL_BACKOFF_MS, 1_000);
        assert_eq!(SSH_RECONNECT_MAX_BACKOFF_MS, 60_000);
        assert_eq!(SSH_RECONNECT_MAX_ATTEMPTS, 10);
        assert_eq!(SSH_LIVENESS_POLL_INTERVAL, std::time::Duration::from_secs(15));
        assert_eq!(SSH_CLOSE_BUDGET, std::time::Duration::from_secs(5));
    }

    #[test]
    fn teardown_is_reaped_only_with_exit_evidence() {
        let unproven = ShellCloseProof {
            eof_sent: true,
            channel_closed: true,
            errors: vec!["shell busy; close proof unavailable".to_string()],
            ..ShellCloseProof::default()
        };
        match SshTeardown::from_proof(&unproven) {
            SshTeardown::Lost { detail } => {
                assert!(
                    detail.contains("shell busy; close proof unavailable"),
                    "an unproven close must say why: {detail}"
                );
            }
            other => panic!("a close without exit evidence must be lost, got {other:?}"),
        }

        let proven = ShellCloseProof {
            eof_sent: true,
            channel_closed: true,
            exit_status: Some(0),
            ..ShellCloseProof::default()
        };
        match SshTeardown::from_proof(&proven) {
            SshTeardown::Reaped { detail } => assert!(detail.contains('0'), "{detail}"),
            other => panic!("channel close + exit status is reaped, got {other:?}"),
        }
    }

    #[test]
    fn phase_of_every_state_is_total() {
        assert_eq!(SshLinkState::Idle.phase(), SshLinkPhase::Idle);
        assert_eq!(
            SshLinkState::Connecting { attempt: 1 }.phase(),
            SshLinkPhase::Connecting
        );
        assert_eq!(
            SshLinkState::Connected { fingerprint: None }.phase(),
            SshLinkPhase::Connected
        );
        assert_eq!(
            SshLinkState::Degraded {
                detail: "shell stalled".to_string()
            }
            .phase(),
            SshLinkPhase::Degraded
        );
        assert_eq!(
            SshLinkState::Reconnecting {
                attempt: 2,
                next_retry_in_ms: 2_000
            }
            .phase(),
            SshLinkPhase::Reconnecting
        );
        assert_eq!(
            SshLinkState::Dropped {
                detail: "cannot reach host".to_string(),
                retryable: true
            }
            .phase(),
            SshLinkPhase::Dropped
        );
        assert_eq!(
            SshLinkState::Closed {
                teardown: SshTeardown::AlreadyDown {
                    detail: "link was already down".to_string()
                }
            }
            .phase(),
            SshLinkPhase::Closed
        );
    }

    #[test]
    fn host_key_and_auth_failures_are_not_retryable() {
        assert!(!is_retryable(&SshError::AuthFailed("bad password".into())));
        assert!(!is_retryable(&SshError::HostKeyUnknown {
            host: "example:22".into(),
            fingerprint: "SHA256:abc".into()
        }));
        assert!(!is_retryable(&SshError::HostKeyChanged {
            host: "example:22".into(),
            line: 7
        }));

        assert!(is_retryable(&SshError::Unreachable("timeout".into())));
        assert!(is_retryable(&SshError::Disconnected("eof".into())));
        assert!(is_retryable(&SshError::Protocol("kex failed".into())));
    }
}
