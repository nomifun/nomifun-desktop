//! Controlled SSH/SFTP protocol evidence for the unsaved connection-test route.
//! The loopback peer executes no OS commands and uses a temporary known_hosts.
#[path = "support/mod.rs"]
mod support;

use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use nomifun_auth::CurrentUser;
use nomifun_common::UserId;
use nomifun_ssh::{SshHostRouterState, dto::CreateSshHostRequest};
use russh::{ChannelId, server};
use serde_json::{Value, json};
use tower::ServiceExt;

const PASSWORD: &str = "public-fixture-password";
const ROUTE: &str = "/api/ssh-hosts/test-connection";

#[derive(Default)]
struct Evidence {
    accepted: AtomicUsize,
    authenticated: AtomicUsize,
    sftp_ready: AtomicUsize,
    shell_exits: AtomicUsize,
    probe_seen: AtomicBool,
    probe_started: tokio::sync::Notify,
    release_probe: tokio::sync::Notify,
}

#[derive(Default)]
struct Input { sftp: bool, bytes: Vec<u8>, commands: usize }

struct Peer {
    channels: HashMap<ChannelId, Input>,
    evidence: Arc<Evidence>,
    wait_probe: bool,
    probe_exit: i32,
    deny_exit_proof: bool,
}

impl server::Handler for Peer {
    type Error = russh::Error;

    async fn auth_password(&mut self, username: &str, password: &str) -> Result<server::Auth, Self::Error> {
        if username != "controlled" || password != PASSWORD {
            return Ok(server::Auth::Reject { proceed_with_methods: None, partial_success: false });
        }
        self.evidence.authenticated.fetch_add(1, Ordering::SeqCst);
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(&mut self, channel: russh::Channel<server::Msg>,
        reply: server::ChannelOpenHandle, _: &mut server::Session) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), Input::default());
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(&mut self, channel: ChannelId, _: &str, _: u32, _: u32, _: u32, _: u32,
        _: &[(russh::Pty, u32)], session: &mut server::Session) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut server::Session) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn subsystem_request(&mut self, channel: ChannelId, name: &str,
        session: &mut server::Session) -> Result<(), Self::Error> {
        assert_eq!(name, "sftp");
        self.channels.get_mut(&channel).unwrap().sftp = true;
        session.channel_success(channel)?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut server::Session) -> Result<(), Self::Error> {
        let input = self.channels.get_mut(&channel).unwrap();
        input.bytes.extend_from_slice(data);
        if input.sftp {
            if input.bytes.len() >= 9 {
                assert_eq!(input.bytes.as_slice(), &[0, 0, 0, 5, 1, 0, 0, 0, 3]);
                session.data(channel, vec![0, 0, 0, 5, 2, 0, 0, 0, 3])?;
                self.evidence.sftp_ready.fetch_add(1, Ordering::SeqCst);
                input.bytes.clear();
            }
            return Ok(());
        }
        if input.bytes.as_slice() == b"exit\n" {
            if !self.deny_exit_proof {
                session.exit_status_request(channel, 0)?;
                session.close(channel)?;
                self.evidence.shell_exits.fetch_add(1, Ordering::SeqCst);
            }
            input.bytes.clear();
            return Ok(());
        }
        let text = String::from_utf8_lossy(&input.bytes);
        let nonce = text.split_once("__NOMI_END_")
            .and_then(|(_, suffix)| suffix.split_once("__"))
            .and_then(|(value, _)| value.parse::<u64>().ok());
        let Some(nonce) = nonce else { return Ok(()); };
        input.commands += 1;
        let is_probe = input.commands > 1;
        if is_probe {
            self.evidence.probe_seen.store(true, Ordering::SeqCst);
            self.evidence.probe_started.notify_waiters();
            if self.wait_probe && input.commands == 2 { self.evidence.release_probe.notified().await; }
        }
        let exit = if is_probe { self.probe_exit } else { 0 };
        session.data(channel, format!("__NOMI_END_{nonce}__{exit}__/controlled\n").into_bytes())?;
        input.bytes.clear();
        Ok(())
    }
}

struct PeerServer {
    port: u16,
    evidence: Arc<Evidence>,
    worker: tokio::task::JoinHandle<()>,
}

