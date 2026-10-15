//! Untrusted-certificate endpoint using existing PowerShell/.NET, with strict
//! certificate-delivery preflight and acknowledged temporary native-key cleanup.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) struct Fixture {
    child: Child,
    directory: Option<tempfile::TempDir>,
    counter: PathBuf,
    stop: PathBuf,
    cleanup: PathBuf,
    port: u16,
    exited: bool,
    cleanup_ack: Option<Value>,
}

impl Fixture {
    pub async fn new() -> Result<Self, String> {
        tokio::task::spawn_blocking(Self::start)
            .await
            .map_err(|_| "TLS fixture startup worker failed")?
    }

    fn start() -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix("nomifun-native-tls-")
            .tempdir()
            .map_err(|_| "TLS fixture temporary directory failed")?;
        let script = directory.path().join("tls.ps1");
        let counter = directory.path().join("counter.json");
        let stop = directory.path().join("stop.marker");
        let cleanup = directory.path().join("cleanup.json");
        std::fs::write(&script, include_str!("browser_navigation_tls.ps1"))
            .map_err(|_| "TLS fixture script preparation failed")?;
        let powershell = std::env::var_os("NOMIFUN_BROWSER_TLS_POWERSHELL")
            .unwrap_or_else(|| "pwsh.exe".into());
        let child = Command::new(powershell)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(script)
            .arg("-CounterPath")
            .arg(&counter)
            .arg("-StopPath")
            .arg(&stop)
            .arg("-CleanupPath")
            .arg(&cleanup)
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "Existing PowerShell/.NET TLS fixture is unavailable")?;
        let mut fixture = Self {
            child,
            directory: Some(directory),
            counter,
            stop,
            cleanup,
            port: 0,
            exited: false,
            cleanup_ack: None,
        };
        let stdout = fixture
            .child
            .stdout
            .take()
            .ok_or("TLS fixture readiness pipe missing")?;
        let mut ready = String::new();
        BufReader::new(stdout)
            .read_line(&mut ready)
            .map_err(|_| "TLS fixture readiness failed")?;
        fixture.port = ready
            .trim()
            .strip_prefix("TLS_FIXTURE_READY:")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .ok_or("Controlled TLS certificate-delivery preflight failed")?;
        let initial = fixture.witness()?;
        if initial["tls_client_hellos"] != 0 || initial["http_requests"] != 0 {
            return Err("TLS preflight contaminated the browser request counters".into());
        }
        Ok(fixture)
    }

    pub fn url(&self) -> String {
        format!("https://127.0.0.1:{}/native-tls-fixture", self.port)
    }

    pub fn witness(&self) -> Result<Value, String> {
        let witness = read_json(&self.counter)?;
        if witness["preflight_certificate_delivered"] != true
            || witness["preflight_untrusted_root"] != true
            || witness["preflight_hostname_invalid"] != false
            || witness["preflight_client_authenticated"] != false
            || witness["preflight_excluded_from_browser_counts"] != true
            || witness["certificate_has_private_key"] != true
            || witness["temporary_native_key_import"] != true
            || witness["certificate_file_persisted"] != false
            || witness["trust_store_modified"] != false
            || witness["browser_certificate_policy_modified"] != false
            || ["tls_client_hellos", "http_requests", "completed_http_documents",
                "server_authenticated_connections", "server_no_credentials_failures"]
                .iter()
                .any(|field| witness[*field].as_u64().is_none())
        {
            return Err("TLS fixture certificate-delivery witness malformed".into());
        }
        Ok(witness)
    }

    fn stop_owned(&mut self) -> Result<Value, String> {
        if self.exited {
            return self
                .cleanup_ack
                .clone()
                .ok_or_else(|| "TLS helper exited without native-key cleanup acknowledgement".into());
        }
        std::fs::write(&self.stop, b"STOP")
            .map_err(|_| "Owned TLS helper stop signal failed")?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|_| "TLS helper exit probe failed")? {
                self.exited = true;
                let ack = read_json(&self.cleanup)?;
                if !status.success()
                    || ack["certificate_disposed"] != true
                    || ack["rsa_disposed"] != true
                    || ack["temporary_native_key_removed"] != true
                {
                    return Err("TLS helper did not acknowledge exact temporary native-key cleanup".into());
                }
                self.cleanup_ack = Some(ack.clone());
                return Ok(ack);
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                self.exited = true;
                return Err("TLS helper cleanup deadline; forced termination left native-key cleanup unknown".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn close(mut self) -> Result<Value, String> {
        let ack = self.stop_owned()?;
        self.directory
            .take()
            .ok_or("TLS fixture ownership missing")?
            .close()
            .map_err(|_| "TLS fixture directory cleanup failed")?;
        Ok(json!({"managed_cleanup": ack, "owned_directory_removed": true,
            "forced_termination": false}))
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|_| "TLS fixture witness unavailable")?;
    if bytes.len() > 8192 {
        return Err("TLS fixture witness exceeds bound".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "TLS fixture witness malformed")?;
    if !value.is_object() {
        return Err("TLS fixture witness is not an object".into());
    }
    Ok(value)
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.stop_owned().is_err() {
            if let Some(directory) = self.directory.take() {
                // Keep failure evidence. A killed child cannot prove managed
                // private-key disposal; never label that path as cleaned.
                let _ = directory.keep();
                eprintln!("NAVIGATION_TLS_CLEANUP_UNKNOWN owned_directory_retained=true");
            }
        }
    }
}
