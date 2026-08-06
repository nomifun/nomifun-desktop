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

/// Signal one process. Callers pass a verified, still-live pid — never a negative
/// value, because a process-group signal from a test fixture is one snapshot race
/// away from killing something that was never ours. Declared locally rather than
/// pulling `libc` into this crate's dev-dependencies, mirroring `nomifun-mcp`'s
/// test helper.
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
///
/// Every descendant is re-verified immediately before it is signalled. We hold a
/// `Child` for `root`, so its pid cannot be recycled while we work, but the
/// descendants are pids we merely observed in `/proc`: one of them can exit in
/// the window between the snapshot and the signal, and on a busy machine the
/// kernel hands that number straight to somebody else's process. Signalling
/// blind there means SIGKILL to an innocent bystander — a developer's shell, or
/// a compiler job. Checking that the pid still names an sshd with the same
/// parent costs one `/proc` read and removes that whole class of accident.
///
/// For the same reason there is no process-*group* signal here. It would be
/// redundant on Linux (the descendants have already left the group via `setsid`)
/// and it is the one call that could take out an entire unrelated group at once.
fn kill_tree(root: i32) {
    let table = process_table();
    for (pid, ppid) in descendants_deepest_first(root, &table) {
        if still_sshd_child_of(pid, ppid) {
            signal(pid, SIGKILL);
        }
    }
    signal(root, SIGKILL);
}

/// True when `pid` is still an sshd process whose parent is still `expected_ppid`
/// — i.e. still the process we saw in the snapshot, not a recycled number.
fn still_sshd_child_of(pid: i32, expected_ppid: i32) -> bool {
    let Some((comm, ppid)) = read_comm_and_ppid(pid) else {
        return false; // already gone; nothing to kill and nothing to risk
    };
    ppid == expected_ppid && comm.contains("sshd")
}

/// Descendants of `root`, deepest generation first, paired with the parent we
/// observed them under so the kill path can re-verify them.
fn descendants_deepest_first(root: i32, table: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut generations: Vec<Vec<(i32, i32)>> = vec![vec![(root, 0)]];
    // sshd's tree is listener → session → session-child; the bound just keeps a
    // pathological /proc snapshot from looping.
    while generations.len() < 8 {
        let parents: Vec<i32> = generations
            .last()
            .expect("seeded with the root")
            .iter()
            .map(|(pid, _)| *pid)
            .collect();
        let children: Vec<(i32, i32)> = table
            .iter()
            .filter(|(pid, ppid)| parents.contains(ppid) && *pid != root)
            .map(|(pid, ppid)| (*pid, *ppid))
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
        if let Some((_, ppid)) = read_comm_and_ppid(pid) {
            table.push((pid, ppid));
        }
    }
    table
}

/// `(comm, ppid)` straight from `/proc/<pid>/stat`, or `None` if the process is
/// gone. `comm` may contain spaces and parentheses, so the fields after it are
/// only unambiguous from the LAST ')': then state, then ppid.
fn read_comm_and_ppid(pid: i32) -> Option<(String, i32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (before_close, after_comm) = stat.rsplit_once(')')?;
    let comm = before_close.split_once('(').map(|(_, c)| c)?.to_string();
    let ppid = after_comm.split_whitespace().nth(1)?.parse::<i32>().ok()?;
    Some((comm, ppid))
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
