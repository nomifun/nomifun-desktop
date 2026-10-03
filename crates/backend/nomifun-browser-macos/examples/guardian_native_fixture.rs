//! No-UI Darwin fixture for the production guardian and its production client.
//!
//! This does not load CEF, call cef_shutdown, access Keychain, or prove browser
//! shutdown. Native entry/return are protocol transitions made by this fixture.
//! Run each mode under an external 60-second supervisor. The timeout mode uses
//! the unchanged production 30-second native and shared five-second cleanup
//! budgets. Every spawned process is an owned direct child of fixture Main.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("guardian_native_fixture requires native macOS");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = fixture::main() {
        eprintln!("guardian_native_fixture: {error}");
        std::process::exit(2);
    }
}

#[cfg(target_os = "macos")]
mod fixture {
    use std::{
        ffi::OsString,
        fs::{self, OpenOptions},
        io::{Read, Write},
        os::{
            fd::AsRawFd,
            unix::{fs::PermissionsExt, net::UnixStream, process::ExitStatusExt},
        },
        path::{Path, PathBuf},
        process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
        sync::Arc,
        time::{Duration, Instant},
    };

    use nomi_process_runtime::{ExactProcessIdentity, probe_process_identity};
    use nomifun_browser_macos::{
        guardian::{self, HELPER_CLEANUP_BUDGET, NATIVE_SHUTDOWN_BUDGET, RPC_TIMEOUT, Reply, Request},
        guardian_client::{GuardOwner, register_helper},
    };
    use serde_json::{Value, json};
    use tempfile::TempDir;

    const CHILD_CONTROL_BUDGET: Duration = Duration::from_secs(3);
    const FIXTURE_COMPLETION_GRACE: Duration = Duration::from_secs(3);
    const ENVIRONMENT_KEYS: [&str; 10] = [
        "PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "USER", "LOGNAME",
        "__CF_USER_TEXT_ENCODING", "MallocNanoZone", "LC_CTYPE",
    ];

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Mode { NormalReturn, Timeout, AckLoss, AbortInitialization }

    impl Mode {
        fn parse(value: &str) -> Result<Self, String> {
            match value {
                "normal_return" => Ok(Self::NormalReturn),
                "timeout" => Ok(Self::Timeout),
                "ack_loss" => Ok(Self::AckLoss),
                "abort_initialization" => Ok(Self::AbortInitialization),
                _ => Err("mode must be normal_return, timeout, ack_loss, or abort_initialization".into()),
            }
        }

