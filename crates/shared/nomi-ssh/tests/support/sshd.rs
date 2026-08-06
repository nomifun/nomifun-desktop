//! Throwaway sshd fixture for integration tests. Spins a pubkey-only sshd on a
//! free high port with its own host key, its own authorized_keys, and its own
//! client key — never touching the developer's real ~/.ssh. Returns `None` when
//! sshd is unavailable so callers self-skip honestly instead of failing.
//!
//! [`start_cert_sshd`] is the same host configured with `TrustedUserCAKeys`
//! pointing at a CA the fixture generates, plus three certificates issued off it
//! (good / wrong principal / already expired) so the certificate branch can be
//! exercised end to end.
//!
//! The exact command sequence here was verified by hand against
//! OpenSSH 10.2p1 on this machine before being committed.
#![allow(dead_code)]

use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Certificates issued by the fixture's own CA, all for the client key.
struct Certs {
    /// Valid now, principal = the connecting username.
    valid: PathBuf,
    /// Valid now, principal = somebody else.
    other_principal: PathBuf,
    /// Correct principal, validity window already in the past.
    expired: PathBuf,
}

pub struct TestSshd {
    pub port: u16,
    pub username: String,
    client_key_path: PathBuf,
    /// A second key the server does *not* authorize — for negative cases.
    spare_key_path: PathBuf,
    known_hosts_path: PathBuf,
    /// Only populated by [`start_cert_sshd`].
    certs: Option<Certs>,
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
    /// A key this host will never accept — neither in `authorized_keys` nor
    /// certified by the CA.
    pub fn spare_key_path(&self) -> &PathBuf {
        &self.spare_key_path
    }
    pub fn spare_key_pem(&self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::fs::read_to_string(&self.spare_key_path).expect("spare key"))
    }

    fn certs(&self) -> &Certs {
        self.certs
            .as_ref()
            .expect("this fixture was not started with start_cert_sshd")
    }
    /// Certificate for the client key that this host should accept.
    pub fn client_cert(&self) -> String {
        read_cert(&self.certs().valid)
    }
    /// Same key and CA, issued to a principal that is not our username.
    pub fn cert_for_another_principal(&self) -> String {
        read_cert(&self.certs().other_principal)
    }
    /// Same key, CA and principal, but its validity window has already closed.
    pub fn expired_cert(&self) -> String {
        read_cert(&self.certs().expired)
    }
}

fn read_cert(path: &Path) -> String {
    std::fs::read_to_string(path).expect("certificate")
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
    start_sshd(false)
}

/// Start an sshd that trusts the fixture's own CA (`TrustedUserCAKeys`) and
/// authorizes **no** bare keys, so a successful login proves the certificate was
/// accepted rather than the plain public key.
pub fn start_cert_sshd() -> Option<TestSshd> {
    start_sshd(true)
}

fn start_sshd(with_ca: bool) -> Option<TestSshd> {
    let sshd = which_sshd()?;
    let tmp = TempDir::new().ok()?;
    let dir = tmp.path();
    let username = whoami();

    let host_key = dir.join("host_ed25519");
    let client_key = dir.join("client_ed25519");
    let spare_key = dir.join("spare_ed25519");
    keygen(&host_key)?;
    keygen(&client_key)?;
    keygen(&spare_key)?;

    let authorized_keys = dir.join("authorized_keys");
    if with_ca {
        // Deliberately empty: only the certificate may open this door.
        std::fs::write(&authorized_keys, "").ok()?;
    } else {
        std::fs::copy(client_key.with_extension("pub"), &authorized_keys).ok()?;
    }
    set_mode(&authorized_keys, 0o600);
    set_mode(&host_key, 0o600);
    set_mode(&client_key, 0o600);
    set_mode(&spare_key, 0o600);

    let certs = if with_ca {
        let ca = dir.join("ca_ed25519");
        keygen(&ca)?;
        set_mode(&ca, 0o600);
        Some(Certs {
            valid: issue_cert(&ca, &client_key, dir, "valid", &username, "-5m:+60m")?,
            other_principal: issue_cert(
                &ca,
                &client_key,
                dir,
                "other_principal",
                "nomi-not-this-user",
                "-5m:+60m",
            )?,
            expired: issue_cert(&ca, &client_key, dir, "expired", &username, "-40m:-20m")?,
        })
    } else {
        None
    };

    let port = pick_free_port()?;
    let pid_file = dir.join("sshd.pid");
    let cfg = dir.join("sshd_config");
    let trusted_ca = if with_ca {
        format!("TrustedUserCAKeys {}\n", dir.join("ca_ed25519.pub").display())
    } else {
        String::new()
    };
    let cfg_text = format!(
        "Port {port}\n\
         ListenAddress 127.0.0.1\n\
         HostKey {host}\n\
         PidFile {pid}\n\
         AuthorizedKeysFile {ak}\n\
         {trusted_ca}\
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
        username,
        client_key_path: client_key,
        spare_key_path: spare_key,
        known_hosts_path: known_hosts,
        certs,
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

/// Sign `key`'s public half with `ca` under a fresh name, so several
/// certificates can coexist for the same key (`ssh-keygen -s` always writes
/// `<input minus .pub>-cert.pub`, which would otherwise collide). Returns the
/// path of the emitted `*-cert.pub`.
fn issue_cert(
    ca: &Path,
    key: &Path,
    dir: &Path,
    name: &str,
    principal: &str,
    validity: &str,
) -> Option<PathBuf> {
    let target = dir.join(format!("{name}.pub"));
    std::fs::copy(key.with_extension("pub"), &target).ok()?;
    let status = Command::new("ssh-keygen")
        .arg("-s")
        .arg(ca)
        .args(["-I", "nomi-test-cert", "-n", principal, "-V", validity])
        .arg(&target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    status.success().then_some(())?;
    let cert = dir.join(format!("{name}-cert.pub"));
    cert.is_file().then_some(cert)
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
