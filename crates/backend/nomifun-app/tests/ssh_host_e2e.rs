//! E2E for the SSH host-book routes: owner CRUD round-trip, masked secret in
//! responses, auth/owner protection, and the live link-status snapshot.
mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, delete_with_token, get_with_token, json_with_token, setup_and_login};

fn password_host_body() -> serde_json::Value {
    json!({
        "name": "prod-web",
        "host": "10.0.3.21",
        "port": 22,
        "username": "deploy",
        "authType": "password",
        "password": "hunter2_supersecret",
        "sudoPassword": "sudo_secret_pw"
    })
}

/// A host whose password auth has no password. The pool refuses it as an unusable
/// credential *before* opening a socket, which is how these HTTP tests get a link
/// into a real, terminal state without needing an sshd.
fn uncredentialed_host_body() -> serde_json::Value {
    json!({
        "name": "no-credential",
        "host": "127.0.0.1",
        "port": 1,
        "username": "nobody",
        "authType": "password"
    })
}

async fn create_host(
    app: &axum::Router,
    body: serde_json::Value,
    token: &str,
    csrf: &str,
) -> String {
    let req = json_with_token("POST", "/api/ssh-hosts", body, token, csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await["data"]["sshHostId"]
        .as_str()
        .expect("sshHostId")
        .to_string()
}

async fn create_launch_model(app: &axum::Router, token: &str, csrf: &str) -> serde_json::Value {
    let response = app.clone().oneshot(json_with_token("POST", "/api/providers", json!({
        "platform":"openai", "name":"No-request SSH launch model",
        "base_url":"https://example.invalid/v1", "auth_scheme":"bearer",
        "credentials":{"api_keys":["fixture-only"]}, "enabled":true,
        "initial_model":{"model":"ssh-launch-model", "enabled":true, "capabilities":[{
            "task":"chat", "traits":[], "protocol":"openai.chat_text",
            "connection_role":"default", "context_limit":128000
        }]}
    }), token, csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let provider = body_json(response).await;
    json!({"provider_id":provider["data"]["provider_id"], "model":"ssh-launch-model"})
}

#[tokio::test]
async fn ssh_session_launch_connects_without_a_turn_and_preserves_the_session_on_failure() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "fixture-password").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let host_id = create_host(&app, json!({
        "name":"SSH launch fixture", "host":"127.0.0.1", "port":port,
        "username":"fixture", "authType":"password"
    }), &token, &csrf).await;
    assert_eq!(services.ssh_pool.active_link_count(), 0, "saving a host does not connect");
    let model = create_launch_model(&app, &token, &csrf).await;
    let key = uuid::Uuid::now_v7().to_string();
    let request = || {
        let mut request = json_with_token("POST", "/api/agent-sessions/ssh",
            json!({"ssh_host_id":host_id, "model":model}), &token, &csrf);
        request.headers_mut().insert("idempotency-key", key.parse().unwrap());
        request
    };
    let response = app.clone().oneshot(request()).await.unwrap();
    let status = response.status();
    let opened = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let session = opened["data"]["agent_session_id"].as_str().unwrap();
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["data"], opened["data"], "same-key creation reuses the canonical receipt");
    let binding = &opened["data"]["agent_binding"];
    assert!(binding["typed_resource_bindings"].as_array().unwrap().iter().any(|resource|
        resource["resource_kind"] == "ssh_host" && resource["resource_id"] == host_id
            && resource["owner_id"] == services.authoritative_user_id.as_ref()));
    let response = app.clone().oneshot(get_with_token(
        &format!("/api/agent-sessions/{session}/projection"), &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let projection = body_json(response).await;
    assert_eq!(projection["data"]["agent_snapshot"]["enabled_capability_actions"]["ssh"], json!(["ssh/exec"]));
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let session_id = nomifun_agent_contracts::AgentSessionId::from(session.to_owned());
    let canonical = store.get_live_session(&session_id).await.unwrap();
    assert_eq!(serde_json::to_value(canonical.agent_binding).unwrap(), *binding);
    assert!(store.head(&session_id).await.unwrap().active_turn_id.is_none());
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if services.ssh_pool.snapshot(services.authoritative_user_id.as_ref()).iter()
                .any(|status| status.conversation_id == session && status.state == nomifun_ssh::SshLinkPhase::Dropped) {
                break;
            }
            tokio::task::yield_now().await;
        }
    }).await.expect("new Session starts its own dial without model input");
    assert_eq!(services.ssh_pool.active_link_count(), 1);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept()).await.is_err(),
        "a missing credential fails before opening a socket");
    assert_eq!(store.get_live_session(&session_id).await.unwrap().agent_session_id, session_id);
    let response = app.clone().oneshot(get_with_token("/api/agent-preset-templates", &token)).await.unwrap();
    let library = body_json(response).await;
    assert_eq!(library["data"]["user_presets"], json!([]), "SSH preparation creates no visible personal Agent");
    assert!(library["data"]["official_presets"]["chat.minimal"]["current_stable_revision"].is_null());

    let foreign_owner = nomifun_common::UserId::new();
    let foreign = services.ssh_pool.host_service().create(foreign_owner.as_str(),
        serde_json::from_value(json!({"name":"Other owner's host", "host":"127.0.0.1", "port":port,
            "username":"fixture", "authType":"password"})).unwrap()).await.unwrap();
    let response = app.clone().oneshot(json_with_token("POST", "/api/agent-sessions/ssh",
        json!({"ssh_host_id":foreign.ssh_host_id, "model":model}), &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND, "host ownership is checked before creating a Session");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 1);
    services.cleanup_after_startup_failure(anyhow::anyhow!("SSH Session launch fixture ended")).await;
}