        fn name(self) -> &'static str {
            match self {
                Self::NormalReturn => "normal_return",
                Self::Timeout => "timeout",
                Self::AckLoss => "ack_loss",
                Self::AbortInitialization => "abort_initialization",
            }
        }
    }

    pub fn main() -> Result<(), String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        // Match the real helper's early guardian dispatch before any runtime
        // or CEF setup. GuardOwner launches the copied generic executable here.
        if args.first().is_some_and(|arg| arg == "--nomifun-cef-guardian") {
            if args.len() != 2 { return Err("guardian mode requires exactly one socket argument".into()); }
            let socket = args[1].strip_prefix("--socket=").filter(|value| !value.is_empty())
                .ok_or("guardian mode requires --socket=<absolute path>")?;
            return guardian::run_guardian(Path::new(socket));
        }
        if args.first().is_some_and(|arg| arg == "--test-helper") {
            if args.len() != 4 { return Err("test helper requires socket, nonce, and role".into()); }
            register_helper(Path::new(&args[1]), &args[2], &args[3])?;
            return helper_commands();
        }
        if args.len() == 1 && args[0] == "--test-sibling" { return helper_commands(); }

        let (mode, evidence_root, result_path) = parse_main_args(&args)?;
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2)
            .enable_all().build().map_err(|error| format!("fixture Tokio runtime: {error}"))?;
        let _entered = runtime.enter();
        let started = Instant::now();
        let mut evidence = json!({
            "schema": 1,
            "mode": mode.name(),
            "plane": "native_macos_production_guardian_client_with_fixture_children",
            "full_cef_shutdown_proven": false,
            "cef_framework_loaded": false,
            "system_keychain_accessed": false,
            "native_call": "fixture_protocol_transition_only",
            "native_budget_ms": NATIVE_SHUTDOWN_BUDGET.as_millis(),
            "shared_helper_cleanup_budget_ms": HELPER_CLEANUP_BUDGET.as_millis(),
            "argv": args,
            "evidence_root": evidence_root,
            "result_path": result_path,
            "success": false,
        });

        let outcome = run_fixture(&runtime, mode, &evidence_root, &mut evidence);
        evidence["elapsed_ms"] = json!(started.elapsed().as_millis());
        match &outcome {
            Ok(()) => evidence["success"] = json!(true),
            Err(error) => evidence["error"] = json!(error),
        }
        let mut result_file = OpenOptions::new().write(true).create_new(true).open(&result_path)
            .map_err(|error| format!("create unique external evidence file {}: {error}", result_path.display()))?;
        serde_json::to_writer_pretty(&mut result_file, &evidence)
            .map_err(|error| format!("write fixture evidence: {error}"))?;
        result_file.write_all(b"\n").map_err(|error| format!("finish fixture evidence: {error}"))?;
        result_file.sync_all().map_err(|error| format!("sync fixture evidence: {error}"))?;
        println!("{}", json!({"mode": mode.name(), "success": outcome.is_ok(), "result": result_path}));
        outcome
    }

    fn parse_main_args(args: &[String]) -> Result<(Mode, PathBuf, PathBuf), String> {
        let mut mode = None;
        let mut evidence_root = None;
        let mut result_name = None;
        let mut index = 0;
        while index < args.len() {
            let value = args.get(index + 1).ok_or("fixture arguments require a value")?;
            match args[index].as_str() {
                "--mode" if mode.is_none() => mode = Some(Mode::parse(value)?),
                "--evidence-root" if evidence_root.is_none() => evidence_root = Some(PathBuf::from(value)),
                "--result-name" if result_name.is_none() => result_name = Some(value.clone()),
                _ => return Err("usage: --mode <normal_return|timeout|ack_loss|abort_initialization> --evidence-root <approved external root> [--result-name <new basename.json>]".into()),
            }
            index += 2;
        }
        let mode = mode.ok_or("fixture mode is required")?;
        let root = evidence_root.ok_or("explicit external evidence root is required")?;
        if !root.is_absolute() || root.file_name().and_then(|name|name.to_str())!=Some("guardian-native") || root.components().count()<3 {
            return Err("evidence root must be an explicit absolute guardian-native directory".into());
        }
        fs::create_dir_all(&root).map_err(|error| format!("create external evidence root: {error}"))?;
        let root = root.canonicalize().map_err(|error| format!("canonical evidence root: {error}"))?;
        if root.file_name().and_then(|name|name.to_str())!=Some("guardian-native") {
            return Err("evidence root must resolve to the scoped guardian-native directory".into());
        }
        let name = result_name.unwrap_or_else(|| format!("{}.json", mode.name()));
        if !name.ends_with(".json") || name.is_empty() || name.contains('/') || name.contains('\\')
            || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')) {
            return Err("result name must be a plain JSON basename".into());
        }
        let result = root.join(name);
        if result.exists() { return Err("result file exists; supply a new --result-name for a rerun".into()); }
        Ok((mode, root, result))
    }

    fn helper_commands() -> Result<(), String> {
        println!("READY {}", std::process::id());
        std::io::stdout().flush().map_err(|error| format!("helper READY: {error}"))?;
        let mut input = std::io::stdin().lock();
        let mut line = Vec::new();
        loop {
            line.clear();
            loop {
                let mut byte = [0_u8; 1];
                match input.read(&mut byte) {
                    Ok(0) => return Ok(()),
                    Ok(_) if byte[0] == b'\n' => break,
                    Ok(_) if line.len() < 32 => line.push(byte[0]),
                    Ok(_) => return Err("fixture command is too long".into()),
                    Err(error) => return Err(format!("helper command: {error}")),
                }
            }
            match line.as_slice() {
                b"PING" => println!("ALIVE {}", std::process::id()),
                b"EXIT" => return Ok(()),
                _ => return Err("unknown fixture helper command".into()),
            }
            std::io::stdout().flush().map_err(|error| format!("helper response: {error}"))?;
        }
    }

    struct OwnedFixtureChild {
        child: Child,
        input: Option<ChildStdin>,
        output: ChildStdout,
        identity: ExactProcessIdentity,
        reaped: Option<ExitStatus>,
    }

    impl OwnedFixtureChild {
        fn spawn(executable: &Path, args: &[OsString]) -> Result<Self, String> {
            let mut command = Command::new(executable);
            command.env_clear().envs(ENVIRONMENT_KEYS.iter().filter_map(|key| {
                std::env::var_os(key).map(|value| (OsString::from(key), value))
            })).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
            let mut child = command.spawn().map_err(|error| format!("spawn owned fixture child: {error}"))?;
            let input = child.stdin.take();
            let output = child.stdout.take().expect("requested fixture stdout pipe");
            let identity = match probe_process_identity(child.id()) {
                Ok(Some(identity)) if identity.platform_start_key != 0 => identity,
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("capture owned fixture child identity: {result:?}"));
                }
            };
            let mut owned = Self { child, input, output, identity, reaped: None };
            let ready = owned.read_line(Instant::now() + CHILD_CONTROL_BUDGET)?;
            if ready != format!("READY {}", owned.identity.pid) {
                return Err(format!("fixture child omitted matching READY: {ready}"));
            }
            if owned.identity.executable.as_ref().and_then(|path| path.canonicalize().ok()).as_deref()
                != Some(executable) {
                return Err("fixture child executable identity does not match its exact copy".into());
            }
            Ok(owned)
        }

        fn read_line(&mut self, deadline: Instant) -> Result<String, String> {
            let mut line = Vec::new();
            loop {
                let remaining = deadline.checked_duration_since(Instant::now())
                    .ok_or("fixture child response deadline expired")?;
                let mut descriptor = libc::pollfd { fd: self.output.as_raw_fd(), events: libc::POLLIN, revents: 0 };
                let millis = remaining.as_millis().max(1).min(libc::c_int::MAX as u128) as libc::c_int;
                // SAFETY: poll receives one writable descriptor for our owned pipe.
                let count = unsafe { libc::poll(&raw mut descriptor, 1, millis) };
                if count < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::Interrupted { continue; }
                    return Err(format!("fixture child response poll: {error}"));
                }
                if count == 0 { return Err("fixture child response deadline expired".into()); }
                let mut byte = [0_u8; 1];
                match self.output.read(&mut byte) {
                    Ok(0) => return Err("fixture child closed its response pipe".into()),
                    Ok(_) if byte[0] == b'\n' => return String::from_utf8(line).map_err(|error| error.to_string()),
                    Ok(_) if line.len() < 128 => line.push(byte[0]),
                    Ok(_) => return Err("fixture child response is too long".into()),
                    Err(error) => return Err(format!("fixture child response: {error}")),
                }
            }
        }

        fn command(&mut self, command: &[u8]) -> Result<(), String> {
            self.input.as_mut().ok_or("fixture child input has closed")?
                .write_all(command).map_err(|error| format!("fixture child command: {error}"))
        }

        fn ping(&mut self) -> Result<(), String> {
            if self.child.try_wait().map_err(|error| error.to_string())?.is_some() {
                return Err("fixture child exited before PING".into());
            }
            self.command(b"PING\n")?;
            let reply = self.read_line(Instant::now() + CHILD_CONTROL_BUDGET)?;
            if reply != format!("ALIVE {}", self.identity.pid) { return Err("fixture child PING identity mismatch".into()); }
            let live = probe_process_identity(self.identity.pid).map_err(|error| error.to_string())?
                .ok_or("fixture child disappeared after PING")?;
            if live.platform_start_key != self.identity.platform_start_key || live.executable != self.identity.executable {
                return Err("fixture child generation changed after PING".into());
            }
            Ok(())
        }

        fn reap_until(&mut self, deadline: Instant) -> Result<ExitStatus, String> {
            if let Some(status) = self.reaped { return Ok(status); }
            loop {
                if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
                    self.reaped = Some(status);
                    return Ok(status);
                }
                if Instant::now() >= deadline { return Err("owned fixture child did not exit by its deadline".into()); }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn exit_normally(&mut self) -> Result<ExitStatus, String> {
            self.command(b"EXIT\n")?;
            self.input.take();
            let status = self.reap_until(Instant::now() + CHILD_CONTROL_BUDGET)?;
            if !status.success() { return Err(format!("fixture child normal exit failed: {status}")); }
            Ok(status)
        }
    }

    impl Drop for OwnedFixtureChild {
        fn drop(&mut self) {
            self.input.take();
            if self.reaped.is_none() {
                // Only this still-owned direct Child handle grants cleanup
                // authority. No enumeration, signal-by-name, or PID fallback.
                let _ = self.child.kill();
                self.reaped = self.child.wait().ok();
            }
        }
    }

    struct Session<'a> {
        runtime: &'a tokio::runtime::Runtime,
        owner: Option<Arc<GuardOwner>>,
        children: Vec<OwnedFixtureChild>,
        copies: Option<TempDir>,
    }

    impl Drop for Session<'_> {
        fn drop(&mut self) {
            // On a failed assertion settle fixture children before asking the
            // guardian to prove their absence. Return is a teardown transition
            // only; it never changes the recorded scenario result to success.
            self.children.clear();
            if let Some(owner) = self.owner.take() {
                if owner.native_returned().is_err() { let _ = owner.abort_initialization(); }
                let result = self.runtime.block_on(tokio::time::timeout(
                    HELPER_CLEANUP_BUDGET + RPC_TIMEOUT + FIXTURE_COMPLETION_GRACE,
                    owner.stop_and_join(),
                ));
                if !matches!(result, Ok(Ok(_))) {
                    eprintln!("fixture teardown retained guardian authority: {result:?}");
                    // Keep exact helper paths readable while production
                    // GuardianLease retains its independently owned authority.
                    if let Some(copies) = self.copies.take() { let _ = copies.keep(); }
                }
            }
        }
    }

    fn run_fixture(runtime: &tokio::runtime::Runtime, mode: Mode, evidence_root: &Path, evidence: &mut Value) -> Result<(), String> {
        let copies = tempfile::Builder::new().prefix(".guardian-fixture-").tempdir_in(evidence_root)
            .map_err(|error| format!("create owned helper copies directory: {error}"))?;
        fs::set_permissions(copies.path(), fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure helper copies directory: {error}"))?;
        let executable = std::env::current_exe().and_then(|path| path.canonicalize())
            .map_err(|error| format!("fixture executable: {error}"))?;
        let mut helper_paths = Vec::new();
        for name in ["generic", "gpu", "renderer", "plugin", "alerts"] {
            let copy = copies.path().join(name);
            fs::copy(&executable, &copy).map_err(|error| format!("copy fixture helper {name}: {error}"))?;
            fs::set_permissions(&copy, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("set copied helper executable permissions: {error}"))?;
            let copy = copy.canonicalize().map_err(|error| error.to_string())?;
            if !fs::symlink_metadata(&copy).map_err(|error| error.to_string())?.file_type().is_file() {
                return Err("helper copy must be a real regular file".into());
            }
            helper_paths.push(copy);
        }
        evidence["helper_paths"] = json!(helper_paths);
        evidence["five_distinct_regular_executable_copies"] = json!(true);
        let main = probe_process_identity(std::process::id()).map_err(|error| error.to_string())?
            .ok_or("fixture Main identity is unavailable")?;
        evidence["main_identity"] = json!(main);
        let owner = GuardOwner::start(&helper_paths[0], main, helper_paths.clone())?;
        let mut session = Session { runtime, owner: Some(Arc::clone(&owner)), children: Vec::new(), copies: Some(copies) };
        let socket = owner.socket_path().to_owned();
        let guardian_identity = guardian_identity(&socket)?;
        evidence["guardian_identity"] = json!(guardian_identity);
        evidence["guardian_argv"] = json!(["--nomifun-cef-guardian", format!("--socket={}", socket.display())]);
        evidence["guardian_started_by_production_managed_builder"] = json!(true);
        evidence["guardian_socket"] = json!(socket);

        let nonce = owner.declare("renderer")?;
        session.children.push(OwnedFixtureChild::spawn(&helper_paths[2], &[
            OsString::from("--test-helper"), socket.as_os_str().to_owned(),
            OsString::from(nonce), OsString::from("renderer"),
        ])?);
        session.children.push(OwnedFixtureChild::spawn(&helper_paths[2], &[OsString::from("--test-sibling")])?);
        evidence["registered_identity"] = json!(session.children[0].identity);
        evidence["unrelated_sibling_identity"] = json!(session.children[1].identity);
        evidence["helper_argv"] = json!(["--test-helper", socket.to_string_lossy(), "<private per-launch nonce>", "renderer"]);
        session.children[0].ping()?;
        session.children[1].ping()?;
        evidence["both_owned_direct_children_ready"] = json!(true);

        let cleanup_reply = if mode == Mode::AbortInitialization {
            let before = owner.status()?;
            require_disarmed(&before)?;
            if before.phase != "ready" || before.receipt.is_some() {
                return Err("initialization abort fixture did not start from ready pre-native state".into());
            }
            evidence["before_abort_reply"] = json!(before);
            evidence["native_call"] = json!("initialization_abort_only");
            owner.settle_failed_initialization()?;
            evidence["production_settle_failed_initialization_completed"] = json!(true);
            // The production method has already joined its guardian. Retrieve
            // its retained terminal acknowledgement through the idempotent
            // production API, without sending another IPC operation.
            let stopped = runtime.block_on(tokio::time::timeout(
                HELPER_CLEANUP_BUDGET + RPC_TIMEOUT + FIXTURE_COMPLETION_GRACE,
                owner.stop_and_join(),
            )).map_err(|_| "initialization abort terminal receipt exceeded fixture allowance")??;
            require_disarmed(&stopped)?;
            evidence["native_shutdown_never_entered"] = json!(true);
            stopped
        } else {
        let entered_at = Instant::now();
        let entered = owner.enter_native()?;
        if !entered.native_running || entered.deadline_unix_ms.is_none() || entered.phase != "native_running" {
            return Err("production guardian did not arm its native deadline".into());
        }
        evidence["enter_reply"] = json!(entered);
        let entry_ack_at = Instant::now();
        let repeated_entry = owner.enter_native()?;
        if !repeated_entry.native_running || repeated_entry.deadline_unix_ms != entered.deadline_unix_ms {
            return Err("repeated native entry changed the original production deadline".into());
        }
        evidence["repeated_entry_kept_deadline"] = json!(true);
        let receipt_deadline = entry_ack_at + NATIVE_SHUTDOWN_BUDGET + HELPER_CLEANUP_BUDGET + FIXTURE_COMPLETION_GRACE;
        match mode {
            Mode::NormalReturn => {
                let status = session.children[0].exit_normally()?;
                evidence["registered_exit"] = exit_evidence(status);
                let returned = owner.native_returned()?;
                require_disarmed(&returned)?;
                evidence["returned_reply"] = json!(returned);
                await_receipt(&owner, Instant::now() + HELPER_CLEANUP_BUDGET + FIXTURE_COMPLETION_GRACE)?
            }
            Mode::AckLoss => {
                send_return_discarding_ack_body(&socket)?;
                evidence["native_returned_ack_header_bytes_read"] = json!(1);
                evidence["native_returned_ack_body_consumed"] = json!(false);
                evidence["native_returned_ack_decoded"] = json!(false);
                evidence["lost_ack_oracle"] = json!("response_prefix_observed_then_remaining_ack_discarded");
                let reply = await_receipt(&owner, Instant::now() + HELPER_CLEANUP_BUDGET + FIXTURE_COMPLETION_GRACE)?;
                require_disarmed(&reply)?;
                evidence["lost_ack_followup_status"] = json!(reply);
                reply
            }
            Mode::Timeout => {
                let pre_timeout_deadline = entered_at + NATIVE_SHUTDOWN_BUDGET - Duration::from_secs(5);
                while Instant::now() < pre_timeout_deadline {
                    std::thread::sleep(Duration::from_millis(100).min(pre_timeout_deadline.saturating_duration_since(Instant::now())));
                }
                session.children[0].ping()?;
                session.children[1].ping()?;
                let before = owner.status()?;
                if !before.native_running || before.receipt.is_some() || before.deadline_unix_ms.is_none() {
                    return Err("guardian cleaned up before the production native deadline".into());
                }
                evidence["before_timeout_reply"] = json!(before);
                let reply = await_receipt(&owner, receipt_deadline)?;
                if entered_at.elapsed() < NATIVE_SHUTDOWN_BUDGET || !reply.native_running {
                    return Err("timeout cleanup did not preserve unreturned native state and full 30-second budget".into());
                }
                evidence["timeout_receipt_elapsed_ms"] = json!(entered_at.elapsed().as_millis());
                reply
            }
            Mode::AbortInitialization => unreachable!("initialization abort is handled before native entry"),
        }
        };
        require_complete_receipt(&cleanup_reply)?;
        evidence["cleanup_reply"] = json!(cleanup_reply);
        let status = session.children[0].reap_until(Instant::now() + CHILD_CONTROL_BUDGET)?;
        if mode != Mode::NormalReturn && status.signal() != Some(libc::SIGKILL) {
            return Err(format!("guardian did not terminate the held registered helper with SIGKILL: {status}"));
        }
        evidence["registered_exit"] = exit_evidence(status);
        if probe_process_identity(session.children[0].identity.pid).map_err(|error| error.to_string())?.is_some() {
            return Err("owned registered helper PID remains observable after reap".into());
        }
        evidence["registered_owned_child_reaped_and_absent"] = json!(true);
        session.children[1].ping()?;
        evidence["unrelated_sibling_ping_after_cleanup"] = json!(true);

        let stopped = runtime.block_on(tokio::time::timeout(
            HELPER_CLEANUP_BUDGET + RPC_TIMEOUT + FIXTURE_COMPLETION_GRACE,
            owner.stop_and_join(),
        )).map_err(|_| "fixture Stop/join exceeded its supervisor allowance")??;
        require_complete_receipt(&stopped)?;
        evidence["stop_reply"] = json!(stopped);
        evidence["production_guard_owner_stop_and_join_completed"] = json!(true);
        if probe_process_identity(guardian_identity.pid).map_err(|error| error.to_string())?.is_some() || socket.exists() {
            return Err("guardian child or its owned socket remains after production Stop/join".into());
        }
        evidence["guardian_reaped_and_socket_removed"] = json!(true);
        session.owner.take();
        session.children[1].ping()?;
        evidence["unrelated_sibling_ping_after_guardian_join"] = json!(true);
        evidence["unrelated_sibling_exit"] = exit_evidence(session.children[1].exit_normally()?);
        Ok(())
    }

    fn guardian_identity(socket: &Path) -> Result<ExactProcessIdentity, String> {
        let peer = UnixStream::connect(socket).map_err(|error| format!("connect guardian identity probe: {error}"))?;
        let mut pid: libc::pid_t = 0;
        let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        // SAFETY: the kernel initializes one exact SDK pid_t from this peer.
        if unsafe { libc::getsockopt(peer.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID,
            (&raw mut pid).cast(), &raw mut length) } != 0 || length as usize != std::mem::size_of::<libc::pid_t>() || pid <= 1 {
            return Err(format!("guardian kernel peer PID: {}", std::io::Error::last_os_error()));
        }
        probe_process_identity(pid as u32).map_err(|error| error.to_string())?
            .ok_or_else(|| "guardian identity vanished".into())
    }

    fn send_return_discarding_ack_body(socket: &Path) -> Result<(), String> {
        let mut peer = UnixStream::connect(socket).map_err(|error| format!("lost-ack connect: {error}"))?;
        peer.set_read_timeout(Some(RPC_TIMEOUT)).map_err(|error| error.to_string())?;
        peer.set_write_timeout(Some(RPC_TIMEOUT)).map_err(|error| error.to_string())?;
        let frame = serde_json::to_vec(&Request::NativeReturned).map_err(|error| error.to_string())?;
        peer.write_all(&(frame.len() as u32).to_be_bytes()).and_then(|()| peer.write_all(&frame))
            .map_err(|error| format!("lost-ack NativeReturned frame: {error}"))?;
        // A completely written request alone does not prove server admission:
        // closing first can invalidate the kernel peer before authentication.
        // Observe exactly one byte of the four-byte response length prefix so
        // dispatch has completed, then discard the remaining acknowledgement.
        // No acknowledgement body is read or decoded. A separate authenticated
        // Status must independently prove NativeReturned actually disarmed.
        let mut prefix = [0_u8; 1];
        peer.read_exact(&mut prefix).map_err(|error| format!("lost-ack response prefix: {error}"))?;
        drop(peer);
        Ok(())
    }

    fn await_receipt(owner: &GuardOwner, deadline: Instant) -> Result<Reply, String> {
        loop {
            if Instant::now() >= deadline { return Err("production guardian cleanup receipt exceeded fixture allowance".into()); }
            let reply = owner.status()?;
            if Instant::now() >= deadline { return Err("production guardian cleanup receipt arrived after fixture allowance".into()); }
            if let Some(receipt) = &reply.receipt {
                if receipt.complete { return Ok(reply); }
                return Err(format!("production guardian returned incomplete receipt: {receipt:?}"));
            }
            std::thread::sleep(Duration::from_millis(25).min(deadline.saturating_duration_since(Instant::now())));
        }
    }

    fn require_disarmed(reply: &Reply) -> Result<(), String> {
        if reply.native_running || reply.deadline_unix_ms.is_some() {
            Err("NativeReturned did not disarm the production guardian deadline".into())
        } else { Ok(()) }
    }

    fn require_complete_receipt(reply: &Reply) -> Result<(), String> {
        if reply.receipt.as_ref().is_some_and(|receipt| receipt.complete && receipt.generation_absence_only
            && receipt.registered == 1 && receipt.absent == 1 && receipt.unresolved_declarations == 0 && receipt.errors.is_empty()) {
            Ok(())
        } else { Err(format!("guardian omitted exact complete registered generation receipt: {reply:?}")) }
    }

    fn exit_evidence(status: ExitStatus) -> Value {
        json!({"success": status.success(), "code": status.code(), "signal": status.signal()})
    }
}
