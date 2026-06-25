//! Opinionated wrapper around [`tokio::process::Command`] that centralises
//! cross-cutting concerns of child-process spawning across the workspace.
//!
//! Two construction flavours are provided:
//!
//! * [`Builder::new`] — for long-running agent CLIs whose stdio is owned
//!   by the caller (e.g. ACP SDK). Defaults to inherited stdio. Callers
//!   typically override to `piped()` to capture the streams.
//!
//! * [`Builder::clean_cli`] — for short-lived CLI tools whose output we
//!   capture and parse. Defaults to piped stdio plus `NO_COLOR=1` and
//!   `TERM=dumb` so ANSI escape codes do not leak into the captured
//!   output.
//!
//! Both flavours:
//! * set `kill_on_drop(true)` so a panicking / erroring caller cannot
//!   leave orphaned children;
//! * remove `NODE_OPTIONS`, `NODE_INSPECT`, `NODE_DEBUG`, `CLAUDECODE`
//!   so the child doesn't inherit debug/agent state that belongs to the
//!   parent (matches v1 `acpConnectors.ts::getCleanAgentEnv`).
//!
//! Enhanced `PATH` (including the bundled bun directory) is handled
//! once at process startup by [`crate::enhance_process_path`]; Builder
//! does not re-inject it.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Stdio;

use tokio::process::{Child, Command};

use crate::resolver::resolve_command_path;

#[cfg(unix)]
use std::os::fd::{AsRawFd, OwnedFd, RawFd};

/// Construction mode — determines default stdio + env extras.
#[derive(Debug, Clone, Copy)]
enum Mode {
    Default,
    CleanCli,
}

pub struct Builder {
    inner: Command,
    mode: Mode,
    /// Hand-off children (terminal windows, editors opened for the user) are
    /// expected to OUTLIVE this process: skip the Windows cleanup job and the
    /// Linux parent-death signal. See [`Builder::hand_off`].
    hand_off: bool,
    /// (unix) Extra fds to hand the child at specific target fd numbers, e.g.
    /// Chrome's `--remote-debugging-pipe` fd3/fd4. Each `(target, source)` is
    /// installed in a clobber-safe `pre_exec` shuffle at [`Builder::spawn`]; the
    /// source `OwnedFd`s are kept alive here until spawn forks the child.
    #[cfg(unix)]
    extra_fds: Vec<(RawFd, OwnedFd)>,
}

/// Force-kill a spawned child and wait for the direct child handle to exit.
///
/// On Unix, children spawned through [`Builder::new`] are process-group
/// leaders, so this targets that group to clean up descendants as well. On
/// Windows, this uses `taskkill /T` to terminate the process tree.
pub async fn kill_process_tree(child: &mut Child) -> io::Result<()> {
    let Some(pid) = child.id() else {
        return child.kill().await;
    };

    #[cfg(unix)]
    force_kill_process_tree(pid, Some(pid))?;
    #[cfg(windows)]
    kill_windows_process_tree(pid).await?;
    #[cfg(not(any(unix, windows)))]
    child.kill().await?;
    child.wait().await.map(|_| ())
}

impl std::fmt::Debug for Builder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builder")
            .field("mode", &self.mode)
            .field("command", self.inner.as_std())
            .finish()
    }
}

/// Renders the configured spawn as a shell-style preview (`cd … && env -u
/// X K=V <prog> <args>…`) suitable for logs and error messages. Format
/// comes for free from `std::process::Command`'s `Debug` impl.
impl std::fmt::Display for Builder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.inner.as_std(), f)
    }
}