impl PeerServer {
    async fn start(connections: usize, wait_probe: bool, probe_exit: i32, deny_exit_proof: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let evidence = Arc::new(Evidence::default());
        let server_evidence = evidence.clone();
        let worker = tokio::spawn(async move {
            let config = Arc::new(server::Config {
                keys: vec![russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[112; 32]).into()],
                auth_rejection_time: Duration::ZERO,
                auth_rejection_time_initial: Some(Duration::ZERO),
                inactivity_timeout: Some(Duration::from_secs(30)),
                ..Default::default()
            });
            let mut workers = Vec::new();
            for _ in 0..connections {
                let (stream, _) = listener.accept().await.unwrap();
                server_evidence.accepted.fetch_add(1, Ordering::SeqCst);
                let peer = Peer { channels: HashMap::new(), evidence: server_evidence.clone(),
                    wait_probe, probe_exit, deny_exit_proof };
                let config = config.clone();
                workers.push(tokio::spawn(async move {
                    server::run_stream(config, stream, peer).await.unwrap().await
                }));
            }
            for worker in workers {
                match worker.await.unwrap() {
                    Ok(()) => {},
                    Err(russh::Error::IO(error)) if matches!(error.kind(), std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::UnexpectedEof) => {},
                    Err(russh::Error::Disconnect) => {},
                    Err(error) => panic!("controlled SSH peer failed: {error}"),
                }
            }
        });
        Self { port, evidence, worker }
    }

    async fn wait_for_probe(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.evidence.probe_started.notified();
                if self.evidence.probe_seen.load(Ordering::SeqCst) { break; }
                notified.await;
            }
        }).await.unwrap();
    }

    async fn finish(self) {
        tokio::time::timeout(Duration::from_secs(5), self.worker).await.unwrap().unwrap();
    }
}

fn router(harness: &support::PoolHarness, owner: &str) -> axum::Router {
    nomifun_ssh::ssh_host_routes(SshHostRouterState {
        service: harness.service(), pool: harness.pool.clone(),
    }).layer(Extension(CurrentUser { id: UserId::parse(owner.to_owned()).unwrap(), username: owner.to_owned() }))
}

fn draft(port: u16) -> Value {
    json!({"host":"127.0.0.1", "port":port, "username":"controlled",
        "authType":"password", "password":PASSWORD})
}

fn request(body: Value) -> Request<Body> {
    Request::builder().method("POST").uri(ROUTE).header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()
}

async fn test(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request(body)).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn saved(harness: &support::PoolHarness, port: u16) -> nomifun_common::SshHostId {
    harness.add_host(CreateSshHostRequest {
        name: "Saved fixture".into(), host: "127.0.0.1".into(), port: port.into(),
        username: "controlled".into(), auth_type: "password".into(), password: Some(PASSWORD.into()),
        private_key: None, passphrase: None, certificate: None, sudo_password: Some("unused-sudo-secret".into()),
    }).await
}

#[tokio::test]
async fn unsaved_probe_checks_shell_and_sftp_then_closes_without_creating_data() {
    let directory = tempfile::tempdir().unwrap();
    let harness = support::harness(directory.path().join("known_hosts"), support::brisk_tuning()).await;
    let peer = PeerServer::start(1, false, 0, false).await;
    let (status, result) = test(&router(&harness, &harness.user_id), draft(peer.port)).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["ok"], true, "{result}");
    assert!(!result.to_string().contains(PASSWORD));
    assert!(harness.service().list(&harness.user_id).await.unwrap().is_empty());
    assert_eq!(harness.pool.active_link_count(), 0);
    assert!(harness.events.status_payloads().is_empty());
    assert_eq!(peer.evidence.sftp_ready.load(Ordering::SeqCst), 1);
    assert_eq!(peer.evidence.shell_exits.load(Ordering::SeqCst), 1);
    assert!(directory.path().join("known_hosts").is_file());
    peer.finish().await;
}

#[tokio::test]
async fn edited_probe_uses_current_endpoint_and_reuses_only_unchanged_secrets_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let harness = support::harness(directory.path().join("known_hosts"), support::brisk_tuning()).await;
    let id = saved(&harness, 1).await;
    let before = serde_json::to_value(harness.service().get(&harness.user_id, &id).await.unwrap()).unwrap();
    let app = router(&harness, &harness.user_id);
    let peer = PeerServer::start(2, false, 0, false).await;
    for masked in [false, true] {
        let mut body = draft(peer.port);
        body["sshHostId"] = json!(id.as_str());
        if masked { body["password"] = json!("***"); } else { body.as_object_mut().unwrap().remove("password"); }
        let (status, result) = test(&app, body).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["data"]["ok"], true, "{result}");
    }
    assert_eq!(serde_json::to_value(harness.service().get(&harness.user_id, &id).await.unwrap()).unwrap(), before,
        "testing edited coordinates cannot overwrite endpoint, fingerprint or timestamps");
    assert_eq!(peer.evidence.authenticated.load(Ordering::SeqCst), 2);
    assert_eq!(peer.evidence.shell_exits.load(Ordering::SeqCst), 2);
    peer.finish().await;
}