// Controlled SSH/SFTP protocol evidence. The peer implements only the shell
// readiness marker, SFTP version handshake and exit proof; it executes no OS
// command and never touches the operator's known_hosts.
#[derive(Default)]
struct SshPeerEvidence {
    authenticated: std::sync::atomic::AtomicUsize,
    sftp_ready: std::sync::atomic::AtomicUsize,
    shell_exits: std::sync::atomic::AtomicUsize,
}

struct SshPeer {
    channels: std::collections::HashMap<russh::ChannelId, (bool, Vec<u8>)>,
    evidence: std::sync::Arc<SshPeerEvidence>,
}

impl russh::server::Handler for SshPeer {
    type Error = russh::Error;

    async fn auth_password(&mut self, username: &str, password: &str) -> Result<russh::server::Auth, Self::Error> {
        assert_eq!((username, password), ("controlled", "public-fixture-password"));
        self.evidence.authenticated.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(russh::server::Auth::Accept)
    }

    async fn channel_open_session(&mut self, channel: russh::Channel<russh::server::Msg>,
        reply: russh::server::ChannelOpenHandle, _: &mut russh::server::Session) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), (false, Vec::new()));
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(&mut self, channel: russh::ChannelId, _: &str, _: u32, _: u32, _: u32, _: u32,
        _: &[(russh::Pty, u32)], session: &mut russh::server::Session) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(&mut self, channel: russh::ChannelId, session: &mut russh::server::Session) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn subsystem_request(&mut self, channel: russh::ChannelId, name: &str, session: &mut russh::server::Session) -> Result<(), Self::Error> {
        assert_eq!(name, "sftp");
        self.channels.get_mut(&channel).unwrap().0 = true;
        session.channel_success(channel)?;
        Ok(())
    }

    async fn data(&mut self, channel: russh::ChannelId, bytes: &[u8], session: &mut russh::server::Session) -> Result<(), Self::Error> {
        let (sftp, input) = self.channels.get_mut(&channel).unwrap();
        input.extend_from_slice(bytes);
        if *sftp {
            if input.len() >= 9 {
                assert_eq!(input.as_slice(), &[0, 0, 0, 5, 1, 0, 0, 0, 3]);
                session.data(channel, vec![0, 0, 0, 5, 2, 0, 0, 0, 3])?;
                self.evidence.sftp_ready.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                input.clear();
            }
        } else if input.as_slice() == b"exit\n" {
            session.exit_status_request(channel, 0)?;
            session.close(channel)?;
            self.evidence.shell_exits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            input.clear();
        } else if let Some(nonce) = String::from_utf8_lossy(input).split_once("__NOMI_END_")
            .and_then(|(_, suffix)| suffix.split_once("__"))
            .and_then(|(value, _)| value.parse::<u64>().ok()) {
            session.data(channel, format!("__NOMI_END_{nonce}__0__/controlled\n").into_bytes())?;
            input.clear();
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn ssh_launch_disconnect_reconnect_and_host_shutdown_use_the_same_real_protocol_link() {
    use std::sync::{Arc, atomic::Ordering};
    use std::time::Duration;
    use nomifun_app::compatibility::{AppServices, create_router};

    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("work")).unwrap();
    let mut services = AppServices::from_config(nomifun_db::init_database_memory().await.unwrap(),
        &nomifun_app::AppConfig {
            data_dir: directory.path().join("data"), work_dir: directory.path().join("work"),
            ..Default::default()
        }).await.unwrap();
    services.ssh_pool = nomifun_ssh::SshConnectionPool::new(
        services.ssh_pool.host_service(), directory.path().join("known_hosts"),
        nomifun_ssh::SshEventEmitter::new(services.event_bus.clone()),
    );
    let mut app = create_router(&services).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "fixture-password").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let evidence = Arc::new(SshPeerEvidence::default());
    let server_evidence = evidence.clone();
    let begin_handshake = Arc::new(tokio::sync::Notify::new());
    let server_begin_handshake = begin_handshake.clone();
    let peer = tokio::spawn(async move {
        let config = Arc::new(russh::server::Config {
            keys: vec![russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[91; 32]).into()],
            inactivity_timeout: Some(Duration::from_secs(30)), ..Default::default()
        });
        let mut connections = Vec::new();
        for index in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            if index == 0 { server_begin_handshake.notified().await; }
            let config = config.clone();
            let handler = SshPeer { channels: Default::default(), evidence: server_evidence.clone() };
            connections.push(tokio::spawn(async move {
                russh::server::run_stream(config, stream, handler).await.unwrap().await
            }));
        }
        for connection in connections {
            match connection.await.unwrap() {
                Ok(()) => {},
                Err(russh::Error::IO(error)) if matches!(error.kind(), std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe) => {},
                Err(error) => panic!("controlled SSH peer failed: {error}"),
            }
        }
    });
    let host_id = create_host(&app, json!({"name":"Controlled remote", "host":"127.0.0.1", "port":port,
        "username":"controlled", "authType":"password", "password":"public-fixture-password"}), &token, &csrf).await;
    assert_eq!(evidence.authenticated.load(Ordering::SeqCst), 0, "saving a host is passive");
    let model = create_launch_model(&app, &token, &csrf).await;
    let key = uuid::Uuid::now_v7().to_string();
    let create = || {
        let mut request = json_with_token("POST", "/api/agent-sessions/ssh", json!({"ssh_host_id":host_id, "model":model}), &token, &csrf);
        request.headers_mut().insert("idempotency-key", key.parse().unwrap());
        request
    };
    let response = tokio::time::timeout(Duration::from_secs(2), app.clone().oneshot(create())).await
        .expect("Session creation returns before the peer permits its SSH handshake").unwrap();
    begin_handshake.notify_one();
    assert_eq!(response.status(), StatusCode::OK);
    let opened = body_json(response).await;
    let session = opened["data"]["agent_session_id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !services.ssh_pool.snapshot(services.authoritative_user_id.as_ref()).iter()
            .any(|status| status.conversation_id == session && status.state == nomifun_ssh::SshLinkPhase::Connected) {
            tokio::task::yield_now().await;
        }
    }).await.expect("launch establishes SSH, shell and SFTP before any model input");
    assert_eq!(evidence.authenticated.load(Ordering::SeqCst), 1);
    assert_eq!(evidence.sftp_ready.load(Ordering::SeqCst), 1);
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let session_id = nomifun_agent_contracts::AgentSessionId::from(session.to_owned());
    assert!(store.head(&session_id).await.unwrap().active_turn_id.is_none());

    let response = app.clone().oneshot(json_with_token("POST", &format!("/api/agent-sessions/{session}/ssh/disconnect"), json!({}), &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(5), async {
        while services.ssh_pool.active_link_count() != 0 {
            tokio::task::yield_now().await;
        }
    }).await.expect("disconnect closes the original native link");
    assert_eq!(evidence.shell_exits.load(Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let closes = store.session_ssh_link_closes(&nomifun_agent_contracts::PrincipalRef {
                principal_kind:"user".into(), principal_id: services.authoritative_user_id.to_string(),
            }, &session_id).await.unwrap();
            if closes.len() == 1 && matches!(closes[0].1.as_ref().map(|closed| &closed.result),
                Some(nomifun_agent_contracts::SessionSshLinkCloseResult::Reaped {..})) { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("disconnect persists the exact original close proof");
    let response = app.clone().oneshot(create()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["data"], opened["data"]);
    assert_eq!(services.ssh_pool.active_link_count(), 0, "creation replay does not reconnect a closed Session");
    for path in [format!("/api/agent-sessions/{session}"), format!("/api/agent-sessions/{session}/projection")] {
        assert_eq!(app.clone().oneshot(get_with_token(&path, &token)).await.unwrap().status(), StatusCode::OK);
    }
    assert_eq!(services.ssh_pool.active_link_count(), 0, "reading a disconnected Session is passive");
    assert_eq!(evidence.authenticated.load(Ordering::SeqCst), 1);
    let canonical = store.get_live_session(&session_id).await.unwrap();
    let resource = canonical.agent_binding.typed_resource_bindings.iter()
        .find(|resource| resource.resource_kind.as_ref() == "ssh_host").unwrap();
    let authority = nomifun_ssh::AgentSshAuthority::bound(services.authoritative_user_id.to_string(),
        nomifun_ssh::AgentSshHostResource::from_operation_names(
            resource.owner_id.clone(),
            nomifun_common::SshHostId::parse(resource.resource_id.as_ref()).unwrap(),
            resource.operations.iter(),
        ).unwrap()).unwrap();
    let actions = nomifun_ssh::SshActionOwner::new(services.ssh_pool.clone());
    let result = tokio::time::timeout(Duration::from_secs(2), actions.exec(&authority,
        &nomifun_ssh::SshActionContext {
            principal_id: services.authoritative_user_id.to_string(),
            agent_session_id: session.to_owned(), operation_id: "disconnected-fixture-action".into(),
        }, nomifun_ssh::SshExecInput { command:"MUST_NOT_RUN".into(), timeout_ms: 1_000 },
    )).await.unwrap();
    assert!(result.is_err(), "a remote tool must ask for explicit reconnection after disconnect");
    assert_eq!(services.ssh_pool.active_link_count(), 0);
    assert_eq!(evidence.authenticated.load(Ordering::SeqCst), 1);

    let response = app.clone().oneshot(json_with_token("POST", &format!("/api/agent-sessions/{session}/ssh/connect"), json!({}), &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !services.ssh_pool.snapshot(services.authoritative_user_id.as_ref()).iter()
            .any(|status| status.conversation_id == session && status.state == nomifun_ssh::SshLinkPhase::Connected) {
            tokio::task::yield_now().await;
        }
    }).await.expect("explicit connect establishes the current canonical binding");
    assert_eq!(evidence.authenticated.load(Ordering::SeqCst), 2);
    assert_eq!(evidence.sftp_ready.load(Ordering::SeqCst), 2);
    assert!(directory.path().join("known_hosts").is_file());
    services.shutdown_nomi_core_host().await.unwrap();
    assert_eq!(services.ssh_pool.active_link_count(), 0);
    assert_eq!(evidence.shell_exits.load(Ordering::SeqCst), 2, "host shutdown closes the reconnected shell");
    tokio::time::timeout(Duration::from_secs(5), peer).await.unwrap().unwrap();
}

#[tokio::test]
async fn restarting_the_host_and_reading_a_saved_ssh_session_does_not_connect() {
    use nomifun_app::compatibility::{AppServices, create_router};

    let directory = tempfile::tempdir().unwrap();
    let config = nomifun_app::AppConfig {
        data_dir: directory.path().join("data"), work_dir: directory.path().join("work"),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    std::fs::create_dir_all(&config.work_dir).unwrap();
    let first = AppServices::from_config(nomifun_db::init_database(&config.database_path()).await.unwrap(), &config).await.unwrap();
    let mut app = create_router(&first).await;
    let (token, csrf) = setup_and_login(&mut app, &first, "admin", "fixture-password").await;
    let host_id = create_host(&app, uncredentialed_host_body(), &token, &csrf).await;
    let model = create_launch_model(&app, &token, &csrf).await;
    let response = app.clone().oneshot(json_with_token("POST", "/api/agent-sessions/ssh",
        json!({"ssh_host_id":host_id, "model":model}), &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let opened = body_json(response).await;
    let session = opened["data"]["agent_session_id"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !first.ssh_pool.snapshot(first.authoritative_user_id.as_ref()).iter()
            .any(|status| status.conversation_id == session && status.state == nomifun_ssh::SshLinkPhase::Dropped) {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    drop(app);
    first.shutdown_nomi_core_host().await.unwrap();
    drop(first);

    let second = AppServices::from_config(nomifun_db::init_database(&config.database_path()).await.unwrap(), &config).await.unwrap();
    let mut restored = create_router(&second).await;
    let (token, _) = setup_and_login(&mut restored, &second, "admin", "fixture-password").await;
    assert_eq!(second.ssh_pool.active_link_count(), 0, "startup never dials historical Sessions");
    for path in [format!("/api/agent-sessions/{session}"), format!("/api/agent-sessions/{session}/projection")] {
        let response = restored.clone().oneshot(get_with_token(&path, &token)).await.unwrap();
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
    }
    assert_eq!(second.ssh_pool.active_link_count(), 0);
    assert!(second.ssh_pool.snapshot(second.authoritative_user_id.as_ref()).is_empty());
    second.shutdown_nomi_core_host().await.unwrap();
}

#[tokio::test]
async fn unauthenticated_requests_are_rejected() {
    let (app, _services) = build_app().await;
    let req = get_with_token("/api/ssh-hosts", "not-a-real-token");
    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "unauthenticated list must not succeed"
    );
}

#[tokio::test]
async fn owner_can_crud_and_secret_is_masked() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;

    // Create
    let req = json_with_token("POST", "/api/ssh-hosts", password_host_body(), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created = body_json(resp).await;
    assert_eq!(created["success"], true);
    let data = &created["data"];
    let host_id = data["sshHostId"].as_str().expect("sshHostId").to_string();
    // Plaintext must never appear; presence is masked.
    let raw = created.to_string();
    assert!(!raw.contains("hunter2_supersecret"), "password leaked: {raw}");
    assert!(!raw.contains("sudo_secret_pw"), "sudo password leaked: {raw}");
    assert_eq!(data["password"], "***");
    assert_eq!(data["sudoPassword"], "***");
    assert_eq!(data["host"], "10.0.3.21");

    // List shows it
    let req = get_with_token("/api/ssh-hosts", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let listed = body_json(resp).await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);

    // Update the name only, resending the mask for the password (unchanged)
    let req = json_with_token(
        "PUT",
        &format!("/api/ssh-hosts/{host_id}"),
        json!({ "name": "prod-web-renamed", "password": "***" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(updated["data"]["name"], "prod-web-renamed");
    assert_eq!(updated["data"]["password"], "***");

    // Delete
    let req = delete_with_token(&format!("/api/ssh-hosts/{host_id}"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Gone
    let req = get_with_token("/api/ssh-hosts", &token);
    let resp = app.oneshot(req).await.unwrap();
    let listed = body_json(resp).await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn unknown_field_is_rejected() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let req = json_with_token(
        "POST",
        "/api/ssh-hosts",
        json!({ "name": "x", "host": "h", "username": "u", "authType": "password", "bogus": 1 }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn statuses_snapshot_is_owner_scoped_and_matches_the_link_state() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let host_id = create_host(&app, uncredentialed_host_body(), &token, &csrf).await;
    let parsed = nomifun_common::SshHostId::parse(host_id.clone()).expect("host id");

    // Nothing has been dialled yet: an empty list, not a 404 and not an error.
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/ssh-hosts/statuses", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let before = body_json(resp).await;
    assert_eq!(before["success"], true);
    assert_eq!(before["data"].as_array().expect("array").len(), 0);

    // One link for the owner, one for somebody else, in the pool the router
    // actually reads. Both dials fail on the missing password, which is what puts
    // them into a state worth reporting.
    let owner = services.authoritative_user_id.to_string();
    let stranger = nomifun_common::UserId::new().as_str().to_string();
    for (user_id, conversation_id) in [
        (owner.as_str(), "conv-owner"),
        (stranger.as_str(), "conv-stranger"),
    ] {
        services
            .ssh_pool
            .acquire(user_id, conversation_id, &parsed, "/")
            .await
            .expect_err("password auth without a password cannot dial");
    }
    assert_eq!(
        services.ssh_pool.active_link_count(),
        2,
        "both failed dials must still leave a link the pool knows about"
    );

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/ssh-hosts/statuses", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let rows = body["data"].as_array().expect("array");
    assert_eq!(
        rows.len(),
        1,
        "another owner's link must be invisible here: {body}"
    );
    let row = &rows[0];
    assert_eq!(row["conversationId"], "conv-owner");
    assert_eq!(row["sshHostId"], host_id);
    // The snapshot is the same projection the realtime event carries, so it must
    // spell out the terminal drop rather than leaving the client to guess.
    assert_eq!(row["state"], "dropped");
    assert_eq!(row["retryable"], false);
    assert_eq!(row["reaped"], serde_json::Value::Null);
    assert!(
        row["detail"].as_str().is_some_and(|d| !d.is_empty()),
        "a dropped link owes the operator a reason: {row}"
    );
    // Whatever the reason says, it must not be the credential itself.
    let raw = body.to_string();
    assert!(!raw.contains("password_encrypted"), "{raw}");
}

#[tokio::test]
async fn statuses_route_does_not_shadow_the_ssh_host_id_capture() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let host_id = create_host(&app, password_host_body(), &token, &csrf).await;

    // `statuses` is a literal segment sitting exactly where `{ssh_host_id}` also
    // matches. axum prefers the literal, which is only safe as long as no real
    // host id can be spelled `statuses` — a uuid never can. Pinned here because
    // the failure mode is a silent one: the single-host GET would start answering
    // with a list.
    let resp = app
        .clone()
        .oneshot(get_with_token(&format!("/api/ssh-hosts/{host_id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let one = body_json(resp).await;
    assert_eq!(one["data"]["sshHostId"], host_id);
    assert_eq!(one["data"]["name"], "prod-web");
    assert!(
        one["data"].is_object(),
        "the id capture must still reach the single-host handler: {one}"
    );

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/ssh-hosts/statuses", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = body_json(resp).await;
    assert!(
        listed["data"].is_array(),
        "the literal segment is its own route: {listed}"
    );
}