impl Builder {
    /// Builder for long-running agent subprocesses (ACP SDK, legacy CLI).
    ///
    /// Defaults:
    /// - stdio: inherit (callers typically override with `.stdin(piped())`
    ///   etc. when they need to own the streams)
    /// - `kill_on_drop(true)`
    /// - removes `NODE_OPTIONS`, `NODE_INSPECT`, `NODE_DEBUG`, `CLAUDECODE`
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        let mut inner = Command::new(resolve_program(program.as_ref()));
        inner.kill_on_drop(true);
        configure_platform_spawn(&mut inner);
        strip_pollution(&mut inner);
        Self {
            inner,
            mode: Mode::Default,
            hand_off: false,
            #[cfg(unix)]
            extra_fds: Vec::new(),
        }
    }

    /// Builder for short-lived CLI tools whose output we capture.
    ///
    /// Defaults:
    /// - stdio: all piped
    /// - `kill_on_drop(true)`
    /// - removes `NODE_OPTIONS`, `NODE_INSPECT`, `NODE_DEBUG`, `CLAUDECODE`
    /// - sets `NO_COLOR=1`, `TERM=dumb`
    pub fn clean_cli<S: AsRef<OsStr>>(program: S) -> Self {
        let mut inner = Command::new(resolve_program(program.as_ref()));
        inner
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .env("TERM", "dumb");
        configure_platform_spawn(&mut inner);
        strip_pollution(&mut inner);
        Self {
            inner,
            mode: Mode::CleanCli,
            hand_off: false,
            #[cfg(unix)]
            extra_fds: Vec::new(),
        }
    }

    /// Mark this child as a hand-off: a process launched FOR the user that
    /// must outlive us (an opened terminal window, an editor, an installer).
    /// It is excluded from the force-kill safety nets — the Windows cleanup
    /// job and the Linux PDEATHSIG — which would otherwise terminate it (and
    /// everything it spawned) the moment this process exits.
    pub fn hand_off(&mut self) -> &mut Self {
        self.hand_off = true;
        self
    }

    /// (unix) Hand owned fds to the child at specific target fd numbers — e.g.
    /// Chrome's `--remote-debugging-pipe` reads commands on fd 3 and writes
    /// responses on fd 4. Each `(target_fd, source)` is installed via a
    /// clobber-safe `pre_exec` shuffle at [`spawn`](Self::spawn): every source is
    /// first relocated to a high temp fd (so none sits on a target slot), then
    /// `dup2`'d onto its target (which clears `FD_CLOEXEC` on the target so it
    /// survives `exec`). The source `OwnedFd`s are kept alive in the Builder
    /// until `spawn` forks; the parent's copies are dropped when `spawn` returns.
    ///
    /// The caller should keep its own (parent-side) ends with `FD_CLOEXEC` set so
    /// they don't leak into this child or any other spawn.
    #[cfg(unix)]
    pub fn inherit_fds(&mut self, mappings: Vec<(RawFd, OwnedFd)>) -> &mut Self {
        self.extra_fds.extend(mappings);
        self
    }

    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.inner.arg(arg);
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.args(args);
        self
    }

    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.env(key, val);
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.envs(vars);
        self
    }

    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Self {
        self.inner.env_remove(key);
        self
    }

    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
        self.inner.current_dir(dir);
        self
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.inner.stdin(cfg);
        self
    }

    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.inner.stdout(cfg);
        self
    }

    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.inner.stderr(cfg);
        self
    }

    /// Spawn the process and return the standard `tokio::process::Child`.
    ///
    /// Unless [`hand_off`](Self::hand_off) was set, the child is covered by
    /// the force-kill safety nets: on Windows it is assigned to the
    /// process-global cleanup job ([`crate::job`]) — descendants inherit
    /// membership and the kernel kills the whole tree when this process dies,
    /// even force-killed (`tauri dev` rebuild, Ctrl+C), where `kill_on_drop`
    /// never runs. (Descendants the child creates in the brief window before
    /// the assignment land outside the job — see `crate::job` docs.) On Linux
    /// the equivalent is PDEATHSIG, installed here. On macOS — which has
    /// neither — the equivalent is a kqueue `NOTE_EXIT` watcher on the parent
    /// pid that group-kills the child's pgid on parent death (see
    /// [`install_macos_pdeath_watch`]).
    pub fn spawn(mut self) -> io::Result<Child> {
        #[cfg(target_os = "linux")]
        if !self.hand_off {
            install_pdeathsig(&mut self.inner);
        }
        // (unix) Install the inherited-fd shuffle (e.g. Chrome's --remote-debugging-pipe
        // fd3/fd4) before forking. The source OwnedFds stay alive in `self` (dropped when
        // this fn returns, i.e. after the fork), so they're valid in the child.
        #[cfg(unix)]
        if !self.extra_fds.is_empty() {
            install_fd_shuffle(&mut self.inner, &self.extra_fds);
        }
        let child = self.inner.spawn()?;
        #[cfg(windows)]
        if !self.hand_off {
            crate::job::assign_to_cleanup_job(&child);
        }
        // macOS has no PDEATHSIG and no Job Object; the equivalent safety net
        // is a kqueue watcher on the parent pid that group-kills the child on
        // parent death. Installed only when we have the child's pid (a child
        // that already exited needs nothing cleaned up).
        #[cfg(target_os = "macos")]
        if !self.hand_off {
            if let Some(pid) = child.id() {
                install_macos_pdeath_watch(pid);
            }
        }
        Ok(child)
    }

    /// Run to completion and collect stdout/stderr.
    ///
    /// Equivalent to `tokio::process::Command::output` (stdout/stderr forced
    /// to piped), but routed through [`Self::spawn`] so the Windows cleanup
    /// job covers these children too.
    pub async fn output(mut self) -> io::Result<std::process::Output> {
        self.inner.stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = self.spawn()?;
        child.wait_with_output().await
    }
}

