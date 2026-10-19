//! Controlled protocol peers, not sshd/sudo/OS execution evidence. These tests
//! use the real SSH and SFTP handshakes, channel receivers and close messages.
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use russh::{Channel, ChannelId, Pty, server};

#[derive(Default)]
struct PeerEvidence {
    waiting: tokio::sync::Notify,
    waiting_seen: std::sync::atomic::AtomicBool,
    interrupted: Mutex<Vec<ChannelId>>,
}

#[derive(Default)]
struct Input { bytes: Vec<u8>, sftp: bool, ordinal: usize }
struct Peer {
    channels: HashMap<ChannelId, Input>,
    shells: usize,
    wait_validation: bool,
    deny_temporary_exit_proof: bool,
    evidence: Arc<PeerEvidence>,
}

fn nonce(input: &[u8]) -> Option<u64> {
    let input = String::from_utf8_lossy(input);
    let suffix = input.split_once("__NOMI_END_")?.1;
    suffix.split_once("__")?.0.parse().ok()
}

impl server::Handler for Peer {
    type Error = russh::Error;
    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> { Ok(server::Auth::Accept) }
    async fn channel_open_session(&mut self, channel: Channel<server::Msg>, reply: server::ChannelOpenHandle,
        _: &mut server::Session) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), Input::default()); reply.accept().await; Ok(())
    }
    async fn pty_request(&mut self, channel: ChannelId, _: &str, _: u32, _: u32, _: u32, _: u32,
        _: &[(Pty, u32)], session: &mut server::Session) -> Result<(), Self::Error> {
        session.channel_success(channel)?; Ok(())
    }
    async fn shell_request(&mut self, channel: ChannelId, session: &mut server::Session) -> Result<(), Self::Error> {
        self.shells += 1; self.channels.get_mut(&channel).unwrap().ordinal = self.shells;
        session.channel_success(channel)?; Ok(())
    }
    async fn subsystem_request(&mut self, channel: ChannelId, name: &str, session: &mut server::Session) -> Result<(), Self::Error> {
        assert_eq!(name, "sftp"); self.channels.get_mut(&channel).unwrap().sftp = true;
        session.channel_success(channel)?; Ok(())
    }
    async fn signal(&mut self, channel: ChannelId, signal: russh::Sig, session: &mut server::Session) -> Result<(), Self::Error> {
        if matches!(signal, russh::Sig::INT) { self.evidence.interrupted.lock().unwrap().push(channel); }
        if matches!(signal, russh::Sig::TERM)
            && !(self.deny_temporary_exit_proof && self.channels[&channel].ordinal > 1) {
            session.exit_status_request(channel, 143)?; session.close(channel)?;
        }
        Ok(())
    }
    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut server::Session) -> Result<(), Self::Error> {
        let input = self.channels.get_mut(&channel).unwrap(); input.bytes.extend_from_slice(data);
        if input.sftp {
            // Only the real client's INIT handshake is implemented; no remote
            // filesystem is fabricated and no file operation is used here.
            if input.bytes.len() >= 9 {
                assert_eq!(&input.bytes[..9], &[0, 0, 0, 5, 1, 0, 0, 0, 3]);
                session.data(channel, vec![0, 0, 0, 5, 2, 0, 0, 0, 3])?; input.bytes.clear();
            }
            return Ok(());
        }
        if data == b"exit\n" {
            if !(self.deny_temporary_exit_proof && input.ordinal > 1) {
                session.exit_status_request(channel, 0)?; session.close(channel)?;
            }
            input.bytes.clear(); return Ok(());
        }
        let Some(nonce) = nonce(&input.bytes) else { return Ok(()); };
        let text = String::from_utf8_lossy(&input.bytes);
        let validation = text.contains("sudo -k -n -v");
        let execution = text.contains("CONTROLLED_SUDO_WAIT");
        if (validation && self.wait_validation) || execution {
            session.data(channel, if validation { b"validation-partial\n".to_vec() } else { b"execution-partial\n".to_vec() })?;
            self.evidence.waiting_seen.store(true, std::sync::atomic::Ordering::Release);
            self.evidence.waiting.notify_waiters();
        } else {
            session.data(channel, format!("sibling-ok\n__NOMI_END_{nonce}__0__/controlled\n").into_bytes())?;
        }
        input.bytes.clear(); Ok(())
    }
}

