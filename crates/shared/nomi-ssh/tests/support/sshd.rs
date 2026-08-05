//! Throwaway sshd fixture for integration tests. Spins a pubkey-only sshd on a
//! free high port with its own host key, its own authorized_keys, and its own
//! client key — never touching the developer's real ~/.ssh. Returns `None` when
//! sshd is unavailable so callers self-skip honestly instead of failing.
//!
//! The exact command sequence here was verified by hand against
//! OpenSSH 10.2p1 on this machine before being committed.
#![allow(dead_code)]

use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tempfile::TempDir;

pub struct TestSshd {
    pub port: u16,
    pub username: String,
    client_key_path: PathBuf,
    known_hosts_path: PathBuf,
    child: Child,
    _tmp: TempDir,
}

impl TestSshd {
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn known_hosts_path(&self) -> PathBuf {
        self.known_hosts_path.clone()
    }
    /// PEM body of the client private key, for `Auth::PrivateKey`.
    pub fn client_key_pem(&self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::fs::read_to_string(&self.client_key_path).expect("client key"))
    }
    /// Path to the client private key, for building an `ssh -i` command.
    pub fn client_key_path(&self) -> &PathBuf {
        &self.client_key_path
    }
}

impl Drop for TestSshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn which_sshd() -> Option<PathBuf> {
    for candidate in ["/usr/sbin/sshd", "/usr/bin/sshd", "/sbin/sshd"] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn pick_free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    Some(listener.local_addr().ok()?.port())
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".to_string())
}

fn wait_until_listening(port: u16) -> Option<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Start a pubkey-auth sshd. Returns `None` if `sshd`/`ssh-keygen` are missing
/// or the environment can't run it (honest skip, never a fake pass).
pub fn start_pubkey_sshd() -> Option<TestSshd> {
    let sshd = which_sshd()?;
    let tmp = TempDir::new().ok()?;
    let dir = tmp.path();

    let host_key = dir.join("host_ed25519");
    let client_key = dir.join("client_ed25519");
    keygen(&host_key)?;
    keygen(&client_key)?;

    let authorized_keys = dir.join("authorized_keys");
    std::fs::copy(client_key.with_extension("pub"), &authorized_keys).ok()?;
    set_mode(&authorized_keys, 0o600);
    set_mode(&host_key, 0o600);
    set_mode(&client_key, 0o600);

    let port = pick_free_port()?;
    let pid_file = dir.join("sshd.pid");
    let cfg = dir.join("sshd_config");
    let cfg_text = format!(
        "Port {port}\n\
         ListenAddress 127.0.0.1\n\
         HostKey {host}\n\
         PidFile {pid}\n\
         AuthorizedKeysFile {ak}\n\
         PubkeyAuthentication yes\n\
         PasswordAuthentication no\n\
         UsePAM no\n\
         StrictModes no\n\
         Subsystem sftp internal-sftp\n\
         LogLevel ERROR\n",
        host = host_key.display(),
        pid = pid_file.display(),
        ak = authorized_keys.display(),
    );
    std::fs::write(&cfg, cfg_text).ok()?;

    let child = Command::new(sshd)
        .arg("-f")
        .arg(&cfg)
        .arg("-D")
        .arg("-e")
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let known_hosts = dir.join("known_hosts");
    // Pre-seed nothing: the connection test learns the key under AcceptNew.

    wait_until_listening(port)?;

    Some(TestSshd {
        port,
        username: whoami(),
        client_key_path: client_key,
        known_hosts_path: known_hosts,
        child,
        _tmp: tmp,
    })
}

fn keygen(path: &PathBuf) -> Option<()> {
    let status = Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-N", "", "-q", "-f"])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .status()
        .ok()?;
    status.success().then_some(())
}

#[cfg(unix)]
fn set_mode(path: &PathBuf, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}
#[cfg(not(unix))]
fn set_mode(_path: &PathBuf, _mode: u32) {}

/// Convenience: connect a `SshConnection` to the fixture using its client key
/// under AcceptNew. Panics on failure (tests want that).
pub async fn connect(sshd: &TestSshd) -> nomi_ssh::connection::SshConnection {
    use nomi_ssh::connection::{HostKeyPolicy, SshConnection};
    use nomi_ssh::credential::{Auth, SshCredential};
    let cred = SshCredential {
        host: "127.0.0.1".into(),
        port: sshd.port(),
        username: sshd.username.clone(),
        auth: Auth::PrivateKey {
            pem: sshd.client_key_pem(),
            passphrase: None,
        },
    };
    SshConnection::connect(&cred, HostKeyPolicy::AcceptNew { known_hosts: sshd.known_hosts_path() })
        .await
        .expect("connect to test sshd")
}

// silence "unused" on the std::io::Write import used by future tasks
fn _touch_write(mut w: impl Write) {
    let _ = w.write_all(b"");
}