fn strip_pollution(cmd: &mut Command) {
    cmd.env_remove("NODE_OPTIONS")
        .env_remove("NODE_INSPECT")
        .env_remove("NODE_DEBUG")
        .env_remove("CLAUDECODE");
}

#[cfg(unix)]
fn configure_platform_spawn(cmd: &mut Command) {
    // Start each child in its own process group so explicit teardown can
    // kill the whole subtree (CLI + MCP descendants) in one shot.
    cmd.process_group(0);
}

/// (unix) Install a clobber-safe `pre_exec` shuffle that places each `(target, source)`
/// fd at `target` in the child (e.g. Chrome `--remote-debugging-pipe` fd3/fd4).
///
/// Algorithm (async-signal-safe): relocate every source to a high temp fd first (so no
/// source sits on a target slot), then `dup2` each temp onto its target — `dup2` clears
/// `FD_CLOEXEC` on the target, so the target survives `exec` even when the caller created
/// the source with `FD_CLOEXEC`. Temps are closed afterward. Reading the captured `maps`
/// `Vec` post-fork is safe (it was allocated pre-fork; no allocation happens in the child).
#[cfg(unix)]
fn install_fd_shuffle(cmd: &mut Command, extra_fds: &[(RawFd, OwnedFd)]) {
    // Capture (target, source_raw) by value. The OwnedFds stay alive in the Builder
    // until spawn forks, so source_raw is a valid fd in the forked child.
    let maps: Vec<(RawFd, RawFd)> = extra_fds.iter().map(|(t, fd)| (*t, fd.as_raw_fd())).collect();
    // SAFETY: the closure runs post-fork/pre-exec in the child; fcntl/dup2/close are all
    // async-signal-safe. It only reads the pre-fork-allocated `maps` and uses stack locals.
    unsafe {
        cmd.pre_exec(move || {
            const MAX_FDS: usize = 16;
            if maps.len() > MAX_FDS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "too many inherited fds",
                ));
            }
            // Phase 1: relocate sources to high temp fds (>= 20), away from target slots.
            let mut temps = [(0 as RawFd, 0 as RawFd); MAX_FDS]; // (target, temp)
            let mut base: RawFd = 20;
            for (i, &(target, source)) in maps.iter().enumerate() {
                let temp = libc::fcntl(source, libc::F_DUPFD, base);
                if temp < 0 {
                    return Err(io::Error::last_os_error());
                }
                base = temp + 1;
                temps[i] = (target, temp);
            }
            // Phase 2: dup2 temp -> target (clears CLOEXEC on target → survives exec), close temp.
            for &(target, temp) in temps.iter().take(maps.len()) {
                if libc::dup2(temp, target) < 0 {
                    return Err(io::Error::last_os_error());
                }
                libc::close(temp);
            }
            Ok(())
        });
    }
}

