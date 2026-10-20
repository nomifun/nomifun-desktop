//! Finite, desktop-action-free package self-check over the real worker channel.

use anyhow::{Context, ensure};
use cua_driver_sdk::worker::{ChannelRequest, ChannelResponse, PRIVATE_WORKER_PROTOCOL_VERSION};
use cua_driver_sdk::{ConfiguredDriverOptions, RuntimeAuthorizationOptions, SessionPermissionMode};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// This owns the exact child and its pipes. Failure or a test panic always
/// terminates and reaps that child, without discovering other desktop owners.
pub struct WorkerProbe {
    child: Child,
    input: Option<ChildStdin>,
    responses: Receiver<std::io::Result<String>>,
    reader: Option<std::thread::JoinHandle<()>>,
    pub generation: String,
    _root: tempfile::TempDir,
    next_request_id: u64,
}

pub fn configured_driver() -> ConfiguredDriverOptions {
    ConfiguredDriverOptions {
        claude_code_compatibility: false,
        authorization: RuntimeAuthorizationOptions {
            allowed_modes: vec![SessionPermissionMode::Standard],
            compatibility_mode: SessionPermissionMode::Standard,
            compatibility_capability_manifest_path: None,
            compatibility_bounded_manifest_path: None,
            unrestricted_acknowledged: false,
            max_session_ttl_seconds: 86_400,
            max_idle_ttl_seconds: 3_600,
        },
    }
}

impl WorkerProbe {
    pub fn spawn(binary: &Path) -> anyhow::Result<Self> {
        let root = tempfile::Builder::new()
            .prefix("nomifun-computer-worker-protocol-")
            .tempdir()?;
        let generation = uuid::Uuid::new_v4().to_string();
        let mut command = Command::new(binary);
        command
            .args(["__private-worker", "--generation", &generation])
            .env_clear()
            .current_dir(root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for name in ["PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "LANG"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        // The pinned SDK also reads HOME/USERPROFILE/.cua-driver and Linux
        // libei uses XDG_CONFIG_HOME. Isolate every writable/config root,
        // including sources the SDK's private-client allowlist does not cover.
        for name in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "TEMP",
            "TMP",
            "TMPDIR",
        ] {
            command.env(name, root.path());
        }
        command
            .env("CUA_DRIVER_RS_HOME", root.path().join("computer-driver"))
            .env(
                "CUA_DRIVER_CURSOR_THEME_DIR",
                root.path().join("cursor-themes"),
            )
            .env("CUA_DRIVER_RS_ENABLE_WAYLAND", "0")
            .env("NO_AT_BRIDGE", "1")
            .env("CUA_DRIVER_RS_DISABLE_A11Y_ADVERTISE", "1");
        #[cfg(target_os = "linux")]
        command
            // tempfile creates this private runtime directory with mode 0700.
            .env("XDG_RUNTIME_DIR", root.path())
            // A present address prevents upstream's /run/user and /proc bus
            // discovery. This deliberately absent socket cannot reach a user bus.
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", root.path().join("no-session-bus").display()),
            );
        let mut child = command.spawn().context("spawn isolated private worker")?;
        let input = child.stdin.take().context("private worker stdin")?;
        let stdout = child.stdout.take().context("private worker stdout")?;
        let (sender, responses) = channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            responses,
            reader: Some(reader),
            generation,
            _root: root,
            next_request_id: 0,
        })
    }

    pub fn request(
        &mut self,
        operation: &str,
        arguments: Option<Value>,
        name: Option<&str>,
        session_handle: Option<&str>,
    ) -> ChannelRequest {
        let request = ChannelRequest {
            protocol_version: PRIVATE_WORKER_PROTOCOL_VERSION,
            request_id: self.next_request_id,
            generation: self.generation.clone(),
            operation: operation.to_owned(),
            name: name.map(str::to_owned),
            arguments,
            session_handle: session_handle.map(str::to_owned),
        };
        self.next_request_id += 1;
        request
    }

    pub fn exchange(&mut self, request: &ChannelRequest) -> anyhow::Result<ChannelResponse> {
        self.write_line(&serde_json::to_string(request)?)?;
        let response = self.response()?;
        ensure!(
            response.protocol_version == PRIVATE_WORKER_PROTOCOL_VERSION,
            "response protocol mismatch"
        );
        ensure!(
            response.request_id == request.request_id,
            "response request id mismatch"
        );
        ensure!(
            response.generation == self.generation,
            "response worker generation mismatch"
        );
        Ok(response)
    }

    pub fn write_line(&mut self, line: &str) -> anyhow::Result<()> {
        let input = self.input.as_mut().context("private worker stdin closed")?;
        writeln!(input, "{line}")?;
        input.flush()?;
        Ok(())
    }

    pub fn response(&self) -> anyhow::Result<ChannelResponse> {
        let line = self
            .responses
            .recv_timeout(PROBE_TIMEOUT)
            .context("private worker response timeout/EOF")??;
        serde_json::from_str(&line).context("decode private worker response")
    }

    pub fn initialize(&mut self) -> anyhow::Result<ChannelResponse> {
        let request = self.request(
            "initialize",
            Some(serde_json::to_value(
                cua_driver_sdk::worker::WorkerInitialization {
                    configured_driver: configured_driver(),
                    host_bundle_id: "com.nomifun.computer-worker.self-check".into(),
                },
            )?),
            None,
            None,
        );
        let response = self.exchange(&request)?;
        if response.ok {
            let ready = response
                .result
                .as_ref()
                .context("initialize omitted result")?;
            ensure!(
                ready["pid"].as_u64() == Some(u64::from(self.child.id())),
                "initialize pid differs from spawned worker"
            );
            ensure!(
                ready["host_bundle_id"] == "com.nomifun.computer-worker.self-check",
                "initialize host identity mismatch"
            );
        }
        Ok(response)
    }

    pub fn wait(&mut self) -> anyhow::Result<ExitStatus> {
        self.input.take();
        let deadline = Instant::now() + PROBE_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            ensure!(
                Instant::now() < deadline,
                "private worker did not exit after channel closure"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for WorkerProbe {
    fn drop(&mut self) {
        self.input.take();
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub fn self_check() -> anyhow::Result<()> {
    let mut worker = WorkerProbe::spawn(&std::env::current_exe()?)?;
    let ready = worker.initialize()?;
    ensure!(
        ready.ok
            && ready
                .result
                .as_ref()
                .and_then(|v| v.get("ready"))
                .and_then(Value::as_bool)
                == Some(true),
        "worker initialization failed: {:?}",
        ready.error
    );
    let list = worker.request("list", None, None, None);
    let list = worker.exchange(&list)?;
    ensure!(list.ok, "worker list failed: {:?}", list.error);
    let count = list
        .result
        .as_ref()
        .and_then(|v| v.get("tools"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .context("list omitted tools")?;
    ensure!(count > 0, "worker returned an empty tool inventory");
    let shutdown = worker.request("shutdown", None, None, None);
    let shutdown = worker.exchange(&shutdown)?;
    ensure!(
        shutdown.ok
            && shutdown
                .result
                .as_ref()
                .and_then(|v| v.get("shutdown"))
                .and_then(Value::as_bool)
                == Some(true),
        "worker shutdown failed: {:?}",
        shutdown.error
    );
    ensure!(
        worker.wait()?.success(),
        "private worker exited unsuccessfully"
    );
    println!(
        "{}",
        serde_json::json!({"ok":true,"protocol_version":PRIVATE_WORKER_PROTOCOL_VERSION,"upstream_commit":env!("CUA_DRIVER_GIT_SHA"),"tool_count":count})
    );
    Ok(())
}
