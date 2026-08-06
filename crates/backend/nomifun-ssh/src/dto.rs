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

// ── `~/.ssh/config` import ──────────────────────────────────────────────

/// The aliases the user confirmed for import.
///
/// Aliases *only*, on purpose. The server re-reads its own `~/.ssh/config` to
/// learn what each one points at, so the request can never name a file for the
/// server to read: an import that accepted `identityFile` paths would be an
/// arbitrary-file-read primitive wearing a feature's clothes.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportSshHostsRequest {
    pub aliases: Vec<String>,
}

/// One host the import created.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedSshHost {
    /// The `Host` alias it came from, which is also its display name.
    pub alias: String,
    pub ssh_host_id: String,
    /// The host was created but has no credential stored — the config named no
    /// identity file, or the one it named held no readable private key. The row
    /// is still useful (all the coordinates are right); it just cannot connect
    /// until someone opens it and supplies a secret, and saying so is the
    /// difference between an honest import and a book full of dead hosts.
    pub needs_credential: bool,
}

/// Why a requested alias produced no host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SshImportSkipReason {
    /// A saved host already uses this display name.
    DuplicateName,
    /// A saved host already has this `user@host:port`.
    DuplicateEndpoint,
    /// The config no longer offers this alias (it changed since the scan).
    NotInConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedSshHost {
    pub alias: String,
    pub reason: SshImportSkipReason,
}

/// What an import did, per alias. A report: no credential material here, only
/// names and ids.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshImportResult {
    pub imported: Vec<ImportedSshHost>,
    pub skipped: Vec<SkippedSshHost>,
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
    /// Whether a retry could plausibly bring this link back. `Some` only for a
    /// dropped link, and — like `reaped` — never omitted there.
    ///
    /// This is the difference between "wait, it is coming back" and "a person has
    /// to change something" (a rejected credential, a host key that changed). The
    /// client needs that difference to decide whether to offer a call to action,
    /// and `detail` is free-form operator text: string-matching it for the answer
    /// is how "authentication failed" ends up rendered as a transient blip.
    pub retryable: Option<bool>,
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
            retryable: None,
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
            SshLinkState::Dropped { detail, retryable } => {
                event.detail = Some(detail.clone());
                event.retryable = Some(*retryable);
            }
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
        assert_eq!(idle.retryable, None);

        let connecting = event_of(&SshLinkState::Connecting { attempt: 3 });
        assert_eq!(connecting.state, SshLinkPhase::Connecting);
        assert_eq!(connecting.attempt, 3);
        assert_eq!(connecting.next_retry_in_ms, None);
        assert_eq!(connecting.reaped, None);
        assert_eq!(connecting.retryable, None);

        let connected = event_of(&SshLinkState::Connected {
            fingerprint: Some("SHA256:abc".to_string()),
        });
        assert_eq!(connected.state, SshLinkPhase::Connected);
        assert_eq!(connected.attempt, 0);
        assert_eq!(connected.host_fingerprint.as_deref(), Some("SHA256:abc"));
        assert_eq!(connected.reaped, None);
        assert_eq!(connected.retryable, None);

        let degraded = event_of(&SshLinkState::Degraded {
            detail: "shell stalled".to_string(),
        });
        assert_eq!(degraded.state, SshLinkPhase::Degraded);
        assert_eq!(degraded.attempt, 0);
        assert_eq!(degraded.detail.as_deref(), Some("shell stalled"));
        assert_eq!(degraded.reaped, None);
        assert_eq!(degraded.retryable, None);

        let reconnecting = event_of(&SshLinkState::Reconnecting {
            attempt: 4,
            next_retry_in_ms: 8_000,
        });
        assert_eq!(reconnecting.state, SshLinkPhase::Reconnecting);
        assert_eq!(reconnecting.attempt, 4);
        assert_eq!(reconnecting.next_retry_in_ms, Some(8_000));
        assert_eq!(reconnecting.reaped, None);
        assert_eq!(reconnecting.retryable, None);

        // The two drops differ only in `retryable`, and that bit is the whole
        // difference between "wait" and "go fix your credentials" on the client.
        // Guessing it back out of `detail` is what the flag exists to prevent.
        let dropped = event_of(&SshLinkState::Dropped {
            detail: "authentication failed".to_string(),
            retryable: false,
        });
        assert_eq!(dropped.state, SshLinkPhase::Dropped);
        assert_eq!(dropped.attempt, 0);
        assert_eq!(dropped.next_retry_in_ms, None);
        assert_eq!(dropped.detail.as_deref(), Some("authentication failed"));
        assert_eq!(dropped.reaped, None);
        assert_eq!(dropped.retryable, Some(false));

        let retryable_drop = event_of(&SshLinkState::Dropped {
            detail: "the ssh transport went away".to_string(),
            retryable: true,
        });
        assert_eq!(retryable_drop.state, SshLinkPhase::Dropped);
        assert_eq!(retryable_drop.retryable, Some(true));
        assert_eq!(retryable_drop.reaped, None);

        let reaped = event_of(&SshLinkState::Closed {
            teardown: SshTeardown::Reaped {
                detail: "remote shell closed with exit status 0".to_string(),
            },
        });
        assert_eq!(reaped.state, SshLinkPhase::Closed);
        assert_eq!(reaped.reaped, Some(true));
        assert_eq!(reaped.retryable, None);
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
        assert_eq!(lost.retryable, None);

        let already_down = event_of(&SshLinkState::Closed {
            teardown: SshTeardown::AlreadyDown {
                detail: "link was already down".to_string(),
            },
        });
        assert_eq!(already_down.state, SshLinkPhase::Closed);
        assert_eq!(already_down.reaped, Some(false));
        assert_eq!(already_down.retryable, None);
    }

    #[test]
    fn a_dropped_link_always_states_its_retryability() {
        // `skip_serializing_if` on `retryable` would let a drop reach the client
        // with the field absent, and an absent flag reads as "unknown" — which is
        // exactly the guess the client is forbidden to make.
        for retryable in [true, false] {
            let value = serde_json::to_value(event_of(&SshLinkState::Dropped {
                detail: "down".to_string(),
                retryable,
            }))
            .expect("serialize");
            assert_eq!(
                value["retryable"],
                serde_json::Value::Bool(retryable),
                "a dropped link must carry its retryability: {value}"
            );
        }
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
            "retryable",
            "changedAt",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(value["state"], "connected");
        assert_eq!(value["sshHostId"], "host-1");
        assert_eq!(value["conversationId"], "conv-1");
    }
}
