//! Shared fixtures for the connection-pool integration tests: a throwaway sshd
//! ([`sshd`]) and a pool wired to an in-memory host book plus a recording
//! realtime sink.
//!
//! Not every test binary uses every helper, so the module allows dead code — the
//! alternative is a different `#[allow]` in each binary.
#![allow(dead_code)]

pub mod sshd;

use std::sync::{Arc, Mutex};

use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::UserEventSink;
use nomifun_ssh::dto::CreateSshHostRequest;
use nomifun_ssh::{PoolTuning, SshConnectionPool, SshEventEmitter, SshHostService};
use nomifun_common::SshHostId;

/// Captures owner-scoped realtime deliveries so a test can assert what the
/// operator's browser would have received. Same shape as the emitter's own unit
/// test in `src/events.rs`.
#[derive(Default)]
pub struct RecordingUserEvents {
    deliveries: Mutex<Vec<(String, WebSocketMessage<serde_json::Value>)>>,
}

impl UserEventSink for RecordingUserEvents {
    fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
        self.deliveries
            .lock()
            .unwrap()
            .push((user_id.to_owned(), event));
    }
}

impl RecordingUserEvents {
    /// Every `(recipient, event name)` pair seen so far.
    pub fn addressed(&self) -> Vec<(String, String)> {
        self.deliveries
            .lock()
            .unwrap()
            .iter()
            .map(|(user, event)| (user.clone(), event.name.clone()))
            .collect()
    }

