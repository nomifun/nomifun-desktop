//! HTTP DTOs for the SSH host book.
//!
//! Request DTOs are `Deserialize`-only with `deny_unknown_fields`. Response
//! DTOs never carry credential plaintext or ciphertext: a stored secret is
//! surfaced only as the masked sentinel `"***"`, which the client uses to know
//! "this secret is set; don't resend it on update". Unlike `remote_agents`'
//! `mask_token`, we deliberately do NOT reveal a last-4 suffix — that would
//! require decrypting every secret just to render a list, a needless exposure.
use nomifun_common::TimestampMs;
use nomifun_db::SshHostRow;
use serde::{Deserialize, Serialize};

/// The masked placeholder for a stored secret. The client omits any credential
/// field still equal to this on update (the value is unchanged).
pub const SECRET_MASK: &str = "***";

/// Owner-visible view of a saved SSH host. No credential material — only
/// whether each secret is set (masked).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostResponse {
    pub ssh_host_id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    /// `Some("***")` if a password is stored, else `None`.
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub certificate: Option<String>,
    pub sudo_password: Option<String>,
    pub host_fingerprint: Option<String>,
    pub status: String,
    pub last_connected_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

fn mask(stored: &Option<String>) -> Option<String> {
    stored.as_ref().map(|_| SECRET_MASK.to_string())
}

