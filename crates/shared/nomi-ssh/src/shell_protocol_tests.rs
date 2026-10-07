//! A scripted SSH peer, not an external sshd. Exercise the public connection /
//! shell entry point and real russh channel messages without executing scripts.
use super::*;
use crate::{
    connection::HostKeyPolicy,
    credential::{Auth, SshCredential},
};
use russh::{Channel, ChannelId, Pty, server};

struct Peer {
    init_status: i32,
    input: Vec<u8>,
    stop_window_updates: bool,
    closed: Arc<tokio::sync::Notify>,
    exit_proof: bool,
    fence_status: i32,
    fence_runs: Arc<std::sync::atomic::AtomicUsize>,
    received: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl server::Handler for Peer {
    type Error = russh::Error;

    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _: ChannelId,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.closed.notify_one();
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: russh::Sig,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.exit_proof && matches!(signal, russh::Sig::TERM) {
            session.exit_status_request(channel, 143)?;
            session.close(channel)?;
        }
        Ok(())
    }

    fn adjust_window(&mut self, _: ChannelId, current: u32) -> u32 {
        // russh ignores zero here; one disables replenishment (target / 2 == 0).
        if self.stop_window_updates { 1 } else { current }
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.input.extend_from_slice(data);
        self.received.lock().unwrap().extend_from_slice(data);
        if self
            .input
            .windows(b"__NOMI_END_0__".len())
            .any(|s| s == b"__NOMI_END_0__")
        {
            let cwd = if self.init_status == 0 {
                "/requested-directory"
            } else {
                "/original-directory"
            };
            session.data(
                channel,
                format!("__NOMI_END_0__{}__{cwd}\n", self.init_status).into_bytes(),
            )?;
            self.input.clear();
        } else if self.input.windows(b"fixture_terminal_exit".len()).any(|s| s == b"fixture_terminal_exit") {
            session.data(channel, b"terminal_marker".to_vec())?;
            session.exit_status_request(channel, 7)?;
            session.close(channel)?;
            self.input.clear();
        } else if self
            .input
            .windows(b"NoNewPrivs:".len())
            .any(|s| s == b"NoNewPrivs:")
        {
            self.fence_runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            session.data(channel, format!("__NOMI_END_1__{}__/requested-directory\n", self.fence_status).into_bytes())?;
            self.input.clear();
        } else if self
            .input
            .windows(b"__NOMI_END_1__".len())
            .any(|s| s == b"__NOMI_END_1__")
        {
            if self
                .input
                .windows(b"fixture_prompt".len())
                .any(|s| s == b"fixture_prompt")
            {
                session.data(channel, b"Password: ".to_vec())?;
            } else if self
                .input
                .windows(b"fixture_success".len())
                .any(|s| s == b"fixture_success")
            {
                session.data(
                    channel,
                    b"first\n__NOMI_END_1__0__/requested-directory\n".to_vec(),
                )?;
            }
            self.input.clear();
        } else if self
            .input
            .windows(b"__NOMI_END_2__".len())
            .any(|s| s == b"__NOMI_END_2__")
        {
            session.data(channel, b"__NOMI_END_2__0__/requested-directory\n".to_vec())?;
            self.input.clear();
        } else if self.exit_proof && data == b"exit\n" {
            session.exit_status_request(channel, 0)?;
            session.close(channel)?;
        } else if self.input.windows(b"__NOMI_END_3__".len()).any(|s| s == b"__NOMI_END_3__") {
            session.data(channel, b"__NOMI_END_3__0__/requested-directory\n".to_vec())?;
            self.input.clear();
        }
        Ok(())
    }
}

pub(super) async fn connect_peer(
    init_status: i32,
) -> (
    SshConnection,
    tokio::task::JoinHandle<Result<(), russh::Error>>,
) {
    connect_peer_with_window(init_status, 2 * 1024 * 1024).await
}

pub(super) async fn connect_peer_with_window(
    init_status: i32,
    window_size: u32,
) -> (
    SshConnection,
    tokio::task::JoinHandle<Result<(), russh::Error>>,
) {
    let (connection, task, _) = connect_observed_peer(init_status, window_size).await;
    (connection, task)
}

pub(super) async fn connect_observed_peer(
    init_status: i32,
    window_size: u32,
) -> (
    SshConnection,
    tokio::task::JoinHandle<Result<(), russh::Error>>,
    Arc<tokio::sync::Notify>,
) {
    connect_scripted_peer(init_status, window_size, false).await
}