    /// The `state` field of every `ssh.status` payload, in delivery order.
    pub fn status_phases(&self) -> Vec<String> {
        self.deliveries
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, event)| event.name == "ssh.status")
            .map(|(_, event)| event.data["state"].as_str().unwrap_or("?").to_string())
            .collect()
    }

    /// Whether `wanted` appears in order (not necessarily adjacently) in the
    /// emitted phase log.
    ///
    /// The emitted log is the right place to assert a *sequence*: `publish` appends
    /// to it synchronously and it is never coalesced, whereas a `watch` receiver
    /// only ever holds the latest value and will happily hide a phase the pool
    /// really did go through.
    pub fn saw_phases_in_order(&self, wanted: &[&str]) -> bool {
        let mut remaining = wanted.iter();
        let mut looking_for = remaining.next();
        for phase in self.status_phases() {
            if looking_for == Some(&phase.as_str()) {
                looking_for = remaining.next();
            }
        }
        looking_for.is_none()
    }

    /// Wait until `wanted` has appeared in order.
    ///
    /// Polled rather than asserted once, because `publish` updates the link's
    /// `watch` *before* it emits — deliberately, so a client that reacts to an
    /// event and re-reads the snapshot can never see something older than the
    /// event. The cost is that a test which waited on the watch can arrive here a
    /// few microseconds before the matching delivery lands.
    pub async fn await_phases_in_order(&self, wanted: &[&str], budget: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if self.saw_phases_in_order(wanted) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {wanted:?} in order; saw {:?}",
                self.status_phases()
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

/// A pool over an in-memory host book, owned by one seeded user.
pub struct PoolHarness {
    pub pool: SshConnectionPool,
    pub events: Arc<RecordingUserEvents>,
    pub user_id: String,
    /// Held so the in-memory sqlite dataset outlives the test.
    _db: nomifun_db::Database,
}

/// Build a pool whose host book is a fresh in-memory database with one user.
pub async fn harness(known_hosts: std::path::PathBuf, tuning: PoolTuning) -> PoolHarness {
    let db = nomifun_db::init_database_memory().await.expect("db");
    let user_id = nomifun_common::UserId::new().as_str().to_string();
    sqlx::query(
        "INSERT INTO users (user_id, username, password_hash, jwt_secret, created_at, updated_at) \
         VALUES (?, ?, '', '', 0, 0)",
    )
    .bind(&user_id)
    .bind(&user_id)
    .execute(db.pool())
    .await
    .expect("seed user");

    let repo = Arc::new(nomifun_db::SqliteSshHostRepository::new(db.pool().clone()));
    let service = SshHostService::new(repo, [11u8; 32]);
    let events = Arc::new(RecordingUserEvents::default());
    let pool = SshConnectionPool::with_tuning(
        service,
        known_hosts,
        SshEventEmitter::new(events.clone()),
        tuning,
    );
    PoolHarness {
        pool,
        events,
        user_id,
        _db: db,
    }
}

impl PoolHarness {
    pub fn service(&self) -> SshHostService {
        self.pool.host_service()
    }

    /// The raw sqlite pool, for the few assertions that have to look at
    /// `ssh_hosts` columns the service does not expose.
    pub fn service_pool(&self) -> &sqlx::SqlitePool {
        self._db.pool()
    }

    /// Store a host and return its id.
    pub async fn add_host(&self, req: CreateSshHostRequest) -> SshHostId {
        let created = self
            .pool
            .host_service()
            .create(&self.user_id, req)
            .await
            .expect("create host");
        SshHostId::parse(created.ssh_host_id).expect("host id")
    }

    /// A key-auth host pointing at the fixture sshd.
    pub async fn add_fixture_host(&self, sshd: &sshd::TestSshd) -> SshHostId {
        self.add_host(CreateSshHostRequest {
            name: "fixture".into(),
            host: "127.0.0.1".into(),
            port: sshd.port() as i64,
            username: sshd.username.clone(),
            auth_type: "key".into(),
            password: None,
            private_key: Some(sshd.client_key_pem().to_string()),
            passphrase: None,
            certificate: None,
            sudo_password: None,
        })
        .await
    }

    /// The first acquire of a test, which doubles as an environment gate.
    ///
    /// `nomi-ssh` gives a remote shell five seconds to reach its first sentinel,
    /// and a heavily loaded machine cannot always spawn one that fast — a failure
    /// that says nothing whatsoever about the pool. `None` means "this box cannot
    /// host these tests right now", reported the same way a missing sshd is. Every
    /// *other* dial failure still panics, so a real regression cannot hide here.
    pub async fn open_or_skip(
        &self,
        test: &str,
        conversation_id: &str,
        ssh_host_id: &SshHostId,
        remote_cwd: &str,
    ) -> Option<std::sync::Arc<nomifun_ssh::SshLink>> {
        match self
            .pool
            .acquire(&self.user_id, conversation_id, ssh_host_id, remote_cwd)
            .await
        {
            Ok(link) => Some(link),
            Err(nomifun_ssh::SshDialError::Protocol(detail))
                if detail.contains("did not become ready") =>
            {
                println!(
                    "SKIP: {test}: this machine could not start a remote shell inside \
                     nomi-ssh's init budget ({detail})"
                );
                None
            }
            Err(e) => panic!("{test}: acquire failed: {e}"),
        }
    }
}

/// Tuning that turns the pinned 1s→60s ladder into a ladder a test can sit
/// through. Deliberately not the production default — the real constants stay
/// pinned by `state.rs`'s own tests.
///
/// The poll interval is not as small as it could be: every connected link pays a
/// keepalive round trip per tick, and a whole file of these running two at a time
/// was loading the machine enough to push `nomi-ssh`'s 5s shell-init budget over
/// on a busy box. Detection within half a second is plenty to assert a ladder.
pub fn brisk_tuning() -> PoolTuning {
    PoolTuning {
        liveness_poll: std::time::Duration::from_millis(400),
        ping_timeout: std::time::Duration::from_millis(3_000),
        initial_backoff: std::time::Duration::from_millis(150),
        max_backoff: std::time::Duration::from_millis(400),
        max_attempts: 90,
        dial_cooldown: std::time::Duration::from_millis(75),
        close_budget: std::time::Duration::from_millis(3_000),
    }
}

/// Watch a link until it reaches `want`, returning every phase seen on the way
/// (current value first). Panics with the observed sequence on timeout, because
/// "which states did it actually go through" is the whole answer a failing
/// lifecycle test owes the reader.
pub async fn collect_phases_until(
    rx: &mut tokio::sync::watch::Receiver<nomifun_ssh::SshLinkState>,
    want: nomifun_ssh::SshLinkPhase,
    budget: std::time::Duration,
) -> Vec<nomifun_ssh::SshLinkPhase> {
    let mut seen = vec![rx.borrow_and_update().phase()];
    if seen[0] == want {
        return seen;
    }
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for {want:?}; saw {seen:?}");
        match tokio::time::timeout(remaining, rx.changed()).await {
            Ok(Ok(())) => {
                let phase = rx.borrow_and_update().phase();
                seen.push(phase);
                if phase == want {
                    return seen;
                }
            }
            Ok(Err(_)) => panic!("the link's state channel closed; saw {seen:?}"),
            Err(_) => panic!("timed out waiting for {want:?}; saw {seen:?}"),
        }
    }
}