#[tokio::test]
async fn fixed_draft_password_retries_immediately_without_saved_host_cooldown() {
    let directory = tempfile::tempdir().unwrap();
    let harness = support::harness(directory.path().join("known_hosts"), support::brisk_tuning()).await;
    let peer = PeerServer::start(2, false, 0, false).await;
    let id = saved(&harness, peer.port).await;
    let app = router(&harness, &harness.user_id);
    let mut body = draft(peer.port);
    body["sshHostId"] = json!(id.as_str());
    body["password"] = json!("wrong-fixture-password");
    let (status, failure) = test(&app, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{failure}");
    assert_eq!(failure["data"]["ok"], false);
    body["password"] = json!(PASSWORD);
    let (status, success) = test(&app, body).await;
    assert_eq!(status, StatusCode::OK, "{success}");
    assert_eq!(success["data"]["ok"], true, "{success}");
    assert_eq!(peer.evidence.accepted.load(Ordering::SeqCst), 2);
    peer.finish().await;
}

#[tokio::test]
async fn ownership_invalid_inputs_and_cleared_secrets_fail_before_network() {
    let directory = tempfile::tempdir().unwrap();
    let harness = support::harness(directory.path().join("known_hosts"), support::brisk_tuning()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let id = saved(&harness, port).await;
    let app = router(&harness, &harness.user_id);
    for (field, value) in [("port", json!(0)), ("port", json!(65536)), ("authType", json!("unknown")),
        ("name", json!("irrelevant")), ("sudoPassword", json!("not-tested"))] {
        let mut body = draft(port);
        body[field] = value;
        let (status, result) = test(&app, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {result}");
    }
    let mut body = draft(port);
    body["sshHostId"] = json!(id.as_str());
    let stranger = UserId::new();
    let (status, result) = test(&router(&harness, stranger.as_str()), body.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{result}");
    body["password"] = json!("");
    let (status, result) = test(&app, body).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["ok"], false);
    assert!(tokio::time::timeout(Duration::from_millis(30), listener.accept()).await.is_err());
    assert_eq!(harness.pool.active_link_count(), 0);
}

#[tokio::test]
async fn probe_nonzero_status_and_missing_exit_proof_are_failures() {
    for (exit, deny_exit) in [(17, false), (0, true)] {
        let directory = tempfile::tempdir().unwrap();
        let mut tuning = support::brisk_tuning();
        tuning.close_budget = Duration::from_millis(50);
        let harness = support::harness(directory.path().join("known_hosts"), tuning).await;
        let peer = PeerServer::start(1, false, exit, deny_exit).await;
        let (status, result) = test(&router(&harness, &harness.user_id), draft(peer.port)).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["data"]["ok"], false, "{result}");
        peer.finish().await;
    }
}

#[tokio::test]
async fn probe_timeout_is_a_failure_even_when_the_original_shell_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let harness = support::harness(directory.path().join("known_hosts"), support::brisk_tuning()).await;
    let peer = PeerServer::start(1, true, 0, false).await;
    let app = router(&harness, &harness.user_id);
    let body = draft(peer.port);
    let request = tokio::spawn(async move { test(&app, body).await });
    peer.wait_for_probe().await;
    // Withhold the actual probe response beyond its 15-second deadline, then
    // supply both original and resync markers within the recovery allowance.
    tokio::time::sleep(Duration::from_secs(16)).await;
    peer.evidence.release_probe.notify_one();
    let (status, result) = tokio::time::timeout(Duration::from_secs(5), request).await.unwrap().unwrap();
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["ok"], false, "{result}");
    assert!(result["data"]["message"].as_str().unwrap().contains("timed out"), "{result}");
    assert_eq!(peer.evidence.shell_exits.load(Ordering::SeqCst), 1);
    peer.finish().await;
}

#[tokio::test]
async fn cancelled_http_waiter_does_not_cancel_probe_cleanup_and_shutdown_joins_it() {
    let directory = tempfile::tempdir().unwrap();
    let harness = support::harness(directory.path().join("known_hosts"), support::brisk_tuning()).await;
    let peer = PeerServer::start(1, true, 0, false).await;
    let app = router(&harness, &harness.user_id);
    let body = draft(peer.port);
    let request = tokio::spawn(async move { test(&app, body).await });
    peer.wait_for_probe().await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    let pool = harness.pool.clone();
    let mut shutdown = tokio::spawn(async move { pool.shutdown_all().await });
    assert!(tokio::time::timeout(Duration::from_millis(30), &mut shutdown).await.is_err(),
        "shutdown must wait for the original probe's admitted native work");
    peer.evidence.release_probe.notify_one();
    tokio::time::timeout(Duration::from_secs(5), shutdown).await.unwrap().unwrap();
    assert_eq!(peer.evidence.shell_exits.load(Ordering::SeqCst), 1);
    peer.finish().await;
}