pub(super) async fn connect_scripted_peer(
    init_status: i32,
    window_size: u32,
    exit_proof: bool,
) -> (
    SshConnection,
    tokio::task::JoinHandle<Result<(), russh::Error>>,
    Arc<tokio::sync::Notify>,
) {
    let (connection, task, closed, _, _) = connect_fenced_peer(init_status, window_size, exit_proof, 0).await;
    (connection, task, closed)
}

async fn connect_fenced_peer(
    init_status: i32,
    window_size: u32,
    exit_proof: bool,
    fence_status: i32,
) -> (
    SshConnection,
    tokio::task::JoinHandle<Result<(), russh::Error>>,
    Arc<tokio::sync::Notify>,
    Arc<std::sync::atomic::AtomicUsize>,
    Arc<std::sync::Mutex<Vec<u8>>>,
) {
    let closed = Arc::new(tokio::sync::Notify::new());
    let peer_closed = closed.clone();
    let fence_runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peer_fence_runs = fence_runs.clone();
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let peer_received = received.clone();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(server::Config {
        // Public deterministic fixture material; never used for a real host.
        keys: vec![russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[42; 32]).into()],
        inactivity_timeout: Some(Duration::from_secs(30)),
        window_size,
        ..Default::default()
    });
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        server::run_stream(
            config,
            stream,
            Peer {
                init_status,
                input: Vec::new(),
                stop_window_updates: window_size < 2 * 1024 * 1024,
                closed: peer_closed,
                exit_proof,
                fence_status,
                fence_runs: peer_fence_runs,
                received: peer_received,
            },
        )
        .await?
        .await
    });
    let directory = tempfile::tempdir().unwrap();
    let credential = SshCredential {
        host: "127.0.0.1".into(),
        port,
        username: "fixture".into(),
        auth: Auth::Password(zeroize::Zeroizing::new("synthetic-fixture-password".into())),
    };
    let connection = SshConnection::connect(
        &credential,
        HostKeyPolicy::AcceptNew {
            known_hosts: directory.path().join("known_hosts"),
        },
    )
    .await
    .unwrap();
    (connection, task, closed, fence_runs, received)
}

#[tokio::test]
async fn rejected_privilege_fence_never_submits_the_callers_command() {
    let (connection, task, _, _, received) = connect_fenced_peer(0, 2 * 1024 * 1024, false, 125).await;
    let shell = connection.open_shell(".").await.unwrap();
    let result = shell.run_unprivileged("printf MUST_NOT_EXECUTE", Duration::from_secs(1)).await;
    let reusable = shell.is_reusable().await;
    finish_peer(&connection, task).await;
    assert!(matches!(result, Err(UnprivilegedShellError::Rejected(SshError::InvalidInput(_)))), "{result:?}");
    assert!(reusable, "a proven guard rejection keeps the safe probe channel usable");
    assert!(!String::from_utf8_lossy(&received.lock().unwrap()).contains("MUST_NOT_EXECUTE"));
}

#[tokio::test]
async fn privilege_fence_initializes_once_for_sequential_commands() {
    let (connection, task, _, fence_runs, _) = connect_fenced_peer(0, 2 * 1024 * 1024, false, 0).await;
    let shell = connection.open_shell(".").await.unwrap();
    for command in ["export NOMI_TEST=one", "printf second_command"] {
        assert_eq!(shell.run_unprivileged(command, Duration::from_secs(1)).await.unwrap().exit_code, 0);
    }
    finish_peer(&connection, task).await;
    assert_eq!(fence_runs.load(std::sync::atomic::Ordering::Relaxed), 1);
}

pub(super) async fn finish_peer(
    connection: &SshConnection,
    task: tokio::task::JoinHandle<Result<(), russh::Error>>,
) {
    connection.disconnect().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("server stopped")
        .unwrap();
    match result {
        Ok(()) => {}
        Err(russh::Error::IO(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
            ) => {}
        Err(error) => panic!("unexpected SSH peer failure: {error}"),
    }
}

#[tokio::test]
async fn a_failed_initial_cd_does_not_publish_a_shell_in_the_wrong_directory() {
    let (connection, task) = connect_peer(2).await;
    let result = connection.open_shell("/requested-but-unavailable").await;
    finish_peer(&connection, task).await;
    assert!(
        matches!(result, Err(SshError::InvalidInput(ref detail)) if detail.contains("/requested-but-unavailable")),
        "a ready sentinel with failed cd must not produce a usable shell"
    );
}

#[tokio::test]
async fn a_successful_initial_cd_still_publishes_a_usable_shell() {
    let (connection, task) = connect_peer(0).await;
    let result = connection.open_shell("/requested-directory").await;
    finish_peer(&connection, task).await;
    assert!(result.is_ok());
}