/// Linux: have the KERNEL deliver SIGKILL to the child when this process
/// dies without running any cleanup — force-killed by a `tauri dev` rebuild,
/// OOM-killed, crashed. kill_on_drop and the explicit group-kill only work
/// while our code still runs; this is the no-userland-cleanup safety net
/// (the Windows counterpart is the Job Object in `crate::job`). The child's
/// own MCP descendants then exit on stdin EOF.
///
/// PDEATHSIG fires when the spawning THREAD dies, not the process. Every
/// Builder spawn happens on a long-lived tokio multi-thread runtime worker
/// (audited 2026-06: no spawn_blocking / short-lived-thread call sites),
/// where thread death == runtime shutdown == exactly when children must die.
/// Do NOT call Builder::spawn from inside `spawn_blocking` or other
/// short-lived threads — the child would be killed when that thread is
/// reclaimed.
#[cfg(target_os = "linux")]
fn install_pdeathsig(cmd: &mut Command) {
    let parent_pid = std::process::id();
    // SAFETY: the pre_exec closure runs post-fork/pre-exec in the child;
    // prctl, getppid and raise are all async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have died between fork and prctl — the death
            // signal only fires for deaths AFTER it is installed, so
            // re-check and self-terminate to close the race.
            if libc::getppid() != parent_pid as libc::pid_t {
                libc::raise(libc::SIGKILL);
            }
            Ok(())
        });
    }
}

/// macOS parent-death safety net — the analogue of Linux PDEATHSIG and the
/// Windows Job Object, neither of which exists here.
///
/// macOS has no `prctl(PR_SET_PDEATHSIG)` and no kill-on-close Job Object. The
/// only thing that survives our process being force-killed (`tauri dev` rebuild,
/// crash, OOM — where no userland cleanup of ours runs) is a **separate
/// process**. A thread cannot help: it dies with the process on SIGKILL and
/// never gets to act (which is exactly why the previous thread-based version was
/// a silent no-op on force-kill, verified 2026-06-19). So we fork a tiny
/// **watchdog process** that `kqueue`-watches `EVFILT_PROC|NOTE_EXIT` on BOTH the
/// parent pid and the child pid:
/// - **parent exits first** (any cause incl. SIGKILL) → `kill(-child_pgid,
///   SIGKILL)` reaps the child plus its whole group (the child leads its own
///   group via `process_group(0)`, so `pgid == child pid`), mirroring the
///   Linux/Windows nets;
/// - **child exits first** (normal) → the watchdog just exits (nothing to kill),
///   so it never lingers.
///
/// Implementation notes (self-contained — no host re-exec / dispatch wiring):
/// - **double-fork + `setsid`**: the intermediate exits immediately (reaped by
///   us here), reparenting the watchdog to launchd, which reaps it on exit → no
///   zombie;
/// - the watchdog body ([`macos_pdeath_watchdog`]) uses **only async-signal-safe
///   libc** (no allocation, no tracing, no Rust runtime) — mandatory after
///   `fork()` in our multi-threaded tokio process;
/// - it **closes every inherited fd ≥ 3** first, so it holds no copy of the
///   parent's pipes/sockets — notably Chrome's `--remote-debugging-pipe` command
///   pipe, where a stray copy would stop Chrome from seeing EOF and self-exiting;
/// - **races closed**: re-check the parent is alive after registration (the
///   fork→register window), and re-check the child is alive (`kill(child,0)`)
///   right before `kill(-pgid)` so a child that already exited (pgid possibly
///   reused) is never mis-killed.
///
/// Best-effort: any failure degrades to `kill_on_drop` + explicit
/// [`kill_process_tree`] (which still cover graceful paths); never fatal to spawn.
#[cfg(target_os = "macos")]
fn install_macos_pdeath_watch(child_pid: u32) {
    let parent_pid = std::process::id() as libc::pid_t;
    let child_pid = child_pid as libc::pid_t;

    // SAFETY: between fork() and _exit the (grand)child calls only async-signal-safe
    // libc (fork/setsid/kqueue/kevent/kill/close/_exit) — it touches no Rust runtime
    // state, allocator, locks, or tracing. The parent path only forks and best-effort
    // reaps the immediately-exiting intermediate.
    unsafe {
        let pid1 = libc::fork();
        if pid1 < 0 {
            tracing::warn!(
                error = %io::Error::last_os_error(),
                "macos pdeath watch: fork() failed; child tree relies on kill_on_drop only"
            );
            return;
        }
        if pid1 == 0 {
            // Intermediate: own session, then fork the watchdog and exit so the
            // watchdog reparents to launchd (which reaps it on exit → no zombie).
            libc::setsid();
            let pid2 = libc::fork();
            if pid2 != 0 {
                // intermediate (pid2 > 0) or fork failure (pid2 < 0): exit now.
                libc::_exit(0);
            }
            // Grandchild = watchdog. Diverges (ends in _exit).
            macos_pdeath_watchdog(parent_pid, child_pid);
        }
        // Original parent: reap the intermediate (it exits immediately). ECHILD if
        // tokio's SIGCHLD reaper got it first — harmless.
        let mut status: libc::c_int = 0;
        libc::waitpid(pid1, &mut status, 0);
    }
}