struct Fixture {
    handle: Arc<SshConnectionHandle>, credential: DecryptedCredential,
    evidence: Arc<PeerEvidence>, worker: tokio::task::JoinHandle<Result<(), russh::Error>>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    async fn connect(wait_validation: bool, deny_temporary_exit_proof: bool) -> Self {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port(); let evidence = Arc::new(PeerEvidence::default());
        let peer = Peer { channels: HashMap::new(), shells: 0, wait_validation, deny_temporary_exit_proof, evidence: evidence.clone() };
        let config = Arc::new(server::Config {
            keys: vec![russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[54; 32]).into()],
            inactivity_timeout: Some(Duration::from_secs(30)), ..Default::default()
        });
        let worker = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            server::run_stream(config, stream, peer).await?.await
        });
        let credential = DecryptedCredential { host: "127.0.0.1".into(), port, username: "controlled".into(),
            auth_type: "password".into(), password: Some(Zeroizing::new("controlled-public-fixture".into())),
            private_key: None, passphrase: None, certificate: None, sudo_password: None };
        let directory = tempfile::tempdir().unwrap();
        let handle = Arc::new(SshConnectionHandle::connect(clone_credential(&credential), directory.path().join("known_hosts"), ".").await.unwrap());
        Self { handle, credential, evidence, worker, _directory: directory }
    }
    async fn wait_running(&self) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let notified = self.evidence.waiting.notified();
                if self.evidence.waiting_seen.load(std::sync::atomic::Ordering::Acquire) { return; }
                notified.await;
            }
        }).await.unwrap();
    }
    async fn finish(self) {
        self.handle.conn().disconnect().await.unwrap();
        match tokio::time::timeout(Duration::from_secs(3), self.worker).await.unwrap().unwrap() {
            Ok(()) => {},
            Err(russh::Error::IO(error)) if matches!(error.kind(), std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe) => {},
            Err(error) => panic!("controlled SSH peer failed: {error}"),
        }
    }
}

fn clone_credential(credential: &DecryptedCredential) -> DecryptedCredential {
    DecryptedCredential { host: credential.host.clone(), port: credential.port, username: credential.username.clone(),
        auth_type: credential.auth_type.clone(), password: credential.password.clone(), private_key: None,
        passphrase: None, certificate: None, sudo_password: None }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn temporary_sudo_retirement_preempts_validation_and_execution_and_preserves_other_handle() {
    for validation in [true, false] {
        let original = Fixture::connect(validation, false).await; let other = Fixture::connect(false, false).await;
        let handle = original.handle.clone(); let credential = clone_credential(&original.credential);
        let run = tokio::spawn(async move { handle.run_ephemeral_sudo(".", "CONTROLLED_SUDO_WAIT", 60_000, &credential).await });
        original.wait_running().await;
        assert!(original.handle.shell().run("primary-before-close", Duration::from_secs(1)).await.is_ok());
        assert!(other.handle.shell().run("sibling-before-close", Duration::from_secs(1)).await.is_ok());
        original.handle.request_retirement();
        let result = tokio::time::timeout(Duration::from_secs(5), run).await.unwrap().unwrap();
        let expected = if validation { "validation-partial" } else { "execution-partial" };
        assert!(result.as_ref().err().is_some_and(|error| error.to_string().contains(expected)), "original partial output must survive retirement: {result:?}");
        assert!(!original.evidence.interrupted.lock().unwrap().is_empty(), "the actual temporary receiver received its owner's interrupt");
        assert!(matches!(original.handle.shell().run("MUST_NOT_RUN", Duration::from_secs(1)).await, Err(SshError::Retired { .. })));
        assert!(other.handle.shell().run("sibling-after-close", Duration::from_secs(1)).await.is_ok());
        assert!(original.handle.close_shells(Duration::from_secs(1)).await.iter().all(|proof|proof.is_reaped()));
        assert!(original.handle.channels_reaped());
        assert!(other.handle.close_shells(Duration::from_secs(1)).await.iter().all(|proof|proof.is_reaped()));
        original.finish().await; other.finish().await;
    }
}

#[tokio::test]
async fn temporary_shell_close_proofs_are_aggregated_without_upgrading_missing_original_exit() {
    let original = Fixture::connect(false, true).await;
    let temporary = original.handle.conn().open_shell(".").await.unwrap();
    original.handle.temporary_shells.lock().unwrap().push(temporary);
    original.handle.request_retirement();
    let proofs = original.handle.close_shells(Duration::from_millis(40)).await;
    assert_eq!(proofs.len(), 2); assert!(proofs[0].is_reaped()); assert!(!proofs[1].is_reaped());
    assert!(!original.handle.channels_reaped(), "the primary's exit cannot stand in for an unconfirmed temporary shell");
    assert_eq!(original.handle.temporary_shells.lock().unwrap().len(), 1, "retain the exact original receiver");
    original.finish().await;
}
