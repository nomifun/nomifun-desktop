//! Throwaway sshd fixture for the pool's integration tests. Mirrors
//! `crates/shared/nomi-ssh/tests/support/sshd.rs` — that module is test-only in
//! another crate, so it cannot be imported and is copied here instead. Keep the
//! two in step when either changes.
//!
//! Extended beyond the nomi-ssh original with [`TestSshd::stop`] /
//! [`TestSshd::restart`], because the pool's reconnect ladder can only be
//! observed against a host that goes away and comes back **on the same port with
//! the same host key** (a new key would be a `HostKeyChanged` rejection, which is
//! deliberately terminal).
//!
//! Everything lives in a tempdir: own host key, own client key, own
//! `authorized_keys`, own `known_hosts`. The operator's real `~/.ssh` is never
//! read or written.
#![allow(dead_code)]

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tempfile::TempDir;

pub struct TestSshd {
    pub port: u16,
    pub username: String,
    sshd_binary: PathBuf,
    config_path: PathBuf,
    client_key_path: PathBuf,
    known_hosts_path: PathBuf,
    /// `None` between [`TestSshd::stop`] and [`TestSshd::restart`].
    child: Option<Child>,
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
        zeroize::Zeroizing::new(
            std::fs::read_to_string(&self.client_key_path).expect("client key"),
        )
    }
    pub fn client_key_path(&self) -> &PathBuf {
        &self.client_key_path
    }

    /// Take the host away. Kills the listener *and every process descended from
    /// it*, because OpenSSH forks a per-connection `sshd-session` that calls
    /// `setsid` — it leaves the listener's process group and session behind, so a
    /// group signal reaches the listener while every established transport stays
    /// happily alive and the reconnect ladder never starts. The parent links do
    /// survive `setsid`, so the tree is still walkable, and the tree is what owns
    /// the client's socket.
    ///
    /// Returns once the port refuses connections, so a caller may assume the host
    /// is really gone.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            kill_tree(child.id() as i32);
            let _ = child.kill();
            let _ = child.wait();
        }
        wait_until_refused(self.port);
    }

    /// Bring the same host back: same port, same host key, same
    /// `authorized_keys`, so a client that learned the key before the outage
    /// reconnects without a host-key prompt.
    pub fn restart(&mut self) -> Option<()> {
        if self.child.is_some() {
            return Some(());
        }
        self.child = Some(spawn_sshd(&self.sshd_binary, &self.config_path)?);
        wait_until_listening(self.port)
    }
}

impl Drop for TestSshd {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            kill_tree(child.id() as i32);
            let _ = child.kill();
            let _ = child.wait();
        }
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

fn wait_until_refused(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

const SIGKILL: i32 = 9;

/// Signal one process, or a whole process group when `pid` is negative. Declared
/// locally rather than pulling `libc` into this crate's dev-dependencies,
/// mirroring `nomifun-mcp`'s test helper.
#[cfg(unix)]
fn signal(pid: i32, signal: i32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid, signal) == 0 }
}
#[cfg(not(unix))]
fn signal(_pid: i32, _signal: i32) -> bool {
    false
}

/// Kill `root` and everything below it, deepest first — killing the root before
/// its children would reparent them to init and lose the trail.
fn kill_tree(root: i32) {
    for pid in descendants_deepest_first(root) {
        signal(pid, SIGKILL);
    }
    // The group signal is redundant on Linux but catches platforms where the
    // /proc walk found nothing.
    signal(-root, SIGKILL);
    signal(root, SIGKILL);
}

/// Descendants of `root`, deepest generation first. Empty when `/proc` is not
/// available.
fn descendants_deepest_first(root: i32) -> Vec<i32> {
    let table = process_table();
    let mut generations: Vec<Vec<i32>> = vec![vec![root]];
    // sshd's tree is listener → session → session-child; the bound just keeps a
    // pathological /proc snapshot from looping.
    while generations.len() < 8 {
        let parents = generations.last().expect("seeded with the root");
        let children: Vec<i32> = table
            .iter()
            .filter(|(pid, ppid)| parents.contains(ppid) && *pid != root)
            .map(|(pid, _)| *pid)
            .collect();
        if children.is_empty() {
            break;
        }
        generations.push(children);
    }
    generations.into_iter().skip(1).rev().flatten().collect()
}

/// `(pid, ppid)` for every process we can see.
fn process_table() -> Vec<(i32, i32)> {
    let mut table = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return table;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // `comm` may contain spaces and parentheses, so the fields after it are
        // only unambiguous from the LAST ')': then state, then ppid.
        let Some((_, after_comm)) = stat.rsplit_once(')') else {
            continue;
        };
        if let Some(ppid) = after_comm
            .split_whitespace()
            .nth(1)
            .and_then(|p| p.parse::<i32>().ok())
        {
            table.push((pid, ppid));
        }
    }
    table
}

/// Start sshd as the leader of its own process group so [`TestSshd::stop`] can
/// take its forked session children down with it.
fn spawn_sshd(sshd: &PathBuf, config: &PathBuf) -> Option<Child> {
    let mut command = Command::new(sshd);
    command
        .arg("-f")
        .arg(config)
        .args(["-D", "-e"])
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().ok()
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

    let child = spawn_sshd(&sshd, &cfg)?;

    // Pre-seed nothing: the first dial learns the key under AcceptNew.
    let known_hosts = dir.join("known_hosts");

    wait_until_listening(port)?;

    Some(TestSshd {
        port,
        username: whoami(),
        sshd_binary: sshd,
        config_path: cfg,
        client_key_path: client_key,
        known_hosts_path: known_hosts,
        child: Some(child),
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