/// The watchdog loop, run in the double-forked grandchild. **async-signal-safe
/// only** — raw libc, no allocation / tracing / Rust runtime. Always ends in `_exit`.
///
/// # Safety
/// Must run post-fork in a dedicated process that does nothing else. Every call
/// here is async-signal-safe libc.
#[cfg(target_os = "macos")]
#[allow(unsafe_op_in_unsafe_fn)] // whole body is async-signal-safe libc FFI (see # Safety)
unsafe fn macos_pdeath_watchdog(parent_pid: libc::pid_t, child_pid: libc::pid_t) -> ! {
    // 1) Drop every inherited fd ≥ 3 so we hold no copy of the parent's
    //    pipes/sockets (notably Chrome's command pipe — a stray copy would block
    //    Chrome's EOF-driven self-exit). Close BEFORE creating our kqueue.
    let maxfd = {
        let m = libc::sysconf(libc::_SC_OPEN_MAX);
        if m <= 3 || m > 4096 { 4096 } else { m as libc::c_int }
    };
    let mut fd = 3;
    while fd < maxfd {
        libc::close(fd);
        fd += 1;
    }

    let kq = libc::kqueue();
    if kq < 0 {
        libc::_exit(11);
    }

    // 2) Register NOTE_EXIT on parent then child (separate calls so each one's
    //    ESRCH — already dead — is detected on its own).
    if !register_note_exit(kq, parent_pid) {
        // Parent already gone (or registration failed): kill the child group if
        // it's still alive, then exit.
        if libc::kill(child_pid, 0) == 0 {
            libc::kill(-child_pid, libc::SIGKILL);
        }
        libc::_exit(0);
    }
    if !register_note_exit(kq, child_pid) {
        // Child already gone: nothing to clean up.
        libc::_exit(0);
    }

    // 3) Race re-check: the parent could have died between our fork and the
    //    registration above (NOTE_EXIT only fires for deaths AFTER EV_ADD).
    if libc::kill(parent_pid, 0) != 0 && *libc::__error() == libc::ESRCH {
        if libc::kill(child_pid, 0) == 0 {
            libc::kill(-child_pid, libc::SIGKILL);
        }
        libc::_exit(0);
    }

    // 4) Block until the kernel reports the parent OR the child exited.
    let mut ev = zeroed_kevent();
    loop {
        let n = libc::kevent(kq, std::ptr::null(), 0, &mut ev, 1, std::ptr::null());
        if n < 0 {
            if *libc::__error() == libc::EINTR {
                continue; // interrupted → retry
            }
            libc::_exit(13); // unexpected → degrade to kill_on_drop
        }
        if n >= 1 {
            break;
        }
    }

    // `ev.ident` read by value (kevent is repr(packed) on Apple; fields are Copy).
    if ev.ident == parent_pid as libc::uintptr_t {
        // Parent died → group-kill the child, but only if it's still alive (close
        // the pgid-reuse window: a dead child's pgid could belong to another group).
        if libc::kill(child_pid, 0) == 0 {
            libc::kill(-child_pid, libc::SIGKILL);
        }
    }
    // else: child exited first → nothing to kill.
    libc::_exit(0);
}