impl From<SshHostRow> for SshHostResponse {
    fn from(r: SshHostRow) -> Self {
        SshHostResponse {
            ssh_host_id: r.ssh_host_id,
            name: r.name,
            host: r.host,
            port: r.port,
            username: r.username,
            auth_type: r.auth_type,
            password: mask(&r.password_encrypted),
            private_key: mask(&r.private_key_encrypted),
            passphrase: mask(&r.passphrase_encrypted),
            certificate: mask(&r.certificate_encrypted),
            sudo_password: mask(&r.sudo_password_encrypted),
            host_fingerprint: r.host_fingerprint,
            status: r.status,
            last_connected_at: r.last_connected_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Create-host request. Credential fields are plaintext here (they are
/// encrypted in the service before storage) and never echoed back.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSshHostRequest {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: i64,
    pub username: String,
    /// "password" | "key" | "certificate" | "agent".
    pub auth_type: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub certificate: Option<String>,
    pub sudo_password: Option<String>,
}

/// Update-host request. A `None` field is left unchanged. A credential field
/// equal to the mask (`"***"`) is left unchanged; an empty string clears it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateSshHostRequest {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub username: Option<String>,
    pub auth_type: Option<String>,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub certificate: Option<String>,
    pub sudo_password: Option<String>,
}

fn default_port() -> i64 {
    22
}

/// One link's status as the client sees it. This is the *only* wire shape for
/// link state: the realtime `ssh.status` event and the REST snapshot both carry
/// it, so a reconnecting link cannot look different depending on how the client
/// learned about it.
///
/// camelCase like the rest of this crate's DTOs (not the terminal domain's
/// snake_case) because the `ssh` client namespace brands `sshHostId`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SshStatusEvent {
    pub ssh_host_id: String,
    pub conversation_id: String,
    pub state: crate::state::SshLinkPhase,
    /// Which dial attempt this is; 0 outside `Connecting`/`Reconnecting`.
    pub attempt: u32,
    pub next_retry_in_ms: Option<u64>,
    pub host_fingerprint: Option<String>,
    /// Operator-facing explanation. Carries transport diagnostics only — never
    /// credential material, which is why it is projected from the state's own
    /// `detail` rather than from an error chain that has seen a password.
    pub detail: Option<String>,
    /// Whether the remote shell was proven reaped. `Some` only for a closed
    /// link, and never omitted there: "we don't know" is `Some(false)`, which is
    /// a teardown failure, not an absent field.
    pub reaped: Option<bool>,
    pub changed_at: TimestampMs,
}

impl SshStatusEvent {
    /// Project a link state onto the wire. Total over the state enum so a new
    /// variant cannot ship without deciding what the client should see.
    pub fn from_state(
        ssh_host_id: &str,
        conversation_id: &str,
        state: &crate::state::SshLinkState,
    ) -> Self {
        use crate::state::{SshLinkState, SshTeardown};

        let mut event = SshStatusEvent {
            ssh_host_id: ssh_host_id.to_string(),
            conversation_id: conversation_id.to_string(),
            state: state.phase(),
            attempt: 0,
            next_retry_in_ms: None,
            host_fingerprint: None,
            detail: None,
            reaped: None,
            changed_at: nomifun_common::now_ms(),
        };
        match state {
            SshLinkState::Idle => {}
            SshLinkState::Connecting { attempt } => event.attempt = *attempt,
            SshLinkState::Connected { fingerprint } => {
                event.host_fingerprint = fingerprint.clone();
            }
            SshLinkState::Degraded { detail } => event.detail = Some(detail.clone()),
            SshLinkState::Reconnecting {
                attempt,
                next_retry_in_ms,
            } => {
                event.attempt = *attempt;
                event.next_retry_in_ms = Some(*next_retry_in_ms);
            }
            SshLinkState::Dropped { detail, .. } => event.detail = Some(detail.clone()),
            SshLinkState::Closed { teardown } => {
                let (reaped, detail) = match teardown {
                    SshTeardown::Reaped { detail } => (true, detail),
                    SshTeardown::Lost { detail } => (false, detail),
                    SshTeardown::AlreadyDown { detail } => (false, detail),
                };
                event.reaped = Some(reaped);
                event.detail = Some(detail.clone());
            }
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{SshLinkPhase, SshLinkState, SshTeardown};

    fn event_of(state: &SshLinkState) -> SshStatusEvent {
        SshStatusEvent::from_state("host-1", "conv-1", state)
    }

    #[test]
    fn status_event_projects_every_link_state() {
        let idle = event_of(&SshLinkState::Idle);
        assert_eq!(idle.state, SshLinkPhase::Idle);
        assert_eq!(idle.attempt, 0);
        assert_eq!(idle.next_retry_in_ms, None);
        assert_eq!(idle.reaped, None);

        let connecting = event_of(&SshLinkState::Connecting { attempt: 3 });
        assert_eq!(connecting.state, SshLinkPhase::Connecting);
        assert_eq!(connecting.attempt, 3);
        assert_eq!(connecting.next_retry_in_ms, None);
        assert_eq!(connecting.reaped, None);

        let connected = event_of(&SshLinkState::Connected {
            fingerprint: Some("SHA256:abc".to_string()),
        });
        assert_eq!(connected.state, SshLinkPhase::Connected);
        assert_eq!(connected.attempt, 0);
        assert_eq!(connected.host_fingerprint.as_deref(), Some("SHA256:abc"));
        assert_eq!(connected.reaped, None);

        let degraded = event_of(&SshLinkState::Degraded {
            detail: "shell stalled".to_string(),
        });
        assert_eq!(degraded.state, SshLinkPhase::Degraded);
        assert_eq!(degraded.attempt, 0);
        assert_eq!(degraded.detail.as_deref(), Some("shell stalled"));
        assert_eq!(degraded.reaped, None);

        let reconnecting = event_of(&SshLinkState::Reconnecting {
            attempt: 4,
            next_retry_in_ms: 8_000,
        });
        assert_eq!(reconnecting.state, SshLinkPhase::Reconnecting);
        assert_eq!(reconnecting.attempt, 4);
        assert_eq!(reconnecting.next_retry_in_ms, Some(8_000));
        assert_eq!(reconnecting.reaped, None);

        let dropped = event_of(&SshLinkState::Dropped {
            detail: "authentication failed".to_string(),
            retryable: false,
        });
        assert_eq!(dropped.state, SshLinkPhase::Dropped);
        assert_eq!(dropped.attempt, 0);
        assert_eq!(dropped.next_retry_in_ms, None);
        assert_eq!(dropped.detail.as_deref(), Some("authentication failed"));
        assert_eq!(dropped.reaped, None);

        let reaped = event_of(&SshLinkState::Closed {
            teardown: SshTeardown::Reaped {
                detail: "remote shell closed with exit status 0".to_string(),
            },
        });
        assert_eq!(reaped.state, SshLinkPhase::Closed);
        assert_eq!(reaped.reaped, Some(true));
        assert_eq!(
            reaped.detail.as_deref(),
            Some("remote shell closed with exit status 0")
        );

        let lost = event_of(&SshLinkState::Closed {
            teardown: SshTeardown::Lost {
                detail: "no exit evidence: shell busy".to_string(),
            },
        });
        assert_eq!(lost.state, SshLinkPhase::Closed);
        assert_eq!(lost.reaped, Some(false));

        let already_down = event_of(&SshLinkState::Closed {
            teardown: SshTeardown::AlreadyDown {
                detail: "link was already down".to_string(),
            },
        });
        assert_eq!(already_down.state, SshLinkPhase::Closed);
        assert_eq!(already_down.reaped, Some(false));
    }

    #[test]
    fn status_event_serializes_camel_case() {
        let event = event_of(&SshLinkState::Connected {
            fingerprint: Some("SHA256:abc".to_string()),
        });
        let value = serde_json::to_value(&event).expect("serialize");
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        // Sorted, because serde_json's map ordering is not part of the contract —
        // the exact key set is.
        let mut expected = vec![
            "sshHostId",
            "conversationId",
            "state",
            "attempt",
            "nextRetryInMs",
            "hostFingerprint",
            "detail",
            "reaped",
            "changedAt",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(value["state"], "connected");
        assert_eq!(value["sshHostId"], "host-1");
        assert_eq!(value["conversationId"], "conv-1");
    }
}
