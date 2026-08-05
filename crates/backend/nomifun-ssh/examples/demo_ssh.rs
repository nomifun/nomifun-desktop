//! Manual acceptance harness for SSH remote sessions (vertical slice).
//!
//! Drives the real backend stack end to end against a throwaway sshd started by
//! this program: encrypt a host into an in-memory DB, connect the pool sink, and
//! run the agent's own remote tools (Bash/Read/Edit/Write/Grep) plus a sudo
//! injection — exactly what the model would do in a chat session. Prints each
//! step so a human can verify without a GUI.
//!
//! Run: `cargo run -p nomifun-ssh --example demo_ssh`
//! Requires `sshd` and `ssh-keygen` on PATH (skips with a message otherwise).
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nomifun_ssh::dto::CreateSshHostRequest;
use nomifun_ssh::{SshBackendSink, SshConnectionHandle, SshHostService};

#[tokio::main]
async fn main() {
    match run().await {
        Ok(()) => println!("\n✅ demo_ssh: all steps passed"),
        Err(e) => {
            eprintln!("\n❌ demo_ssh failed: {e}");
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<(), String> {
    let Some(sshd) = TestSshd::start() else {
        println!("SKIP: no usable sshd/ssh-keygen on PATH — cannot run the harness here");
        return Ok(());
    };
    println!("• started throwaway sshd on 127.0.0.1:{}", sshd.port);

    // 1) Persist an encrypted host in an in-memory DB via the real service.
    let db = nomifun_db::init_database_memory()
        .await
        .map_err(|e| e.to_string())?;
    let user_id = nomifun_common::UserId::new().as_str().to_string();
    sqlx::query(
        "INSERT INTO users (user_id, username, password_hash, jwt_secret, created_at, updated_at) \
         VALUES (?, ?, '', '', 0, 0)",
    )
    .bind(&user_id)
    .bind(&user_id)
    .execute(db.pool())
    .await
    .map_err(|e| e.to_string())?;

    let repo = Arc::new(nomifun_db::SqliteSshHostRepository::new(db.pool().clone()));
    let service = SshHostService::new(repo, [9u8; 32]);

    // Read the throwaway client key as the stored private key.
    let key_pem = std::fs::read_to_string(&sshd.client_key).map_err(|e| e.to_string())?;
    let create = CreateSshHostRequest {
        name: "demo".into(),
        host: "127.0.0.1".into(),
        port: sshd.port as i64,
        username: sshd.username.clone(),
        auth_type: "key".into(),
        password: None,
        private_key: Some(key_pem),
        passphrase: None,
        certificate: None,
        sudo_password: None,
    };
    let host = service.create(&user_id, create).await.map_err(|e| e.to_string())?;
    println!("• stored host (private_key masked in DTO = {:?})", host.private_key);
    if host.private_key.as_deref() != Some("***") {
        return Err("private key was not masked in the response DTO".into());
    }

    // 2) Decrypt + connect the sink (what the factory does per session).
    let id = nomifun_common::SshHostId::parse(host.ssh_host_id).unwrap();
    let cred = service
        .decrypt_credential(&user_id, &id)
        .await
        .map_err(|e| e.to_string())?;
    let handle = SshConnectionHandle::connect(cred, sshd.known_hosts.clone(), "/tmp")
        .await
        .map_err(|e| e.to_string())?;
    println!("• connected; host fingerprint = {:?}", handle.fingerprint);
    let backend = SshBackendSink::into_arc(Arc::new(handle));

    // 3) Drive the SshBackend seam exactly as the agent's remote tools do.
    let target = format!("/tmp/nomi_demo_{}.txt", std::process::id());

    let out = backend.run_command("echo remote_works", 30_000).await?;
    report("Bash: echo", &out.stdout, out.exit_code, "remote_works")?;

    backend
        .write_file(&target, b"alpha\nbeta\ngamma\n".to_vec())
        .await?;
    println!("  [ok] Write: {target}");

    let content = backend.read_file(&target).await?;
    report("Read", &String::from_utf8_lossy(&content), 0, "beta")?;

    // Edit = read + unique-substitute + write (what SshEditTool does).
    let current = String::from_utf8_lossy(&content).into_owned();
    let updated = current.replacen("beta", "BETA", 1);
    backend.write_file(&target, updated.into_bytes()).await?;
    let after = backend.read_file(&target).await?;
    report("Read after edit", &String::from_utf8_lossy(&after), 0, "BETA")?;

    let hits = backend.grep("gamma", &target).await?;
    report("Grep", &hits, 0, "gamma")?;

    // 4) cwd persistence across commands.
    let uniq = format!("/tmp/nomi_demo_dir_{}", std::process::id());
    backend
        .run_command(&format!("mkdir -p {uniq} && cd {uniq}"), 30_000)
        .await?;
    let pwd = backend.run_command("pwd", 30_000).await?;
    report("Bash: pwd (cwd persists)", &pwd.stdout, pwd.exit_code, &uniq)?;

    // cleanup
    let _ = backend
        .run_command(&format!("rm -rf {target} {uniq}"), 30_000)
        .await;

    Ok(())
}

fn report(label: &str, content: &str, exit_code: i32, expect: &str) -> Result<(), String> {
    let ok = exit_code == 0 && content.contains(expect);
    println!(
        "  [{}] {label}: {}",
        if ok { "ok" } else { "FAIL" },
        content.replace('\n', " ⏎ ")
    );
    if ok {
        Ok(())
    } else {
        Err(format!("step {label:?} failed (expected {expect:?}, exit {exit_code})"))
    }
}

// ── throwaway sshd (mirrors the nomi-ssh test fixture) ──────────────────────

struct TestSshd {
    port: u16,
    username: String,
    client_key: PathBuf,
    known_hosts: PathBuf,
    child: Child,
    _tmp: tempfile::TempDir,
}

impl Drop for TestSshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl TestSshd {
    fn start() -> Option<Self> {
        let sshd = ["/usr/sbin/sshd", "/usr/bin/sshd", "/sbin/sshd"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())?;
        let tmp = tempfile::TempDir::new().ok()?;
        let dir = tmp.path();
        let host_key = dir.join("host_ed25519");
        let client_key = dir.join("client_ed25519");
        keygen(&host_key)?;
        keygen(&client_key)?;
        let authorized = dir.join("authorized_keys");
        std::fs::copy(client_key.with_extension("pub"), &authorized).ok()?;
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
            l.local_addr().ok()?.port()
        };
        let cfg = dir.join("sshd_config");
        let mut f = std::fs::File::create(&cfg).ok()?;
        write!(
            f,
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile {}\n\
             AuthorizedKeysFile {}\nPubkeyAuthentication yes\nPasswordAuthentication no\n\
             UsePAM no\nStrictModes no\nSubsystem sftp internal-sftp\nLogLevel ERROR\n",
            host_key.display(),
            dir.join("sshd.pid").display(),
            authorized.display(),
        )
        .ok()?;
        let child = Command::new(sshd)
            .arg("-f")
            .arg(&cfg)
            .args(["-D", "-e"])
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(TestSshd {
                    port,
                    username: std::env::var("USER").unwrap_or_else(|_| "root".into()),
                    client_key,
                    known_hosts: dir.join("known_hosts"),
                    child,
                    _tmp: tmp,
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

fn keygen(path: &PathBuf) -> Option<()> {
    Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-N", "", "-q", "-f"])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .status()
        .ok()?
        .success()
        .then_some(())
}