/// Register `EVFILT_PROC|NOTE_EXIT` on `pid`. Returns `false` if the kernel
/// rejected it (e.g. the pid is already dead → ESRCH).
///
/// # Safety
/// Async-signal-safe libc only; called from the watchdog grandchild.
#[cfg(target_os = "macos")]
#[allow(unsafe_op_in_unsafe_fn)] // whole body is libc FFI (see # Safety)
unsafe fn register_note_exit(kq: libc::c_int, pid: libc::pid_t) -> bool {
    let change = libc::kevent {
        ident: pid as libc::uintptr_t,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_RECEIPT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    let mut receipt = zeroed_kevent();
    let n = libc::kevent(kq, &change, 1, &mut receipt, 1, std::ptr::null());
    if n < 0 {
        return false;
    }
    // EV_RECEIPT places an EV_ERROR receipt with data==errno (0 == success).
    // `receipt.flags`/`receipt.data` read by value (packed struct; Copy fields).
    if n >= 1 && (receipt.flags & libc::EV_ERROR) != 0 && receipt.data != 0 {
        return false; // e.g. ESRCH: pid already dead.
    }
    true
}

/// A zero-initialised `kevent` (stack-local; no allocation). async-signal-safe.
#[cfg(target_os = "macos")]
fn zeroed_kevent() -> libc::kevent {
    libc::kevent {
        ident: 0,
        filter: 0,
        flags: 0,
        fflags: 0,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

#[cfg(windows)]
fn configure_platform_spawn(cmd: &mut Command) {
    // GUI host: keep console-subsystem children (bun/node/git/taskkill/…) from
    // flashing a console window. CREATE_NO_WINDOW = 0x0800_0000.
    cmd.creation_flags(0x0800_0000);
}

#[cfg(not(any(unix, windows)))]
fn configure_platform_spawn(_cmd: &mut Command) {}

#[cfg(unix)]
fn force_kill_process_tree(pid: u32, process_group_id: Option<u32>) -> io::Result<()> {
    if let Some(group_id) = process_group_id.filter(|group_id| *group_id > 1) {
        let result = unsafe { libc::kill(-(group_id as i32), libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }

        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return kill_unix_target(pid as i32);
        }
        return Err(err);
    }

    kill_unix_target(pid as i32)
}

#[cfg(unix)]
fn kill_unix_target(target: i32) -> io::Result<()> {
    let result = unsafe { libc::kill(target, libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }

    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(err)
    }
}

#[cfg(windows)]
async fn kill_windows_process_tree(pid: u32) -> io::Result<()> {
    let pid_arg = pid.to_string();
    let mut cmd = Builder::clean_cli("taskkill");
    cmd.args(["/F", "/T", "/PID", pid_arg.as_str()]);
    let output = cmd.output().await?;
    if output.status.success() || output.status.code() == Some(128) {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::Other,
        format!(
            "taskkill failed for pid {pid} (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ),
    ))
}

/// Resolve `program` through `resolve_command_path` so callers don't have
/// to. If the input already contains a path separator (relative or
/// absolute) we leave it alone — only bare command names go through
/// the resolver, where the bundled-bun shim and Windows `.cmd / .ps1 /
/// .bat` fallbacks live.
fn resolve_program(program: &OsStr) -> OsString {
    if let Some(s) = program.to_str()
        && !s.is_empty()
        && !s.contains('/')
        && !s.contains('\\')
        && let Some(path) = resolve_command_path(s)
    {
        return path.into_os_string();
    }
    program.to_os_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the unix-only tests below need these.
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn clean_cli_captures_stdout_and_strips_env_pollution() {
        // Set pollution on parent — it must not leak into child.
        // SAFETY: single-threaded test. Rust 2024 requires unsafe.
        unsafe {
            std::env::set_var("NODE_OPTIONS", "--inspect=9229");
            std::env::set_var("CLAUDECODE", "1");
        }

        // Ask the child to print NODE_OPTIONS + CLAUDECODE; Builder must
        // have removed them.
        let mut b = Builder::clean_cli("sh");
        b.arg("-c")
            .arg("echo \"NO:${NODE_OPTIONS:-unset} CC:${CLAUDECODE:-unset}\"");
        let output = b.output().await.unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("NO:unset"), "got: {stdout}");
        assert!(stdout.contains("CC:unset"), "got: {stdout}");
        assert!(output.status.success());

        // SAFETY: single-threaded test cleanup.
        unsafe {
            std::env::remove_var("NODE_OPTIONS");
            std::env::remove_var("CLAUDECODE");
        }
    }

    #[tokio::test]
    async fn clean_cli_sets_no_color_and_term_dumb() {
        let mut b = Builder::clean_cli("sh");
        b.arg("-c").arg("echo \"NC:${NO_COLOR:-unset} TERM:${TERM:-unset}\"");
        let output = b.output().await.unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("NC:1"), "got: {stdout}");
        assert!(stdout.contains("TERM:dumb"), "got: {stdout}");
    }

    #[tokio::test]
    async fn agent_allows_stdio_override() {
        // agent() defaults to inherit. Override to piped, then verify
        // we can capture output.
        let mut b = Builder::new("sh");
        b.arg("-c").arg("echo hello").stdout(Stdio::piped());
        let output = b.output().await.unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn agent_strips_env_pollution() {
        // SAFETY: single-threaded test.
        unsafe {
            std::env::set_var("NODE_INSPECT", "9229");
            std::env::set_var("NODE_DEBUG", "*");
        }

        let mut b = Builder::new("sh");
        b.arg("-c")
            .arg("echo \"NI:${NODE_INSPECT:-unset} ND:${NODE_DEBUG:-unset}\"")
            .stdout(Stdio::piped());
        let output = b.output().await.unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("NI:unset"), "got: {stdout}");
        assert!(stdout.contains("ND:unset"), "got: {stdout}");

        // SAFETY: single-threaded cleanup.
        unsafe {
            std::env::remove_var("NODE_INSPECT");
            std::env::remove_var("NODE_DEBUG");
        }
    }

    #[tokio::test]
    async fn spawn_returns_child_with_pid() {
        let mut b = Builder::new("sh");
        b.arg("-c").arg("sleep 0.05");
        let mut child = b.spawn().unwrap();
        assert!(child.id().is_some());
        let status = child.wait().await.unwrap();
        assert!(status.success());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn spawned_child_is_assigned_to_cleanup_job() {
        use std::os::windows::io::RawHandle;

        use windows_sys::Win32::System::JobObjects::IsProcessInJob;

        let mut b = Builder::new("powershell");
        b.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
        let mut child = b.spawn().unwrap();

        let job = crate::job::global_cleanup_job().expect("global cleanup job");
        let child_handle: RawHandle = child.raw_handle().expect("child handle");
        let mut in_job = 0;
        // SAFETY: both handles are live; IsProcessInJob only reads them.
        let ok = unsafe { IsProcessInJob(child_handle.cast(), job.raw(), &mut in_job) };
        assert_ne!(ok, 0, "IsProcessInJob should succeed");
        assert_ne!(in_job, 0, "Builder-spawned child must be inside the cleanup job");

        kill_process_tree(&mut child).await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn hand_off_child_stays_out_of_cleanup_job() {
        use std::os::windows::io::RawHandle;

        use windows_sys::Win32::System::JobObjects::IsProcessInJob;

        let mut b = Builder::new("powershell");
        b.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]).hand_off();
        let mut child = b.spawn().unwrap();

        let job = crate::job::global_cleanup_job().expect("global cleanup job");
        let child_handle: RawHandle = child.raw_handle().expect("child handle");
        let mut in_job = 0;
        // SAFETY: both handles are live; IsProcessInJob only reads them.
        let ok = unsafe { IsProcessInJob(child_handle.cast(), job.raw(), &mut in_job) };
        assert_ne!(ok, 0, "IsProcessInJob should succeed");
        assert_eq!(in_job, 0, "hand-off child must NOT be in the cleanup job");

        kill_process_tree(&mut child).await.unwrap();
    }

    #[test]
    fn display_renders_shell_style_command() {
        let mut b = Builder::new("/usr/local/bin/bun");
        b.current_dir("/tmp/work dir")
            .env("FOO", "bar baz")
            .args(["x", "--flag", "with space"]);

        let preview = format!("{b}");

        // The shell-style `cd "..." && env -u X K=V "prog" "args"...` rendering is
        // produced ONLY by std::process::Command's Debug impl on UNIX. On Windows,
        // std renders just the quoted program + quoted args (no cd / env -u / K=V).
        #[cfg(unix)]
        {
            assert!(
                preview.starts_with(r#"cd "/tmp/work dir" &&"#),
                "missing cwd prefix: {preview}"
            );
            assert!(preview.contains("env "), "expected env section: {preview}");
            assert!(preview.contains(r#"FOO="bar baz""#), "FOO missing: {preview}");
            // strip_pollution unsets these
            assert!(
                preview.contains("-u NODE_OPTIONS"),
                "missing -u NODE_OPTIONS: {preview}"
            );
            assert!(preview.contains("-u CLAUDECODE"), "missing -u CLAUDECODE: {preview}");
        }

        // On both platforms the program and each quoted arg appear in the preview.
        // (On Windows the program is rendered without the surrounding-context
        // prefix, so assert containment of the program rather than a quoted form.)
        #[cfg(unix)]
        assert!(
            preview.contains(r#""/usr/local/bin/bun""#),
            "program missing: {preview}"
        );
        #[cfg(windows)]
        assert!(preview.contains("bun"), "program missing: {preview}");

        assert!(preview.contains(r#""--flag""#), "arg --flag missing: {preview}");
        assert!(preview.contains(r#""with space""#), "arg with space missing: {preview}");
    }

    #[cfg(unix)]
    fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !is_pid_alive(pid) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(unix)]
    fn is_pid_alive(pid: u32) -> bool {
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result == 0 {
            return true;
        }
        !matches!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH))
    }

    /// macOS parent-death safety net: a child spawned through `Builder`
    /// installs a kqueue `NOTE_EXIT` watcher on the parent (this process).
    /// We cannot kill the live test process to observe it, so this test
    /// exercises the same primitive the watcher fires — group-kill of the
    /// child's pgid (== leader pid, since `process_group(0)` makes the child
    /// its own group leader) — and asserts the pgid is reaped. This guards
    /// the contract the macOS watcher relies on: `kill(-pgid, SIGKILL)`
    /// tears the spawned subtree down, and `kill(pid, 0)` reports `ESRCH`.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_group_kill_reaps_spawned_child_pgid() {
        use std::os::unix::process::ExitStatusExt;

        let mut b = Builder::new("sh");
        b.arg("-c")
            .arg("sleep 30")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = b.spawn().unwrap();
        let pid = child.id().expect("spawned child should have a pid");

        assert!(is_pid_alive(pid), "child pid={pid} should be running after spawn");

        // The child is its own process-group leader (process_group(0)), so its
        // pgid equals its pid. This is exactly the target the kqueue watcher
        // signals when the parent dies.
        force_kill_process_tree(pid, Some(pid)).expect("group kill of child pgid should succeed");

        // A SIGKILL'd DIRECT child lingers as a zombie (kill(pid,0) keeps
        // returning 0) until we wait() on it — a raw liveness poll would spin
        // until timeout. Reaping returns the terminal status, which is the
        // stronger assertion: the child was terminated by SIGKILL (delivered via
        // the negative-pgid group kill), not a normal exit.
        let status = child.wait().await.expect("wait on group-killed child");
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "child pid={pid} must be terminated by SIGKILL via group kill, got {status:?}",
        );
        // After reaping, the pid is truly gone (kill(pid,0) → ESRCH).
        assert!(!is_pid_alive(pid), "child pid={pid} should be gone after group kill + reap");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_process_tree_uses_cached_group_when_leader_has_exited() {
        let marker = tempfile::NamedTempFile::new().unwrap();
        let marker_path = marker.path().to_string_lossy().into_owned();

        let mut builder = Builder::new("sh");
        builder
            .args([
                "-c",
                "sleep 60 & child=$!; printf '%s' \"$child\" > \"$1\"; exit 0",
                "runtime-cached-group-cleanup",
                marker_path.as_str(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = builder.spawn().unwrap();
        let leader_pid = child.id().expect("leader pid should exist");
        let status = child.wait().await.unwrap();
        assert!(status.success(), "leader should exit before cleanup test");

        let child_pid: u32 = std::fs::read_to_string(marker.path())
            .expect("background child pid marker should exist")
            .trim()
            .parse()
            .expect("background child pid should be numeric");

        assert!(
            is_pid_alive(child_pid),
            "background child pid={child_pid} should still be alive"
        );

        force_kill_process_tree(leader_pid, Some(leader_pid)).expect("cached group kill should succeed");

        assert!(
            wait_for_pid_exit(child_pid, Duration::from_secs(5)),
            "background child pid={child_pid} should exit after cached group kill",
        );
    }
}
